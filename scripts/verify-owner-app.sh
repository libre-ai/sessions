#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="${1:-$root/crates/server/static/owner-app}"

python3 - "$bundle" <<'PY'
from pathlib import Path
import hashlib
import json
import re
import struct
import sys

bundle = Path(sys.argv[1]).resolve()
index = bundle / "index.html"
assets_dir = bundle / "assets"
icons_dir = bundle / "icons"
expected_root = {"index.html", "manifest.webmanifest", "owner-shell-manifest.json", "sw.js", "assets", "icons"}
if not index.is_file() or not assets_dir.is_dir() or not icons_dir.is_dir():
    raise SystemExit(f"owner bundle is missing under {bundle}; run ./scripts/build-owner-app.sh")
if {path.name for path in bundle.iterdir()} != expected_root:
    raise SystemExit("owner bundle root allowlist diverged")

symlinks = sorted(path for path in bundle.rglob("*") if path.is_symlink())
if symlinks:
    raise SystemExit(f"owner bundle must not contain symlinks: {symlinks}")
if any(path.is_dir() for path in assets_dir.iterdir()) or any(path.is_dir() for path in icons_dir.iterdir()):
    raise SystemExit("owner bundle asset/icon directories must be flat")

assets = sorted(path for path in assets_dir.iterdir() if path.is_file())
unsafe_names = [path.name for path in assets if not re.fullmatch(r"[A-Za-z0-9._-]+", path.name)]
unsupported = [path.name for path in assets if path.suffix not in {".js", ".wasm", ".css"}]
unversioned = [path.name for path in assets if not re.search(r"-(?:dxh)?[0-9a-f]{8,}\.(?:js|wasm|css)$", path.name)]
if unsafe_names or unsupported or unversioned:
    raise SystemExit(f"invalid owner assets: unsafe={unsafe_names}, unsupported={unsupported}, unversioned={unversioned}")
wasm = [path for path in assets if path.suffix == ".wasm"]
javascript = [path for path in assets if path.suffix == ".js"]
styles = [path for path in assets if path.suffix == ".css"]
runtimes = [path for path in javascript if re.fullmatch(r"owner-runtime-[0-9a-f]{16}\.js", path.name)]
runtime_wasm = [path for path in wasm if re.fullmatch(r"owner-runtime-[0-9a-f]{16}\.wasm", path.name)]
registrations = [path for path in javascript if re.fullmatch(r"owner-sw-register-[0-9a-f]{16}\.js", path.name)]
shell_styles = [path for path in styles if re.fullmatch(r"owner-shell-[0-9a-f]{16}\.css", path.name)]
if len(wasm) != 1 or len(runtime_wasm) != 1 or len(javascript) != 2 or len(styles) != 1 or len(runtimes) != 1 or len(registrations) != 1 or len(shell_styles) != 1:
    raise SystemExit(
        "unexpected owner asset topology: "
        f"wasm={[p.name for p in wasm]}, js={[p.name for p in javascript]}, css={[p.name for p in styles]}"
    )
for generated in [runtimes[0], runtime_wasm[0], registrations[0], shell_styles[0]]:
    filename_digest = generated.stem.rsplit("-", 1)[-1]
    final_digest = hashlib.sha256(generated.read_bytes()).hexdigest()
    if filename_digest != final_digest[:16]:
        raise SystemExit(
            f"generated asset filename does not address its final bytes: {generated.name} != {final_digest[:16]}"
        )

html = index.read_text(encoding="utf-8")
if re.search(r"(?:src|href)=[\"'](?:https?:)?//", html, re.IGNORECASE):
    raise SystemExit("owner bundle index must not load remote assets")
if re.search(r"<style\b|\sstyle\s*=|<script(?![^>]+\bsrc=)", html, re.IGNORECASE):
    raise SystemExit("owner HTML contains inline style, style attribute, or inline script")
for required in [
    'rel="manifest" href="/app/manifest.webmanifest"',
    'rel="apple-touch-icon"',
    'name="theme-color"',
    'name="apple-mobile-web-app-capable"',
]:
    if required not in html:
        raise SystemExit(f"owner HTML metadata missing: {required}")

