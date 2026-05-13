//! cass-ffi — Swift-facing FFI shim for the cass (coding-agent-search) library.
//!
//! Built on UniFFI 0.31's proc-macro mode (no UDL file). The full design —
//! 15-fn surface, 4 opaque handles, ~15 value types — is documented in
//! `cass-ffi/SURFACE.md`. We chose UniFFI over swift-bridge because UniFFI
//! natively supports `Result<T, CustomError>`, `Option<Struct>` as struct
//! fields, enums with associated-data variants, and async functions —
//! swift-bridge 0.1.59 panics in codegen for all four of those.
//!
//! Current state: scaffold-only. `CassEngine` is an opaque handle with a
//! single `version()` constructor — enough to verify the build pipeline
//! end-to-end. Real method bodies land in bd `one-vtg5eh` (actor wrap),
//! one section of SURFACE.md at a time, against an Arc<FrankenStorage>
//! held inside this engine type.

mod types;
pub use types::CassError;

uniffi::setup_scaffolding!();

#[derive(uniffi::Object)]
pub struct CassEngine {
    _private: (),
}

#[uniffi::export]
impl CassEngine {
    /// Smoke-test constructor. Returns a non-functional engine carrying
    /// only the crate version string. Used by the xcframework build script
    /// to verify the bridge symbols actually link.
    #[uniffi::constructor]
    pub fn probe() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { _private: () })
    }

    /// Returns the cass-ffi crate version. Verified to round-trip through
    /// the UniFFI scaffolding by `cargo run --bin uniffi-bindgen` +
    /// downstream Swift consumers.
    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}
