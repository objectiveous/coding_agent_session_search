//! cass-ffi — Swift-facing FFI shim for the cass (coding-agent-search) library.
//!
//! Built on UniFFI 0.31's proc-macro mode (no UDL file). The full design —
//! 15-fn surface, 4 opaque handles, ~15 value types — is documented in
//! `cass-ffi/SURFACE.md`. We chose UniFFI over swift-bridge because UniFFI
//! natively supports `Result<T, CustomError>`, `Option<Struct>` as struct
//! fields, enums with associated-data variants, and async functions —
//! swift-bridge 0.1.59 panics in codegen for all four of those.
//!
//! Concurrency model (decided in bd one-lf8lu8): short cass calls dispatch
//! to `tokio::spawn_blocking` on an engine-owned multi-thread tokio
//! runtime. Long-running ops (progressive search, model install, reindex)
//! get per-handle actors with mpsc channels + `CancellationToken` — those
//! land alongside their methods in subsequent passes of bd one-tybc4w.
//!
//! Storage handle layering: `SearchClient` is `Send` (cass wraps its
//! frankensqlite `Connection` in `SendConnection` with an `unsafe impl Send`
//! and serializes access via an internal `Mutex`). `FrankenStorage` is
//! NOT `Send` — its `Connection` exposes the raw `Rc<RefCell<…>>` fields.
//! For now the engine holds only `SearchClient`, which is sufficient for
//! the search-shaped FFI surface (`search`, `search_in_session`,
//! `start_progressive_search`). The conversation-read methods
//! (`load_conversation`, `expand_around`, `list_sessions`,
//! `list_workspaces`, `list_agents`) need the dedicated worker-thread
//! actor variant of the model and land in a follow-up pass — see the
//! TODO at the bottom of this file.

mod types;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use coding_agent_search::run_search_lexical_self_heal;
use coding_agent_search::search::query::{
    FieldMask, MatchType, SearchClient, SearchFilters, SearchHit as CassSearchHit,
};
use coding_agent_search::search::tantivy::expected_index_dir;
use parking_lot::Mutex;
use tokio::runtime::Runtime;

pub use types::{CassError, SearchHit, SearchOpts, TimeFilter};

uniffi::setup_scaffolding!();

/// Concrete data-directory layout that cass uses on disk.
struct DataLayout {
    db_path: PathBuf,
    index_path: PathBuf,
}

impl DataLayout {
    fn for_dir(data_dir: &Path) -> Self {
        Self {
            db_path: data_dir.join("agent_search.db"),
            index_path: expected_index_dir(data_dir),
        }
    }
}

#[derive(uniffi::Object)]
pub struct CassEngine {
    runtime: Runtime,
    /// `None` once `close()` has run; subsequent method calls return a
    /// clean `engine-closed` error rather than leaking a panic across
    /// the FFI boundary.
    search: Mutex<Option<Arc<SearchClient>>>,
    data_dir: PathBuf,
}

#[uniffi::export(async_runtime = "tokio")]
impl CassEngine {
    /// Smoke-test constructor. Returns a non-functional engine carrying
    /// only the crate version string. Used by the xcframework build script
    /// to verify the bridge symbols actually link without going through
    /// the disk-touching `open` path.
    #[uniffi::constructor]
    pub fn probe() -> Arc<Self> {
        let runtime = Runtime::new().expect("tokio runtime");
        Arc::new(Self {
            runtime,
            search: Mutex::new(None),
            data_dir: PathBuf::new(),
        })
    }

    /// Returns the cass-ffi crate version. Verified to round-trip through
    /// the UniFFI scaffolding by `cargo run --bin uniffi-bindgen` +
    /// downstream Swift consumers.
    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Open a CassEngine against an existing cass data directory. The
    /// directory must contain `agent_search.db` (cass's canonical SQLite
    /// store) and the `index/<schema-version>/` Tantivy index that cass
    /// emits during `cass index`. The SearchClient is opened once; the
    /// engine holds it on a tokio multi-thread runtime so subsequent
    /// `search` calls dispatch via `spawn_blocking`.
    #[uniffi::constructor]
    pub fn open(data_dir: String) -> Result<Arc<Self>, CassError> {
        let data_dir = PathBuf::from(data_dir);
        let layout = DataLayout::for_dir(&data_dir);

        if !layout.db_path.exists() {
            return Err(CassError::data_dir_missing_db(&layout.db_path, &data_dir));
        }

        // Self-heal the lexical index before opening the SearchClient — same
        // prelude that cass's CLI runs before each search command. Without
        // this, a stale or partially-rebuilt Tantivy index returns zero hits
        // for queries whose canonical content lives in SQLite.
        run_search_lexical_self_heal(&data_dir).map_err(CassError::from)?;

        let search = SearchClient::open(&layout.index_path, Some(&layout.db_path))
            .map_err(CassError::from)?
            .ok_or_else(|| CassError::data_dir_missing_index(&layout.index_path, &layout.db_path))?;
        let runtime = Runtime::new().map_err(|err| CassError::internal(err.to_string()))?;

        Ok(Arc::new(Self {
            runtime,
            search: Mutex::new(Some(Arc::new(search))),
            data_dir,
        }))
    }

