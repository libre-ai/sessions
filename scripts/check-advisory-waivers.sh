#!/usr/bin/env bash
set -euo pipefail

# Fails when an advisory waiver outlives its justification.
#
# Two files waive RustSec advisories over the same locked graph:
# `.cargo/audit.toml` (cargo-audit) and `deny.toml` (cargo-deny). Nothing used
# to connect a waiver to the graph it was written for, so a waiver could keep
# suppressing an advisory long after the crate it named had left the tree. That
# is not a cosmetic drift: a waiver that survives its reason silences the
# *future*. If the crate returns through another path, the gate stays quiet
# precisely when it should speak.
#
# Three failure modes, all of them a hard fail:
#
#   1. undated       — the entry carries no expiry, so nobody ever revisits it.
#   2. expired       — the expiry is behind the anchor date (see below).
#   3. crate absent  — the crate the entry names is nowhere in `Cargo.lock`,
#                      so the entry suppresses nothing and would mask a return.
#
# Mode 3 is the one that catches the defect this script was written for.
#
# ## The anchor date, and why it is not `now()`
#
# Comparing an expiry against the wall clock would let a required check go red
# on a morning when no file changed — a gate nobody can fix by fixing the tree,
# on a pull request that introduced nothing. Expiry is therefore measured
# against the committer date of `HEAD`: the tree under test dates itself. The
# verdict is a pure function of the commit, so re-running an old commit yields
# what it yielded then, and `main` never reddens spontaneously. What the check
# does block is *new work riding on a lapsed waiver* — any commit authored past
# an expiry fails, which is the behaviour that was actually wanted.
# `WAIVER_ANCHOR_DATE` overrides the anchor, which is how the negative cases
# below are exercised.
#
# ## Why this runs without a Rust toolchain
#
# It reasons only over tracked files — the two configs and `Cargo.lock` — so it
# is offline, needs no cargo, no advisory database and no network, and belongs
# in the tracked-tree hygiene job rather than in a workflow of its own. Only two
# checks are required on `main`; a check delivered in a new workflow would not
# be one of them, and a gate that cannot block is a decoration.
#
# ## Metadata format
#
# cargo-audit's ignore list is a plain array of strings, and cargo-deny 0.19
# accepts only `id` and `reason` in a structured entry — it rejects an
# `expiration` key outright. Neither tool can carry an expiry natively, so it
# travels in a trailing comment, identical in both files, one per entry:
#
#     "RUSTSEC-YYYY-NNNN", # waiver: crate=<name> expires=<YYYY-MM-DD>
#
# Dates are compared as strings: `YYYY-MM-DD` sorts chronologically, which
# sidesteps the GNU/BSD `date` split entirely.

root="${ROOT:-$(git rev-parse --show-toplevel)}"
cd "$root"

configs=(".cargo/audit.toml" "deny.toml")
lock="Cargo.lock"

# Verdict 3 of 3: the check could not look. Distinct from "clean" on purpose —
# a check that cannot search must never be mistaken for a check that searched
# and found nothing.
blocked() {
  echo "waiver check BLOCKED: $*" >&2
  echo "  (unable to search — this is not a pass)" >&2
  exit 2
}

for f in "${configs[@]}" "$lock"; do
  [ -f "$f" ] || blocked "expected tracked file '$f' is missing"
done

anchor="${WAIVER_ANCHOR_DATE:-}"
if [ -z "$anchor" ]; then
  anchor="$(git log -1 --format=%cs 2>/dev/null || true)"
fi
if ! printf '%s' "$anchor" | grep -qE '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'; then
  blocked "no usable anchor date (got '${anchor}'); set WAIVER_ANCHOR_DATE=YYYY-MM-DD"
fi
echo "anchor date: $anchor"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Every crate name in the lock, one per line. Cargo.lock is the union of normal
# and dev dependencies; treating that union as "present" is deliberate. Absence
# is only reported when a crate appears *nowhere* in the lock, which is an
# unambiguous signal and the conservative side of the doubt.
grep -E '^name = "' "$lock" | sed -E 's/^name = "//; s/"$//' | sort -u > "$tmp/lockcrates"

lock_count="$(wc -l < "$tmp/lockcrates" | tr -d ' ')"
if [ "$lock_count" -eq 0 ]; then
  blocked "parsed zero crate names from '$lock' — the parser is broken, not the lock"
fi
echo "lock crates indexed: $lock_count"

examined=0
findings=0

for f in "${configs[@]}"; do
  # An ignore entry carries the id in quotes on a line that is not itself a
  # comment. A bare mention inside prose (a line whose first non-blank byte is
  # '#') is documentation, not a waiver, and is skipped.
  while IFS= read -r line; do
    [ -n "$line" ] || continue

    id="$(printf '%s' "$line" | sed -E 's/.*"(RUSTSEC-[0-9]{4}-[0-9]{4})".*/\1/')"
    examined=$((examined + 1))

    if ! printf '%s' "$line" \
      | grep -qE 'waiver:[[:space:]]+crate=[A-Za-z0-9_.+-]+[[:space:]]+expires=[0-9]{4}-[0-9]{2}-[0-9]{2}'; then
      echo "  $f: $id — undated: no 'waiver: crate=<name> expires=<YYYY-MM-DD>' marker" >&2
      findings=$((findings + 1))
      continue
    fi

    crate="$(printf '%s' "$line" \
      | sed -E 's/.*waiver:[[:space:]]+crate=([A-Za-z0-9_.+-]+)[[:space:]]+expires=[0-9]{4}-[0-9]{2}-[0-9]{2}.*/\1/')"
    expires="$(printf '%s' "$line" \
      | sed -E 's/.*waiver:[[:space:]]+crate=[A-Za-z0-9_.+-]+[[:space:]]+expires=([0-9]{4}-[0-9]{2}-[0-9]{2}).*/\1/')"

    if ! grep -qxF "$crate" "$tmp/lockcrates"; then
      echo "  $f: $id — crate '$crate' is absent from $lock: the waiver suppresses" >&2
      echo "      nothing today and would mask the advisory if the crate returned." >&2
      findings=$((findings + 1))
      continue
    fi

    if [[ "$expires" < "$anchor" ]]; then
      echo "  $f: $id — expired $expires (anchor $anchor): re-justify or remove." >&2
      findings=$((findings + 1))
      continue
    fi

    echo "  OK $f: $id — crate '$crate' present, valid through $expires"
  done < <(grep -E '^[^#]*"RUSTSEC-[0-9]{4}-[0-9]{4}"' "$f" || true)
done

echo "waiver entries examined: $examined"

# A run that examined nothing has not proven anything. Either both ignore lists
# really are empty — in which case this file should be deleted along with them —
# or the grep stopped matching the format. Both are failures.
if [ "$examined" -eq 0 ]; then
  blocked "zero waiver entries examined — the ignore lists are empty or the format moved"
fi

if [ "$findings" -gt 0 ]; then
  echo "check failed: $findings advisory waiver(s) no longer describe the locked graph" >&2
  exit 1
fi

echo "OK: $examined advisory waiver(s), each dated, unexpired, and naming a crate in $lock."
