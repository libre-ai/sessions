#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="${1:-$root/crates/server/static/join-app}"

python3 - "$bundle" <<'PY'
from pathlib import Path
import hashlib
import json
import re
import sys

bundle = Path(sys.argv[1]).resolve()
index = bundle / "index.html"
assets_dir = bundle / "assets"
if not index.is_file() or not assets_dir.is_dir():
    raise SystemExit(f"join bundle is missing under {bundle}; run ./scripts/build-join-app.sh")

expected_root = {"index.html", "join-shell-manifest.json", "assets"}
if {path.name for path in bundle.iterdir()} != expected_root:
    raise SystemExit("join bundle root allowlist diverged")

symlinks = sorted(path for path in bundle.rglob("*") if path.is_symlink())
if symlinks:
    raise SystemExit(f"join bundle must not contain symlinks: {symlinks}")

assets = sorted(path for path in assets_dir.iterdir() if path.is_file())
unsafe_names = [path.name for path in assets if not re.fullmatch(r"[A-Za-z0-9._-]+", path.name)]
unsupported = [path.name for path in assets if path.suffix not in {".js", ".wasm", ".css"}]
unversioned = [path.name for path in assets if not re.search(r"-(?:dxh)?[0-9a-f]{8,}\.(?:js|wasm|css)$", path.name)]
if unsafe_names or unsupported or unversioned:
    raise SystemExit(f"invalid join assets: unsafe={unsafe_names}, unsupported={unsupported}, unversioned={unversioned}")

wasm = [path for path in assets if path.suffix == ".wasm"]
javascript = [path for path in assets if path.suffix == ".js"]
styles = [path for path in assets if path.suffix == ".css"]
runtimes = [path for path in javascript if re.fullmatch(r"join-runtime-[0-9a-f]{16}\.js", path.name)]
runtime_wasm = [path for path in wasm if re.fullmatch(r"join-runtime-[0-9a-f]{16}\.wasm", path.name)]
shell_styles = [path for path in styles if re.fullmatch(r"join-shell-[0-9a-f]{16}\.css", path.name)]
if len(wasm) != 1 or len(runtime_wasm) != 1 or len(javascript) != 1 or len(styles) != 1 or len(runtimes) != 1 or len(shell_styles) != 1:
    raise SystemExit(
        "unexpected join asset topology: "
        f"wasm={[p.name for p in wasm]}, js={[p.name for p in javascript]}, css={[p.name for p in styles]}"
    )
for generated in [runtimes[0], runtime_wasm[0], shell_styles[0]]:
    filename_digest = generated.stem.rsplit("-", 1)[-1]
    final_digest = hashlib.sha256(generated.read_bytes()).hexdigest()
    if filename_digest != final_digest[:16]:
        raise SystemExit(f"generated asset filename does not address its final bytes: {generated.name} != {final_digest[:16]}")

html = index.read_text(encoding="utf-8")
if re.search(r"(?:src|href)=[\"'](?:https?:)?//", html, re.IGNORECASE):
    raise SystemExit("join bundle index must not load remote assets")
if re.search(r"<style\b|\sstyle\s*=|<script(?![^>]+\bsrc=)", html, re.IGNORECASE):
    raise SystemExit("join HTML contains inline style, style attribute, or inline script")
for required in [
    'name="theme-color"',
    'name="referrer"',
]:
    if required not in html:
        raise SystemExit(f"join HTML metadata missing: {required}")

texts = [html, *(path.read_text(encoding="utf-8") for path in javascript)]
references = {match for text in texts for match in re.findall(r"/join/assets/([A-Za-z0-9._-]+)", text)}
missing = sorted(name for name in references if not (assets_dir / name).is_file())
if missing or {path.name for path in assets} != references:
    raise SystemExit(f"asset references diverge: missing={missing}, refs={sorted(references)}, files={[p.name for p in assets]}")
if wasm[0].name not in references:
    raise SystemExit("join JavaScript does not reference its WASM")

internal = json.loads((bundle / "join-shell-manifest.json").read_text(encoding="utf-8"))
if internal.get("schema") != "rumble.join-shell.v1":
    raise SystemExit("unsupported internal join manifest")
entries = internal.get("precache")
if not isinstance(entries, list) or entries != sorted(entries, key=lambda item: item["url"]):
    raise SystemExit("precache entries must be sorted")
expected_files = [index, *assets]
expected_urls = {"/join" if path == index else "/join/" + path.relative_to(bundle).as_posix() for path in expected_files}
if {item["url"] for item in entries} != expected_urls or len(entries) != len(expected_urls):
    raise SystemExit("precache allowlist is not exactly the public static shell")
for item in entries:
    relative = "index.html" if item["url"] == "/join" else item["url"].removeprefix("/join/")
    if hashlib.sha256((bundle / relative).read_bytes()).hexdigest() != item["sha256"]:
        raise SystemExit(f"precache digest mismatch: {item['url']}")
canonical = (json.dumps(entries, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n").encode()
bundle_id = hashlib.sha256(canonical).hexdigest()
if internal.get("bundle_id") != bundle_id:
    raise SystemExit("join bundle_id mismatch")

for forbidden in ["skipWaiting", "cache.put", "caches.match", "unsafe-inline", "'unsafe-eval'", "innerHTML", "new Function", "eval("]:
    if forbidden in html or any(forbidden in text for text in texts):
        raise SystemExit(f"forbidden join shell construct: {forbidden}")
if re.search(r"rumble-lm-join-(?:dxh)?[0-9a-f]+\.(?:js|wasm)", html):
    raise SystemExit("old dx asset reference remains in final join bundle")
if "/join/assets/join-shell.css" in html:
    raise SystemExit("unresolved join CSS placeholder remains")

size = sum(path.stat().st_size for path in bundle.rglob("*") if path.is_file())
print(f"join bundle verified: {sum(1 for p in bundle.rglob('*') if p.is_file())} files, {size} bytes, bundle_id={bundle_id}")
PY
