use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let output = PathBuf::from(&crate_dir).join("toku.h");

    let config = cbindgen::Config::from_file("cbindgen.toml").unwrap_or_default();

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed to generate header")
        .write_to_file(&output);
}
