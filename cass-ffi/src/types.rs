//! UniFFI value types for the cass-ffi bridge.
//!
//! Kept in lockstep with `cass-ffi/SURFACE.md`. The structs use
//! `#[derive(uniffi::Record)]` for plain value types, enums use
//! `#[derive(uniffi::Enum)]`, and `CassError` is a UniFFI error enum
//! with a single fields-bearing variant so Swift sees individual
//! fields instead of just a message string.
//!
//! NB: SURFACE.md sketches `CassError` as a struct with `#[uniffi(flat_error)]`,
//! but UniFFI 0.31 only accepts the `Error` derive on enums. We model
//! the same fields on a single variant — the upgrade path to multiple
//! `kind`-named variants is straightforward when we want richer Swift
//! pattern-matching ergonomics.
//!
//! Conversion from cass-internal types lives next to each impl in
//! `lib.rs` (see `From<cass_query::SearchHit> for SearchHit`).

use thiserror::Error;

/// A single search hit projected to FFI-safe types. The shape mirrors
/// cass-internal `SearchHit` (src/search/query.rs:1231) but flattens the
/// `MatchType` enum into a snake_case string and casts `usize` →
/// `Option<u64>` (`line_number`) so UniFFI can emit a stable Swift type
/// across 32/64-bit targets.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SearchHit {
    pub title: String,
    pub snippet: String,
    pub content: String,
    pub score: f32,
    pub source_path: String,
    pub agent: String,
    pub workspace: String,
    pub workspace_original: Option<String>,
    pub created_at_ms: Option<i64>,
    pub line_number: Option<u64>,
    pub match_type: String,
    pub source_id: String,
    pub origin_kind: String,
    pub origin_host: Option<String>,
}

/// Role of a single message in a transcript.
///
/// Maps cass-internal `crate::model::types::MessageRole` to a UniFFI enum.
/// The tuple variant `Other(String)` becomes a struct variant
/// `Other { label }` per SURFACE.md to keep Swift's `case .other(label:)`
/// labelling consistent.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum MessageRole {
    User,
    Agent,
    Tool,
    System,
    Other { label: String },
}

/// A single message inside a conversation. Mirrors cass-internal
/// `crate::model::types::Message` minus the runtime-only `extra_json` /
/// `snippets` fields, which UniFFI cannot represent without a richer
/// JSON value type. Those are owned by curation flows that live above
/// this surface.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Message {
    pub id: Option<i64>,
    pub idx: i64,
    pub role: MessageRole,
    pub author: Option<String>,
    pub created_at_ms: Option<i64>,
    pub content: String,
}

/// A conversation projected to FFI-safe types. `messages` is the full
/// message list as cass returns it; for paginated reads use
/// `expand_around` with a (conversation_id, message_idx) anchor.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Conversation {
    pub id: Option<i64>,
    pub agent_slug: String,
    pub workspace_path: Option<String>,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub source_path: String,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub approx_tokens: Option<i64>,
    pub messages: Vec<Message>,
    pub source_id: String,
    pub origin_host: Option<String>,
}

/// A workspace cass has indexed at least one conversation under.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Workspace {
    pub id: Option<i64>,
    pub path: String,
    pub display_name: Option<String>,
}

/// Time window filter shared by `SearchOpts` and `SessionFilter`.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct TimeFilter {
    pub after_ms: Option<i64>,
    pub before_ms: Option<i64>,
}

/// Polled snapshot of an in-flight (or finished) indexing run.
///
/// cass's progress model is poll-based: an `Arc<IndexingProgress>` shared
/// with the indexer thread carries atomic counters + mutex-wrapped status
/// strings. `IndexRun.snapshot()` projects that live state into a value
/// type the Swift side can store, render, and pass around without holding
/// any locks across `await` boundaries.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IndexProgressSnapshot {
    /// Coarse phase: "idle", "scanning", "indexing".
    pub phase: String,
    /// Sessions processed so far during the active or just-completed run.
    pub current: u64,
    /// Best-known total session count for the run (may grow as the
    /// scanner discovers more sources).
    pub total: u64,
    /// Agent slugs cass has discovered transcripts for during this run.
    pub discovered_agents: Vec<String>,
    /// Last non-fatal error message the indexer recorded, if any.
    pub last_error: Option<String>,
}

