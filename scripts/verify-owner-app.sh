#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="${1:-$root/crates/server/static/owner-app}"

python3 - "$bundle" <<'PY'
from pathlib import Path
import re
import sys

bundle = Path(sys.argv[1]).resolve()
index = bundle / "index.html"
assets_dir = bundle / "assets"

if not index.is_file() or not assets_dir.is_dir():
    raise SystemExit(
        f"owner bundle is missing under {bundle}; run ./scripts/build-owner-app.sh"
    )

symlinks = sorted(path for path in bundle.rglob("*") if path.is_symlink())
if symlinks:
    raise SystemExit(f"owner bundle must not contain symlinks: {symlinks}")

assets = sorted(path for path in assets_dir.iterdir() if path.is_file())
if any(path.is_dir() for path in assets_dir.iterdir()):
    raise SystemExit("owner bundle assets must be a flat directory")
unsafe_names = [path.name for path in assets if not re.fullmatch(r"[A-Za-z0-9._-]+", path.name)]
if unsafe_names:
    raise SystemExit(f"owner bundle contains unsafe asset names: {unsafe_names}")

wasm = [path for path in assets if path.suffix == ".wasm"]
javascript = [path for path in assets if path.suffix == ".js"]
unsupported = [path for path in assets if path.suffix not in {".js", ".wasm"}]
if len(wasm) != 1 or len(javascript) < 1 or unsupported:
    raise SystemExit(
        "owner bundle must contain exactly one WASM and at least one JS asset "
        f"(wasm={len(wasm)}, js={len(javascript)}, unsupported={unsupported})"
    )

html = index.read_text(encoding="utf-8")
if re.search(r"(?:src|href)=[\"'](?:https?:)?//", html, re.IGNORECASE):
    raise SystemExit("owner bundle index must not load CDN or remote assets")

texts = [html, *(path.read_text(encoding="utf-8") for path in javascript)]
references = {
    match
    for text in texts
    for match in re.findall(r"/app/assets/([A-Za-z0-9._-]+)", text)
}
if not references:
    raise SystemExit("owner bundle has no /app/assets references")
missing = sorted(name for name in references if not (assets_dir / name).is_file())
if missing:
    raise SystemExit(f"owner bundle references missing assets: {missing}")
if wasm[0].name not in references:
    raise SystemExit("owner bundle JavaScript does not reference its WASM asset")
if not any(path.name in references for path in javascript):
    raise SystemExit("owner bundle index does not reference its JavaScript entry point")

size = sum(path.stat().st_size for path in [index, *assets])
print(f"owner bundle verified: {len(assets) + 1} files, {size} bytes, no remote assets")
PY
