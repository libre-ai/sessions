#!/usr/bin/env python3
"""Create the deterministic, content-addressed join PWA shell after `dx build`."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
CSP = "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; style-src-attr 'none'; img-src 'self'; font-src 'self'; connect-src 'self'; worker-src 'none'; manifest-src 'none'"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n").encode()


def main(bundle: Path) -> None:
    bundle = bundle.resolve()
    index_path = bundle / "index.html"
    assets = bundle / "assets"
    if not index_path.is_file() or not assets.is_dir():
        raise SystemExit("dx join output is incomplete")

    generated_js = sorted(assets.glob("*.js"))
    runtimes = [path for path in generated_js if re.fullmatch(r"rumble-lm-join-(?:dxh)?[0-9a-f]+\.js", path.name)]
    wasm_files = sorted(assets.glob("*.wasm"))
    if len(runtimes) != 1 or len(wasm_files) != 1 or len(generated_js) != 1:
        raise SystemExit(
            "unexpected dx asset topology; expected one runtime JS + one WASM, "
            f"found js={[path.name for path in generated_js]}, wasm={[path.name for path in wasm_files]}"
        )
    runtime = runtimes[0]
    wasm = wasm_files[0]
    old_runtime_name = runtime.name
    old_wasm_name = wasm.name

    wasm_digest = digest(wasm.read_bytes())
    wasm_name = f"join-runtime-{wasm_digest[:16]}.wasm"
    wasm_path = assets / wasm_name
    if wasm_path.exists():
        raise SystemExit(f"refusing to overwrite generated WASM: {wasm_name}")
    wasm.rename(wasm_path)
    if digest(wasm_path.read_bytes()) != wasm_digest:
        raise SystemExit("WASM bytes changed while content-addressing")

    source = runtime.read_bytes()
    if old_wasm_name.encode() not in source:
        raise SystemExit(f"dx runtime does not reference its WASM: {old_wasm_name}")
    source = source.replace(old_wasm_name.encode(), wasm_name.encode())
    source = source.replace(b'.innerHTML=e', b'.replaceChildren(document.createTextNode(e))')
    source = source.replace(b'.innerHTML=""', b'.replaceChildren()')
    source = re.sub(rb"return new Function\(g\(t,e\),g\(n,r\)\)", b'throw new Error("dynamic code disabled")', source)
    if old_runtime_name.encode() in source:
        raise SystemExit("dx runtime self-reference would require recursive content addressing")
    runtime_name = f"join-runtime-{digest(source)[:16]}.js"
    runtime_path = assets / runtime_name
    if runtime_path.exists():
        raise SystemExit(f"refusing to overwrite generated runtime: {runtime_name}")
    runtime_path.write_bytes(source)
    runtime.unlink()

    old_runtime_url = f"/join/assets/{old_runtime_name}"
    runtime_url = f"/join/assets/{runtime_name}"
    replacement_count = 0
    for path in [index_path, *sorted(assets.glob("*.css")), *sorted(assets.glob("*.json"))]:
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
        ROOT / "crates/join/src/join.css",
    ]
    css = b"\n".join(path.read_bytes().rstrip() for path in css_sources) + b"\n"
    css_name = f"join-shell-{digest(css)[:16]}.css"
    (assets / css_name).write_bytes(css)

    html = index_path.read_text(encoding="utf-8")
    if html.count("/join/assets/join-shell.css") != 1:
        raise SystemExit("expected exactly one join CSS placeholder")
    html = html.replace("/join/assets/join-shell.css", f"/join/assets/{css_name}")
    index_path.write_text(html, encoding="utf-8", newline="\n")

    precache_files = [index_path, *sorted(path for path in assets.iterdir() if path.is_file())]
    entries = []
    for path in precache_files:
        relative = path.relative_to(bundle).as_posix()
        url = "/join" if relative == "index.html" else f"/join/{relative}"
        entries.append({"url": url, "sha256": digest(path.read_bytes())})
    entries.sort(key=lambda item: item["url"])
    bundle_id = digest(canonical(entries))

    internal = {
        "schema": "rumble.join-shell.v1",
        "bundle_id": bundle_id,
        "content_security_policy": CSP,
        "precache": entries,
        "excluded_from_bundle_id": ["join-shell-manifest.json"],
    }
    (bundle / "join-shell-manifest.json").write_bytes(canonical(internal))

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
            continue
        data = path.read_bytes()
        hits = [value for value in forbidden if value and value in data]
        if hits:
            raise SystemExit(f"non-reproducible or unsafe content in {path.relative_to(bundle)}: {hits}")
    print(f"join PWA finalized: {len(entries)} precache entries, bundle_id={bundle_id}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: finalize-join-app.py BUNDLE")
    main(Path(sys.argv[1]))
