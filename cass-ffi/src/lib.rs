//! cass-ffi — Swift-facing FFI shim for the cass (coding-agent-search) library.
//!
//! ## Surface
//!
//! See `cass-ffi/SURFACE.md` for the full 15-function + 4-handle + ~15-type
//! design (the source of truth, decided in bd `one-jn0t4w`).
//!
//! ## Current state
//!
//! The bridge module below is intentionally minimal: an opaque `CassEngine`
//! handle with a single `version()` static probe. This is enough to verify
//! the entire build pipeline end-to-end:
//!
//! - cargo resolves the path-pin cohort under Vendor/Cass/
//! - cass + cohort compile against nightly rustc
//! - swift-bridge-build generates the Swift module + C header
//! - `cargo build --release --lib` produces a usable libcass_ffi.a
//!
//! ## Why not the full surface yet
//!
//! swift-bridge 0.1.59 has codegen gaps that prevent the natural
//! expression of cass's value types across the bridge:
//!
//! - **`Result<T, SharedStruct>` panics** in `BuiltInResult::custom_c_struct_name`
//!   reaching `to_alpha_numeric_underscore_name`. Affects every fallible
//!   method that wants to return a structured error.
//! - **`Option<SharedStruct>` panics** in `bridged_option.rs:399` when used
//!   as a shared-struct field. Affects e.g. `Option<TimeFilter>` inside
//!   `SearchOpts`.
//! - **Enums with associated-data variants** carrying `Vec<SharedStruct>`
//!   or other complex types panic in `bridged_type.rs:1986`. Affects the
//!   natural shape of `SearchEvent`, `ModelInstallEvent`, `ReindexEvent`.
//! - **Doc comments on shared structs** are rejected outright by the macro.
//!
//! The full surface in SURFACE.md works around these gaps with flat-error
//! envelope structs (e.g. `SearchHitsResult { hits, error_kind, error_message,
//! ... }`) and `kind` discriminants in lieu of tagged unions. The conversion
//! is mechanical but bloats the bridge file substantially and obscures the
//! design intent — we land it incrementally in bd `one-vtg5eh` (actor wrap)
//! alongside real impls, so each method's workaround is visible next to its
//! real Rust code.
//!
//! Alternative: bump to a forked / patched swift-bridge that fixes these
//! cases. Tracked separately.

mod types;
pub use types::CassError;

#[swift_bridge::bridge]
mod ffi {
    extern "Rust" {
        type CassEngine;

        #[swift_bridge(associated_to = CassEngine)]
        fn version() -> String;
    }
}

/// Opaque engine handle. Real fields (`runtime: tokio::runtime::Runtime`,
/// `storage: Arc<FrankenStorage>`) land in bd `one-vtg5eh`.
pub struct CassEngine {
    _private: (),
}

impl CassEngine {
    /// Returns the cass-ffi crate version. Smoke-test used by the xcframework
    /// build script to verify the bridge symbol actually links.
    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}
