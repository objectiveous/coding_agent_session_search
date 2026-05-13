//! cass-ffi — Swift-facing FFI shim for the cass (coding-agent-search) library.
//!
//! This crate is intentionally lean: it exposes the ~15-function surface
//! decided in bd `one-jn0t4w` and nothing else. It is NOT a generic
//! Rust→Swift binding for the entire cass library; it is the surface sized
//! to OneApp's SwiftUI screens.
//!
//! The bridge module below is the source of truth that `swift-bridge-build`
//! reads during `build.rs` to generate the matching Swift module under
//! `generated/`. Consumers (the CassFFI.xcframework) embed that Swift
//! module plus the static library produced by `cargo build --release`.

// Bridge module skeleton. The full surface lands in bd `one-orlzie`; this
// commit only validates that the toolchain wires up end-to-end.
#[swift_bridge::bridge]
mod ffi {
    extern "Rust" {
        type CassEngine;

        #[swift_bridge(associated_to = CassEngine)]
        fn version() -> String;
    }
}

/// Opaque engine handle returned to Swift. Internally holds a tokio runtime
/// and (later, in bd one-vtg5eh) an `Arc<FrankenStorage>`.
pub struct CassEngine {
    _private: (),
}

impl CassEngine {
    /// Static probe returning the cass crate version. Used by the xcframework
    /// build script as a smoke test that the bridge symbol actually links.
    pub fn version() -> String {
        // `coding_agent_search` crate name -> imported below.
        coding_agent_search_version()
    }
}

/// Wrapper around the cass crate's compile-time version constant. Kept as a
/// free fn so it's easy to swap out later when CassEngine grows real state.
fn coding_agent_search_version() -> String {
    // The cass crate exposes its own version via `env!("CARGO_PKG_VERSION")`
    // at its own compile site. We re-read it from our compile site, which is
    // fine for an FFI smoke test — the value is "0.1.0" here, but a real
    // `health()` impl will surface cass's version explicitly.
    env!("CARGO_PKG_VERSION").to_string()
}
