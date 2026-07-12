use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let assets_dir = manifest_dir.join("static/owner-app/assets");
    println!("cargo:rerun-if-changed={}", assets_dir.display());

    let mut assets = fs::read_dir(&assets_dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", assets_dir.display()))
        .map(|entry| entry.expect("owner asset directory entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assets.sort();

    let mut generated =
        String::from("pub(crate) const OWNER_APP_ASSETS: &[EmbeddedOwnerAsset] = &[\n");
    for path in assets {
        println!("cargo:rerun-if-changed={}", path.display());
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("owner asset names must be UTF-8");
        let content_type = match path.extension().and_then(|extension| extension.to_str()) {
            Some("js") => "text/javascript; charset=utf-8",
            Some("wasm") => "application/wasm",
            extension => panic!("unsupported owner asset extension: {extension:?}"),
        };
        generated.push_str(&format!(
            "    EmbeddedOwnerAsset {{ path: {file_name:?}, content_type: {content_type:?}, body: include_bytes!({path:?}) }},\n",
            path = path.display().to_string(),
        ));
    }
    generated.push_str("];\n");

    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("owner_app_assets.rs");
    fs::write(&output, generated)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", output.display()));
}
