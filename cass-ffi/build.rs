use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bridges = vec![manifest_dir.join("src/lib.rs")];
    for path in &bridges {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let out_dir = manifest_dir.join("generated");
    swift_bridge_build::parse_bridges(bridges)
        .write_all_concatenated(&out_dir, "cass_ffi");
}
