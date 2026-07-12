#!/usr/bin/env python3
"""Create the deterministic, content-addressed owner PWA shell after `dx build`."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parent.parent
CSP = "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; style-src-attr 'none'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n").encode()


def png(size: int, maskable: bool = False) -> bytes:
    # Flat, deterministic RGBA icon: dark field, safe green inset and an R glyph.
    inset = size // (8 if maskable else 12)
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            green = inset <= x < size - inset and inset <= y < size - inset
            stem = size * 31 // 100 <= x < size * 40 // 100 and size * 27 // 100 <= y < size * 73 // 100
            bowl = size * 40 // 100 <= x < size * 65 // 100 and (
                size * 27 // 100 <= y < size * 36 // 100
                or size * 45 // 100 <= y < size * 54 // 100
                or size * 58 // 100 <= x < size * 67 // 100 and size * 34 // 100 <= y < size * 48 // 100
            )
            leg = size * 43 // 100 <= x < size * 67 // 100 and size * 52 // 100 <= y < size * 73 // 100 and x - size * 43 // 100 >= (y - size * 52 // 100) // 2
            if stem or bowl or leg:
                rgba = (17, 24, 39, 255)
            elif green:
                rgba = (34, 197, 94, 255)
            else:
                rgba = (0, 0, 0, 255)
            row.extend(rgba)
        rows.append(bytes(row))

    def chunk(kind: bytes, payload: bytes) -> bytes:
        body = kind + payload
        return struct.pack(">I", len(payload)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(b"".join(rows), level=9))
        + chunk(b"IEND", b"")
    )


def main(bundle: Path) -> None:
    bundle = bundle.resolve()
    index_path = bundle / "index.html"
    assets = bundle / "assets"
    if not index_path.is_file() or not assets.is_dir():
        raise SystemExit("dx owner output is incomplete")

    # The dependency-free UI must produce exactly one dx runtime and one WASM.
    # Fail closed on any extra JavaScript rather than pruning generated output.
    generated_javascript = sorted(assets.glob("*.js"))
    runtimes = [path for path in generated_javascript if re.fullmatch(r"rumble-lm-app-(?:dxh)?[0-9a-f]+\.js", path.name)]
    wasm_files = sorted(assets.glob("*.wasm"))
    if len(runtimes) != 1 or len(wasm_files) != 1 or len(generated_javascript) != 1:
        raise SystemExit(
            "unexpected dx asset topology; expected one runtime JS + one WASM, "
            f"found js={[path.name for path in generated_javascript]}, wasm={[path.name for path in wasm_files]}"
        )
    runtime = runtimes[0]
    wasm = wasm_files[0]
    old_runtime_name = runtime.name
    old_wasm_name = wasm.name

    # The WASM is opaque: derive its final name from the exact dx bytes and only
    # rename it. No section, metadata, path or payload parsing/mutation is done.
    wasm_digest = digest(wasm.read_bytes())
    wasm_name = f"owner-runtime-{wasm_digest[:16]}.wasm"
    wasm_path = assets / wasm_name
    if wasm_path.exists():
        raise SystemExit(f"refusing to overwrite generated WASM: {wasm_name}")
    wasm.rename(wasm_path)
    if digest(wasm_path.read_bytes()) != wasm_digest:
        raise SystemExit("WASM bytes changed while content-addressing")

    # Patch the JavaScript first, including its reference to the renamed opaque
    # WASM, then derive the JavaScript name from those final bytes.
    source = runtime.read_bytes()
    wasm_references = source.count(old_wasm_name.encode())
    if wasm_references < 1:
        raise SystemExit(f"dx runtime does not reference its WASM: {old_wasm_name}")
    source = source.replace(old_wasm_name.encode(), wasm_name.encode())
    source = source.replace(b'.innerHTML=e', b'.replaceChildren(document.createTextNode(e))')
    source = source.replace(b'.innerHTML=""', b'.replaceChildren()')
    source = re.sub(
        rb"return new Function\(g\(t,e\),g\(n,r\)\)",
        b'throw new Error("dynamic code disabled")',
        source,
    )
    if old_runtime_name.encode() in source:
        raise SystemExit("dx runtime self-reference would require recursive content addressing")
    runtime_name = f"owner-runtime-{digest(source)[:16]}.js"
    runtime_path = assets / runtime_name
    if runtime_path.exists():
        raise SystemExit(f"refusing to overwrite generated runtime: {runtime_name}")
    runtime_path.write_bytes(source)
    runtime.unlink()

    # Rewrite every textual reference only after the final runtime bytes and
    # filename are known. The old immutable URL must disappear entirely.
    old_runtime_url = f"/app/assets/{old_runtime_name}"
    runtime_url = f"/app/assets/{runtime_name}"
    reference_files = [index_path, *sorted(assets.glob("*.css")), *sorted(assets.glob("*.json"))]
    replacement_count = 0
    for path in reference_files:
        data = path.read_bytes()
        replacement_count += data.count(old_runtime_name.encode())
        data = data.replace(old_runtime_url.encode(), runtime_url.encode())
        data = data.replace(old_runtime_name.encode(), runtime_name.encode())
        path.write_bytes(data)
    if replacement_count < 1:
        raise SystemExit(f"dx HTML does not reference its runtime: {old_runtime_name}")

    css_sources = [
        ROOT / "crates/ui/src/tokens.css",
        ROOT / "crates/ui/src/themes.css",
        ROOT / "crates/ui/src/portal-bridge.css",
        ROOT / "crates/ui/src/components.css",
        ROOT / "crates/app/src/owner.css",
    ]
    css = b"\n".join(path.read_bytes().rstrip() for path in css_sources) + b"\n"
    css_name = f"owner-shell-{digest(css)[:16]}.css"
    (assets / css_name).write_bytes(css)

    registration = (ROOT / "scripts/owner-sw-register.js").read_bytes()
    registration_name = f"owner-sw-register-{digest(registration)[:16]}.js"
    (assets / registration_name).write_bytes(registration)

    html = index_path.read_text(encoding="utf-8")
    replacements = {
        "/app/assets/owner-shell.css": f"/app/assets/{css_name}",
        "/app/assets/owner-sw-register.js": f"/app/assets/{registration_name}",
    }
    for old, new in replacements.items():
        if html.count(old) != 1:
            raise SystemExit(f"expected exactly one owner HTML placeholder: {old}")
        html = html.replace(old, new)
    index_path.write_text(html, encoding="utf-8", newline="\n")

    icons = bundle / "icons"
    icons.mkdir(exist_ok=True)
    icon_specs = {
        "icon-192.png": (192, False),
        "icon-512.png": (512, False),
        "maskable-192.png": (192, True),
        "maskable-512.png": (512, True),
        "apple-touch-icon.png": (180, False),
    }
    for name, (size, maskable) in icon_specs.items():
        (icons / name).write_bytes(png(size, maskable))

    webmanifest = {
        "id": "/app/",
        "name": "Rumble LM — espace owner",
        "short_name": "Rumble LM",
        "description": "Notebook owner souverain, avec shell hors ligne sans données utilisateur.",
        "lang": "fr",
        "display": "standalone",
        "scope": "/app/",
        "start_url": "/app/",
        "background_color": "#000000",
        "theme_color": "#000000",
        "icons": [
            {"src": "/app/icons/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any"},
            {"src": "/app/icons/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any"},
            {"src": "/app/icons/maskable-192.png", "sizes": "192x192", "type": "image/png", "purpose": "maskable"},
            {"src": "/app/icons/maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable"},
        ],
    }
    (bundle / "manifest.webmanifest").write_bytes(canonical(webmanifest))

    precache_files = [index_path, bundle / "manifest.webmanifest"]
    precache_files.extend(sorted(path for path in assets.iterdir() if path.is_file()))
    precache_files.extend(sorted(path for path in icons.iterdir() if path.is_file()))
    entries = []
    for path in precache_files:
        relative = path.relative_to(bundle).as_posix()
        url = "/app" if relative == "index.html" else f"/app/{relative}"
        entries.append({"url": url, "sha256": digest(path.read_bytes())})
    entries.sort(key=lambda item: item["url"])
    bundle_id = digest(canonical(entries))

    internal = {
        "schema": "rumble.owner-shell.v1",
        "bundle_id": bundle_id,
        "content_security_policy": CSP,
        "precache": entries,
        "excluded_from_bundle_id": ["owner-shell-manifest.json", "sw.js"],
    }
    (bundle / "owner-shell-manifest.json").write_bytes(canonical(internal))

    urls = [entry["url"] for entry in entries]
    sw = f'''"use strict";
const CACHE_PREFIX = "rumble-owner-shell-v1-";
const CACHE_NAME = CACHE_PREFIX + "{bundle_id}";
const SHELL_URL = "/app";
const PRECACHE_URLS = {json.dumps(urls, separators=(",", ":"))};
const PRECACHE = new Set(PRECACHE_URLS);
const NETWORK_ONLY_PREFIXES = ["/auth/","/api/","/corpus/","/sessions/","/ws/"];

function eligible(request) {{
  const url = new URL(request.url);
  return request.method === "GET" && url.origin === self.location.origin &&
    !request.headers.has("Authorization") &&
    !NETWORK_ONLY_PREFIXES.some((prefix) => url.pathname.startsWith(prefix));
}}

self.addEventListener("install", (event) => {{
  event.waitUntil(caches.open(CACHE_NAME).then((cache) => cache.addAll(PRECACHE_URLS)));
}});

self.addEventListener("activate", (event) => {{
  event.waitUntil(caches.keys().then((names) => Promise.all(
    names.filter((name) => name.startsWith(CACHE_PREFIX) && name !== CACHE_NAME)
      .map((name) => caches.delete(name))
  )).then(() => self.clients.claim()));
}});

self.addEventListener("fetch", (event) => {{
  const request = event.request;
  if (!eligible(request)) return;
  const url = new URL(request.url);
  const navigation = request.mode === "navigate" &&
    (url.pathname === "/app" || url.pathname.startsWith("/app/"));
  if (navigation) {{
    event.respondWith(fetch(request).catch(() =>
      caches.open(CACHE_NAME).then((cache) => cache.match(SHELL_URL))
    ));
    return;
  }}
  const cacheKey = url.pathname + url.search;
  if (PRECACHE.has(cacheKey)) {{
    event.respondWith(
      caches.open(CACHE_NAME).then((cache) => cache.match(cacheKey))
        .then((cached) => cached || fetch(request))
    );
  }}
}});
'''.encode()
    (bundle / "sw.js").write_bytes(sw)

    forbidden = [
        str(ROOT).encode(),
        str(Path.home()).encode(),
        old_runtime_name.encode(),
        old_wasm_name.encode(),
        b"unsafe-inline",
        b"'unsafe-eval'",
        b"innerHTML",
        b"new Function",
        b"eval(",
    ]
    for path in sorted(p for p in bundle.rglob("*") if p.is_file()):
        if path.suffix == ".wasm":
            continue  # Opaque by ADR-0005; integrity is established by its SHA-256 name.
        data = path.read_bytes()
        hits = [value for value in forbidden if value and value in data]
        if hits:
            raise SystemExit(f"non-reproducible or unsafe content in {path.relative_to(bundle)}: {hits}")
    print(f"owner PWA finalized: {len(entries)} precache entries, bundle_id={bundle_id}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: finalize-owner-app.py BUNDLE")
    main(Path(sys.argv[1]))
