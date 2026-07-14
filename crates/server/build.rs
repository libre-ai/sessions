use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("bundle directory") {
        let path = entry.expect("bundle entry").path();
        assert!(
            !fs::symlink_metadata(&path)
                .expect("bundle metadata")
                .file_type()
                .is_symlink(),
            "bundle must not contain symlinks"
        );
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("webmanifest") => "application/manifest+json",
        Some("json") => "application/json; charset=utf-8",
        extension => panic!("unsupported bundle file extension: {extension:?}"),
    }
}

fn emit_owner_bundle(manifest_dir: &Path, out_dir: &Path) {
    let owner_dir = manifest_dir.join("static/owner-app");
    println!("cargo:rerun-if-changed={}", owner_dir.display());
    let index = owner_dir.join("index.html");
    let internal_path = owner_dir.join("owner-shell-manifest.json");
    if !index.is_file() || !internal_path.is_file() {
        panic!(
            "owner bundle is missing at {}; run ./scripts/build-owner-app.sh before building presto-server",
            owner_dir.display()
        );
    }

    let internal_bytes = fs::read(&internal_path).expect("read owner shell manifest");
    let internal: Value =
        serde_json::from_slice(&internal_bytes).expect("parse owner shell manifest");
    assert_eq!(
        internal["schema"], "rumble.owner-shell.v1",
        "unsupported owner shell manifest"
    );
    let entries = internal["precache"]
        .as_array()
        .expect("owner precache must be an array");
    let mut expected = BTreeSet::new();
    let mut previous = "";
    for entry in entries {
        let url = entry["url"].as_str().expect("precache URL");
        assert!(url > previous, "owner precache must be strictly sorted");
        previous = url;
        let relative = if url == "/app" {
            "index.html"
        } else {
            url.strip_prefix("/app/")
                .expect("precache URL must stay below /app/")
        };
        let path = owner_dir.join(relative);
        assert!(path.is_file(), "precache file is missing: {relative}");
        assert_eq!(
            sha256(&fs::read(&path).expect("read precache file")),
            entry["sha256"].as_str().expect("precache SHA-256"),
            "precache digest mismatch: {relative}"
        );
        expected.insert(relative.to_owned());
    }
    let mut canonical = serde_json::to_vec(entries).expect("serialize owner precache");
    canonical.push(b'\n');
    assert_eq!(
        sha256(&canonical),
        internal["bundle_id"].as_str().expect("owner bundle_id"),
        "owner bundle_id mismatch"
    );
    expected.insert("owner-shell-manifest.json".to_owned());
    expected.insert("sw.js".to_owned());

    let mut files = Vec::new();
    collect_files(&owner_dir, &mut files);
    files.sort();
    let actual = files
        .iter()
        .map(|path| {
            path.strip_prefix(&owner_dir)
                .expect("owner path")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "owner bundle file allowlist diverged");

    let mut generated =
        String::from("pub(crate) const OWNER_APP_FILES: &[EmbeddedOwnerFile] = &[\n");
    for path in files.into_iter().filter(|path| path != &index) {
        let relative = path
            .strip_prefix(&owner_dir)
            .expect("owner relative path")
            .to_string_lossy()
            .replace('\\', "/");
        let body = fs::read(&path).expect("read owner file");
        generated.push_str(&format!(
            "    EmbeddedOwnerFile {{ path: {relative:?}, content_type: {content_type:?}, etag: {etag:?}, body: include_bytes!({path:?}) }},\n",
            content_type = content_type(&path),
            etag = sha256(&body),
            path = path.display().to_string(),
        ));
    }
    generated.push_str("];\n");

    let output = out_dir.join("owner_app_assets.rs");
    fs::write(&output, generated)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", output.display()));
}

fn emit_join_bundle(manifest_dir: &Path, out_dir: &Path) {
    let join_dir = manifest_dir.join("static/join-app");
    println!("cargo:rerun-if-changed={}", join_dir.display());
    let index = join_dir.join("index.html");
    let internal_path = join_dir.join("join-shell-manifest.json");
    if !index.is_file() || !internal_path.is_file() {
        panic!(
            "join bundle is missing at {}; run ./scripts/build-join-app.sh before building presto-server",
            join_dir.display()
        );
    }

    let internal_bytes = fs::read(&internal_path).expect("read join shell manifest");
    let internal: Value =
        serde_json::from_slice(&internal_bytes).expect("parse join shell manifest");
    assert_eq!(
        internal["schema"], "rumble.join-shell.v1",
        "unsupported join shell manifest"
    );
    let entries = internal["precache"]
        .as_array()
        .expect("join precache must be an array");
    let mut expected = BTreeSet::new();
    let mut previous = "";
    for entry in entries {
        let url = entry["url"].as_str().expect("precache URL");
        assert!(url > previous, "join precache must be strictly sorted");
        previous = url;
        let relative = if url == "/join" {
            "index.html"
        } else {
            url.strip_prefix("/join/")
                .expect("precache URL must stay below /join/")
        };
        let path = join_dir.join(relative);
        assert!(path.is_file(), "precache file is missing: {relative}");
        assert_eq!(
            sha256(&fs::read(&path).expect("read precache file")),
            entry["sha256"].as_str().expect("precache SHA-256"),
            "precache digest mismatch: {relative}"
        );
        expected.insert(relative.to_owned());
    }
    let mut canonical = serde_json::to_vec(entries).expect("serialize join precache");
    canonical.push(b'\n');
    assert_eq!(
        sha256(&canonical),
        internal["bundle_id"].as_str().expect("join bundle_id"),
        "join bundle_id mismatch"
    );
    expected.insert("join-shell-manifest.json".to_owned());

    let mut files = Vec::new();
    collect_files(&join_dir, &mut files);
    files.sort();
    let actual = files
        .iter()
        .map(|path| {
            path.strip_prefix(&join_dir)
                .expect("join path")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "join bundle file allowlist diverged");

    let mut generated = String::from("pub(crate) const JOIN_APP_FILES: &[EmbeddedJoinFile] = &[\n");
    for path in files.into_iter().filter(|path| path != &index) {
        let relative = path
            .strip_prefix(&join_dir)
            .expect("join relative path")
            .to_string_lossy()
            .replace('\\', "/");
        let body = fs::read(&path).expect("read join file");
        generated.push_str(&format!(
            "    EmbeddedJoinFile {{ path: {relative:?}, content_type: {content_type:?}, etag: {etag:?}, body: include_bytes!({path:?}) }},\n",
            content_type = content_type(&path),
            etag = sha256(&body),
            path = path.display().to_string(),
        ));
    }
    generated.push_str("];\n");

    let output = out_dir.join("join_app_assets.rs");
    fs::write(&output, generated)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", output.display()));
}

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    emit_owner_bundle(&manifest_dir, &out_dir);
    emit_join_bundle(&manifest_dir, &out_dir);
}
