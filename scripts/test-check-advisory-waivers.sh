#!/usr/bin/env bash
set -euo pipefail

# Negative-case proof for `check-advisory-waivers.sh`.
#
# A guard that has only ever been seen to pass is not known to guard anything.
# Each case below builds a throwaway tree, runs the checker against it, and
# asserts both the exit status and the reason printed. Case 2 is the regression
# for the defect the checker was written for: a waiver naming a crate that has
# left the graph.
#
# Exit statuses under test: 0 conformant, 1 real divergence, 2 unable to search.

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
subject="$root/scripts/check-advisory-waivers.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0
case_no=0

# A lock holding one of the two crates the cases refer to. `rsa` is deliberately
# absent — that absence is the point of case 2.
make_tree() {
  local dir="$1"
  mkdir -p "$dir/.cargo"
  cat >"$dir/Cargo.lock" <<'EOF'
[[package]]
name = "proc-macro-error2"
version = "2.0.1"

[[package]]
name = "biscuit-auth"
version = "6.0.0"
EOF
}

# expect <name> <expected-exit> <expected-substring> -- <env assignments...>
expect() {
  local name="$1" want_code="$2" want_text="$3" dir="$4"
  case_no=$((case_no + 1))
  local out code
  set +e
  out="$(ROOT="$dir" WAIVER_ANCHOR_DATE="${ANCHOR:-2026-07-26}" "$subject" 2>&1)"
  code=$?
  set -e

  local ok=1
  [ "$code" -eq "$want_code" ] || ok=0
  printf '%s' "$out" | grep -qF "$want_text" || ok=0

  if [ "$ok" -eq 1 ]; then
    printf '  ok %d — %s (exit %d)\n' "$case_no" "$name" "$code"
  else
    printf '  FAIL %d — %s: wanted exit %d containing %s, got exit %d\n' \
      "$case_no" "$name" "$want_code" "'$want_text'" "$code" >&2
    printf '%s\n' "$out" | sed 's/^/      | /' >&2
    failures=$((failures + 1))
  fi
}

echo "check-advisory-waivers negative cases:"

# 1. Conformant: dated, unexpired, crate present in the lock.
d="$tmp/clean"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # waiver: crate=proc-macro-error2 expires=2027-01-31\n]\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # waiver: crate=proc-macro-error2 expires=2027-01-31\n]\n' >"$d/deny.toml"
expect "conformant tree passes" 0 "OK: 2 advisory waiver(s)" "$d"

# 2. REGRESSION — the defect of 2026-07-26. A waiver naming `rsa`, which is
#    absent from the lock. Undetectable before this check: cargo-audit and
#    cargo-deny both pass, because an ignore for an advisory that never fires
#    is silent by construction.
d="$tmp/absent"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2023-0071", # waiver: crate=rsa expires=2027-01-31\n]\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = []\n' >"$d/deny.toml"
expect "waiver naming a crate absent from the lock fails" 1 "crate 'rsa' is absent" "$d"

# 3. Undated: the mode that lets a waiver live forever unreviewed.
d="$tmp/undated"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # proc-macro-error2 unmaintained\n]\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = []\n' >"$d/deny.toml"
expect "undated waiver fails" 1 "undated" "$d"

# 4. Expired against the anchor.
d="$tmp/expired"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # waiver: crate=proc-macro-error2 expires=2026-01-01\n]\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = []\n' >"$d/deny.toml"
expect "expired waiver fails" 1 "expired 2026-01-01 (anchor 2026-07-26)" "$d"

# 5. The same tree, judged from an earlier anchor, passes. This is the property
#    that keeps the gate deterministic: the verdict is a function of the commit
#    under test, so a required check cannot turn red overnight on an unchanged
#    tree, and re-running an old commit reproduces its original verdict.
ANCHOR=2025-12-31 expect "same tree passes at an earlier anchor" 0 "OK: 1 advisory waiver(s)" "$d"

# 6. An advisory id mentioned in prose is documentation, not a waiver. deny.toml
#    carries exactly such a line about the removed rsa entry; counting it would
#    demand an expiry on a sentence.
d="$tmp/prose"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # waiver: crate=proc-macro-error2 expires=2027-01-31\n]\n' >"$d/.cargo/audit.toml"
printf '# the former "RUSTSEC-2023-0071" waiver is gone from the graph\n[advisories]\nignore = []\n' >"$d/deny.toml"
expect "prose mention is not counted as an entry" 0 "waiver entries examined: 1" "$d"

# 7. Zero entries examined is a failure, not a pass: either the lists are empty
#    and this checker should go with them, or the format moved underneath it.
d="$tmp/zero"; make_tree "$d"
printf '[advisories]\nignore = []\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = []\n' >"$d/deny.toml"
expect "zero entries examined blocks" 2 "zero waiver entries examined" "$d"

# 8. Unable to search: a config file the checker depends on is gone.
d="$tmp/nolock"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # waiver: crate=proc-macro-error2 expires=2027-01-31\n]\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = []\n' >"$d/deny.toml"
rm -f "$d/Cargo.lock"
expect "missing Cargo.lock blocks rather than passes" 2 "expected tracked file 'Cargo.lock' is missing" "$d"

# 9. Unable to search: no anchor can be determined. Proven by pointing the
#    checker at a directory outside any git repository with the override unset,
#    so the fallback `git log` finds nothing.
d="$tmp/noanchor"; make_tree "$d"
printf '[advisories]\nignore = [\n    "RUSTSEC-2026-0173", # waiver: crate=proc-macro-error2 expires=2027-01-31\n]\n' >"$d/.cargo/audit.toml"
printf '[advisories]\nignore = []\n' >"$d/deny.toml"
case_no=$((case_no + 1))
set +e
out="$(ROOT="$d" WAIVER_ANCHOR_DATE="" "$subject" 2>&1)"
code=$?
set -e
if [ "$code" -eq 2 ] && printf '%s' "$out" | grep -qF "no usable anchor date"; then
  printf '  ok %d — undeterminable anchor blocks (exit 2)\n' "$case_no"
else
  printf '  FAIL %d — undeterminable anchor: got exit %d\n' "$case_no" "$code" >&2
  printf '%s\n' "$out" | sed 's/^/      | /' >&2
  failures=$((failures + 1))
fi

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures of $case_no case(s) failed" >&2
  exit 1
fi
echo "all $case_no case(s) passed"