    /// Close the engine, dropping the Tantivy reader + SQLite handle.
    /// Idempotent: a second call is a no-op. Subsequent method calls
    /// return a clean `engine-closed` error.
    pub fn close(&self) -> Result<(), CassError> {
        let _ = self.search.lock().take();
        Ok(())
    }

    /// Lexical / hybrid / semantic search across the indexed corpus.
    /// Currently only `Lexical` is wired (the first method landing under
    /// bd one-tybc4w). The other variants return `not-yet-implemented`
    /// until their dispatch lands in subsequent passes.
    pub async fn search(
        &self,
        query: String,
        opts: SearchOpts,
    ) -> Result<Vec<SearchHit>, CassError> {
        let search = match self.search.lock().as_ref() {
            Some(client) => Arc::clone(client),
            None => return Err(CassError::engine_closed()),
        };

        match opts {
            SearchOpts::Lexical { limit } => {
                let limit = limit as usize;
                let join = self.runtime.spawn_blocking(move || {
                    search.search(
                        &query,
                        SearchFilters::default(),
                        limit,
                        0,
                        FieldMask::FULL,
                    )
                });
                let hits = join
                    .await
                    .map_err(|err| CassError::internal(format!("spawn_blocking join: {err}")))?
                    .map_err(CassError::from)?;
                Ok(hits.into_iter().map(SearchHit::from).collect())
            }
            SearchOpts::Hybrid { .. } => Err(CassError::unimplemented("SearchOpts::Hybrid")),
            SearchOpts::Semantic { .. } => Err(CassError::unimplemented("SearchOpts::Semantic")),
        }
    }

    /// Returns the on-disk path the engine was opened against. Useful for
    /// diagnostics and for the curation flows that need the bundle-scoped
    /// data dir without re-resolving it.
    pub fn data_dir(&self) -> String {
        self.data_dir.to_string_lossy().into_owned()
    }
}

impl From<CassSearchHit> for SearchHit {
    fn from(hit: CassSearchHit) -> Self {
        Self {
            title: hit.title,
            snippet: hit.snippet,
            content: hit.content,
            score: hit.score,
            source_path: hit.source_path,
            agent: hit.agent,
            workspace: hit.workspace,
            workspace_original: hit.workspace_original,
            created_at_ms: hit.created_at,
            line_number: hit.line_number.map(|n| n as u64),
            match_type: match_type_to_string(hit.match_type).to_string(),
            source_id: hit.source_id,
            origin_kind: hit.origin_kind,
            origin_host: hit.origin_host,
        }
    }
}

fn match_type_to_string(mt: MatchType) -> &'static str {
    match mt {
        MatchType::Exact => "exact",
        MatchType::Prefix => "prefix",
        MatchType::Suffix => "suffix",
        MatchType::Substring => "substring",
        MatchType::Wildcard => "wildcard",
        MatchType::ImplicitWildcard => "implicit_wildcard",
    }
}

// TODO(one-tybc4w follow-up): FrankenStorage-backed methods
// (`load_conversation`, `expand_around`, `list_sessions`, etc.) need a
// dedicated worker-thread actor — `FrankenStorage` is `!Send` because
// its `Connection` carries `Rc<RefCell<…>>` internals. The pattern: spawn
// one OS thread per engine that owns the storage, expose async methods
// that send command messages over a `tokio::sync::mpsc` channel and
// `await` a `tokio::sync::oneshot::Receiver` for the reply. That actor
// also gets the `tokio_util::sync::CancellationToken` that progressive-
// search / install / reindex handles use. Out of scope for this pass:
// the search-only surface is what the v1 Flashbacks slice (one-g9e)
// actually depends on.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_version_is_cargo_pkg_version() {
        assert_eq!(CassEngine::probe().version(), env!("CARGO_PKG_VERSION"));
    }
}
