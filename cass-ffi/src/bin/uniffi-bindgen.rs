// Standalone uniffi-bindgen binary. Invoked by the CassFFI.xcframework
// build script (bd one-4di061) to emit the Swift module + C header that
// the Swift framework wraps around our libcass_ffi.a.
//
// Usage: `cargo run --bin uniffi-bindgen -- generate --library
// target/release/libcass_ffi.dylib --language swift --out-dir <dir>`

fn main() {
    uniffi::uniffi_bindgen_main()
}
