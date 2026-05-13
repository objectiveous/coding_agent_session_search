//! Internal value-type helpers. The full FFI value-type surface is
//! documented in `cass-ffi/SURFACE.md`; the bridge module in `lib.rs`
//! lands these incrementally as bd `one-vtg5eh` wires up real methods.
//!
//! For now only `CassError` lives here — used by future bridge-method
//! impls to construct error envelopes at the FFI boundary.

#[derive(Debug, Clone)]
pub struct CassError {
    pub code: i32,
    pub kind: String,
    pub message: String,
    pub hint: Option<String>,
    pub retryable: bool,
}

impl CassError {
    pub fn unimplemented(what: &str) -> Self {
        Self {
            code: 1,
            kind: "not-yet-implemented".to_string(),
            message: format!("cass-ffi: {what} not yet wired through the FFI"),
            hint: Some(
                "Surface defined in cass-ffi/SURFACE.md; impls land in bd one-vtg5eh."
                    .to_string(),
            ),
            retryable: false,
        }
    }
}
