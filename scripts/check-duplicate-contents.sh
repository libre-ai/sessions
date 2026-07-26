#!/usr/bin/env bash
set -euo pipefail

# Fails when two tracked files carry byte-identical content outside the
# allowlist below. Files are joined on their SHA-256, never on their name: two
# files sharing a basename are routinely different (this tree has 13 duplicated
# basenames and only 2 duplicated contents), and a name-based check would have
# reported eleven false positives while missing a rename.
#
# The allowlist is deliberately exhaustive rather than pattern-based. Every
# accepted group carries the reason it is accepted, so a reviewer arbitrates a
# named case instead of a wildcard. A stale entry — one whose files no longer
# share content — is also a failure: an allowlist that is not true is worse
# than no allowlist.

root="${ROOT:-$(git rev-parse --show-toplevel)}"
cd "$root"

# Resolved as an argv array, not a shell function: `xargs` execs a real binary
# and would never see a function definition.
if command -v sha256sum >/dev/null 2>&1; then
  hash_argv=(sha256sum)
else
  hash_argv=(shasum -a 256)
fi

# Accepted identical groups. One record per line: paths sorted, joined by " | ".
accepted() {
  cat <<'EOF'
crates/ui/fixtures/portal/tokens.css | crates/ui/src/tokens.css
crates/server/assets/rust-ownership-guide.md | docs/sources/rust-ownership-guide.md
EOF
}

# Reasons, kept next to the entries they justify:
#
# 1. tokens.css — both copies are load-bearing and neither can be deleted.
#    `crates/ui/src/tokens.css` is embedded as the shipped `TOKENS_CSS`
#    constant; `crates/ui/fixtures/portal/tokens.css` is the vendored
#    `portal-forge` emission whose digest is recorded in the sibling
#    `manifest.json`. They are identical because the vendored artefact IS the
#    shipped one today. Their equality is not currently asserted anywhere —
#    see the report accompanying this script.
# 2. rust-ownership-guide.md — one copy is embedded via `include_str!` in
#    `crates/server/src/grounded_fixtures.rs`, the other is unreferenced.
#    Whether `docs/sources/` is a deliberate documentary home for the ingested
#    corpus is an owner decision, so the pair is recorded, not resolved.

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Regular blobs only: symlinks would be followed and hashed as their target.
git ls-files -s \
  | awk -F'\t' '$1 ~ /^100(644|755) /{print $2}' \
  > "$tmp/files"

examined="$(wc -l < "$tmp/files" | tr -d ' ')"
echo "files examined: $examined"

if [ "$examined" -eq 0 ]; then
  echo "check failed: zero files examined — the file listing is broken, not clean" >&2
  exit 1
fi

tr '\n' '\0' < "$tmp/files" | xargs -0 "${hash_argv[@]}" > "$tmp/hashes"

hashed="$(wc -l < "$tmp/hashes" | tr -d ' ')"
if [ "$hashed" -ne "$examined" ]; then
  echo "check failed: hashed $hashed files but listed $examined" >&2
  exit 1
fi

# Group by hash, keep groups of 2+, emit "path | path | ..." with paths sorted.
awk '{ h = $1; sub(/^[0-9a-f]+[ ]+[*]?/, ""); paths[h] = (h in paths ? paths[h] "\n" $0 : $0); n[h]++ }
     END { for (h in n) if (n[h] > 1) { print paths[h] | "sort" ; close("sort"); print "\036" } }' \
  "$tmp/hashes" \
  | awk 'BEGIN{ RS="\036\n"; FS="\n" }
         NF { line=""; for (i = 1; i <= NF; i++) if ($i != "") line = (line == "" ? $i : line " | " $i); if (line != "") print line }' \
  | sort > "$tmp/found"

accepted | sed '/^$/d' | sort > "$tmp/accepted"

status=0

unexpected="$(comm -23 "$tmp/found" "$tmp/accepted")"
if [ -n "$unexpected" ]; then
  echo "check failed: identical file contents outside the allowlist" >&2
  printf '%s\n' "$unexpected" | sed 's/^/  /' >&2
  status=1
fi

stale="$(comm -13 "$tmp/found" "$tmp/accepted")"
if [ -n "$stale" ]; then
  echo "check failed: allowlist entries no longer describe identical files" >&2
  printf '  %s\n' "$stale" >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "OK: $(wc -l < "$tmp/found" | tr -d ' ') accepted duplicate group(s), no new one."
fi

exit "$status"