texts = [html, *(path.read_text(encoding="utf-8") for path in javascript)]
references = {match for text in texts for match in re.findall(r"/app/assets/([A-Za-z0-9._-]+)", text)}
missing = sorted(name for name in references if not (assets_dir / name).is_file())
if missing or {path.name for path in assets} != references:
    raise SystemExit(f"asset references diverge: missing={missing}, refs={sorted(references)}, files={[p.name for p in assets]}")
if wasm[0].name not in references:
    raise SystemExit("owner JavaScript does not reference its WASM")

manifest = json.loads((bundle / "manifest.webmanifest").read_text(encoding="utf-8"))
for key, value in {"id": "/app/", "scope": "/app/", "start_url": "/app/", "display": "standalone", "lang": "fr"}.items():
    if manifest.get(key) != value:
        raise SystemExit(f"manifest {key} must be {value!r}")
expected_icons = {
    "/app/icons/icon-192.png": (192, "any"),
    "/app/icons/icon-512.png": (512, "any"),
    "/app/icons/maskable-192.png": (192, "maskable"),
    "/app/icons/maskable-512.png": (512, "maskable"),
}
actual_icons = {item["src"]: (int(item["sizes"].split("x")[0]), item["purpose"]) for item in manifest["icons"] if item.get("type") == "image/png"}
if actual_icons != expected_icons:
    raise SystemExit("manifest icon allowlist, dimensions, MIME or purposes diverged")
expected_pngs = {path.rsplit("/", 1)[-1]: size for path, (size, _) in expected_icons.items()} | {"apple-touch-icon.png": 180}
if {path.name for path in icons_dir.iterdir()} != set(expected_pngs):
    raise SystemExit("icon file allowlist diverged")
for name, expected_size in expected_pngs.items():
    data = (icons_dir / name).read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n") or data[12:16] != b"IHDR":
        raise SystemExit(f"{name} is not a PNG")
    width, height = struct.unpack(">II", data[16:24])
    if (width, height) != (expected_size, expected_size):
        raise SystemExit(f"{name} has dimensions {width}x{height}")

internal = json.loads((bundle / "owner-shell-manifest.json").read_text(encoding="utf-8"))
if internal.get("schema") != "rumble.owner-shell.v1":
    raise SystemExit("unsupported internal owner manifest")
entries = internal.get("precache")
if not isinstance(entries, list) or entries != sorted(entries, key=lambda item: item["url"]):
    raise SystemExit("precache entries must be sorted")
expected_files = [index, bundle / "manifest.webmanifest", *assets, *sorted(icons_dir.iterdir())]
expected_urls = {"/app" if path == index else "/app/" + path.relative_to(bundle).as_posix() for path in expected_files}
if {item["url"] for item in entries} != expected_urls or len(entries) != len(expected_urls):
    raise SystemExit("precache allowlist is not exactly the public static shell")
for item in entries:
    relative = "index.html" if item["url"] == "/app" else item["url"].removeprefix("/app/")
    if hashlib.sha256((bundle / relative).read_bytes()).hexdigest() != item["sha256"]:
        raise SystemExit(f"precache digest mismatch: {item['url']}")
canonical = (json.dumps(entries, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n").encode()
bundle_id = hashlib.sha256(canonical).hexdigest()
if internal.get("bundle_id") != bundle_id:
    raise SystemExit("owner bundle_id mismatch")

sw = (bundle / "sw.js").read_text(encoding="utf-8")
for required in [
    'const CACHE_PREFIX = "rumble-owner-shell-v1-"',
    bundle_id,
    'request.method === "GET"',
    'request.headers.has("Authorization")',
    '["/auth/","/api/","/corpus/","/sessions/","/ws/"]',
    'fetch(request).catch(() => caches.match(SHELL_URL))',
]:
    if required not in sw:
        raise SystemExit(f"service worker invariant missing: {required}")
for forbidden in ["skipWaiting", "cache.put", "unsafe-inline", "'unsafe-eval'", "innerHTML", "new Function", "eval("]:
    if forbidden in sw or forbidden in html:
        raise SystemExit(f"forbidden owner shell construct: {forbidden}")
if json.dumps([item["url"] for item in entries], separators=(",", ":")) not in sw:
    raise SystemExit("service worker precache list diverges from internal manifest")

size = sum(path.stat().st_size for path in bundle.rglob("*") if path.is_file())
print(f"owner bundle verified: {sum(1 for p in bundle.rglob('*') if p.is_file())} files, {size} bytes, bundle_id={bundle_id}")
PY