/// Search-mode discriminator. The actor wrap dispatches on the variant
/// to the corresponding cass `SearchClient` method (lexical → `search`
/// with default mode, semantic/hybrid → `search_semantic`/`search_hybrid`
/// — wired incrementally as the next beads land).
#[derive(Debug, Clone, uniffi::Enum)]
pub enum SearchOpts {
    Hybrid {
        limit: u32,
        time_filter: Option<TimeFilter>,
    },
    Lexical {
        limit: u32,
    },
    Semantic {
        limit: u32,
        model: Option<String>,
    },
}

/// Stable error envelope across the FFI boundary. `kind` is the branch
/// point — kebab-case strings match cass's `cli_error_kind::ErrorKind`
/// where applicable, plus a small set of FFI-local kinds:
/// `engine-closed`, `not-yet-implemented`, `internal`, `cass-error`,
/// `data-dir-missing-db`, `data-dir-missing-index`.
///
/// Single-variant enum: keeps the natural fields (code/kind/message/hint/
/// retryable) on Swift's side as a struct-shaped associated payload while
/// satisfying UniFFI's enum-only error constraint.
#[derive(Debug, Clone, Error, uniffi::Error)]
pub enum CassError {
    #[error("{kind}: {message}")]
    Failed {
        code: i32,
        kind: String,
        message: String,
        hint: Option<String>,
        retryable: bool,
    },
}

impl CassError {
    pub(crate) fn engine_closed() -> Self {
        CassError::Failed {
            code: 2,
            kind: "engine-closed".to_string(),
            message: "CassEngine has been closed; create a new one to continue.".to_string(),
            hint: None,
            retryable: false,
        }
    }

    pub(crate) fn unimplemented(what: &str) -> Self {
        CassError::Failed {
            code: 1,
            kind: "not-yet-implemented".to_string(),
            message: format!("cass-ffi: {what} not yet wired through the FFI"),
            hint: Some(
                "Surface defined in cass-ffi/SURFACE.md; impls land incrementally on bd one-tybc4w."
                    .to_string(),
            ),
            retryable: false,
        }
    }

    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        CassError::Failed {
            code: 3,
            kind: "internal".to_string(),
            message: msg.into(),
            hint: None,
            retryable: false,
        }
    }

    pub(crate) fn data_dir_missing_db(db_path: &std::path::Path, data_dir: &std::path::Path) -> Self {
        CassError::Failed {
            code: 5,
            kind: "data-dir-missing-db".to_string(),
            message: format!(
                "expected cass SQLite store at {} (run `cass index` first)",
                db_path.display()
            ),
            hint: Some(format!(
                "data_dir = {}; cass writes its DB at <data_dir>/agent_search.db",
                data_dir.display()
            )),
            retryable: false,
        }
    }

    pub(crate) fn worker_died(what: &str) -> Self {
        CassError::Failed {
            code: 7,
            kind: "storage-worker-died".to_string(),
            message: format!(
                "cass-ffi storage worker thread no longer running ({what}); engine is unusable until reopened."
            ),
            hint: Some(
                "drop and reconstruct the CassEngine; this typically indicates a bug in the worker loop.".to_string(),
            ),
            retryable: false,
        }
    }

    pub(crate) fn data_dir_missing_index(index_path: &std::path::Path, db_path: &std::path::Path) -> Self {
        CassError::Failed {
            code: 6,
            kind: "data-dir-missing-index".to_string(),
            message: format!(
                "neither Tantivy index ({}) nor SQLite fallback at {} produced an open SearchClient",
                index_path.display(),
                db_path.display()
            ),
            hint: Some("run `cass index --full` to populate the lexical index.".to_string()),
            retryable: false,
        }
    }

    /// Convenience accessor — most call sites only branch on the kind.
    pub fn kind(&self) -> &str {
        match self {
            CassError::Failed { kind, .. } => kind,
        }
    }
}

impl From<anyhow::Error> for CassError {
    fn from(err: anyhow::Error) -> Self {
        CassError::Failed {
            code: 4,
            kind: "cass-error".to_string(),
            message: format!("{err:#}"),
            hint: None,
            retryable: false,
        }
    }
}
