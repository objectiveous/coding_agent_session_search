//! cass-ffi — Swift-facing FFI shim for the cass (coding-agent-search) library.
//!
//! Built on UniFFI 0.31's proc-macro mode (no UDL file). The full design —
//! 15-fn surface, 4 opaque handles, ~15 value types — is documented in
//! `cass-ffi/SURFACE.md`. We chose UniFFI over swift-bridge because UniFFI
//! natively supports `Result<T, CustomError>`, `Option<Struct>` as struct
//! fields, enums with associated-data variants, and async functions —
//! swift-bridge 0.1.59 panics in codegen for all four of those.
//!
//! Concurrency model (decided in bd one-lf8lu8): two halves.
//!
//! 1. Short search calls dispatch to `tokio::spawn_blocking` on the
//!    engine-owned multi-thread tokio runtime. `SearchClient` is `Send`
//!    (cass wraps its frankensqlite `Connection` in `SendConnection` with
//!    an `unsafe impl Send` and serialises access via an internal `Mutex`),
//!    so the closure can move an `Arc<SearchClient>` across the blocking
//!    pool freely.
//!
//! 2. Storage reads (conversation reads, listing) go through a dedicated
//!    worker-thread actor (`storage_worker::StorageWorker`). `FrankenStorage`
//!    is `!Send` because its `Connection` exposes raw `Rc<RefCell<…>>`
//!    fields — UniFFI's `FfiConverterArc<UT>: Send + Sync` means a bare
//!    `FrankenStorage` cannot live inside a `#[derive(uniffi::Object)]`.
//!    The actor pins the storage to one OS thread; engine async methods
//!    send command messages over a `tokio::sync::mpsc` channel and `await`
//!    a per-call `tokio::sync::oneshot::Receiver` for the reply.
//!
//! Long-running ops (progressive search, model install, reindex) get
//! per-handle actors with their own `CancellationToken` — those land
//! alongside their methods in subsequent passes.

mod index_run;
mod storage_worker;
mod types;

pub use index_run::IndexRun;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use coding_agent_search::indexer::{IndexOptions, IndexingProgress, run_index};
use coding_agent_search::model::types::{
    Conversation as CassConversation, Message as CassMessage, MessageRole as CassMessageRole,
    Workspace as CassWorkspace,
};
use coding_agent_search::run_search_lexical_self_heal;
use coding_agent_search::search::query::{
    FieldMask, MatchType, SearchClient, SearchClientOptions, SearchFilters,
    SearchHit as CassSearchHit,
};
use coding_agent_search::search::tantivy::expected_index_dir;
use coding_agent_search::ui::data::ConversationView;
use parking_lot::Mutex;
use tokio::runtime::Runtime;

pub use types::{
    CassError, Conversation, IndexProgressSnapshot, Message, MessageRole, SearchHit, SearchOpts,
    TimeFilter, Workspace,
};

use storage_worker::{StorageCmd, StorageHandle, StorageWorker};

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
    /// `None` once `close()` has run; subsequent search calls return a
    /// clean `engine-closed` error rather than panicking.
    search: Mutex<Option<Arc<SearchClient>>>,
    /// Worker-thread actor owning the `FrankenStorage`. `None` once
    /// `close()` has run; the worker thread joins on Drop.
    storage: Mutex<Option<StorageWorker>>,
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
            storage: Mutex::new(None),
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
    /// emits during `cass index`. Both halves of the concurrency model
    /// (search-side `Arc<SearchClient>` + storage-side worker thread)
    /// come up here; failures during either rollback the other.
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

        // Storage worker first: cheaper to roll back (drop the Sender,
        // worker exits) than to roll back the SearchClient + tokio runtime.
        let storage = StorageWorker::spawn(layout.db_path.clone())?;

        // Match the `cass search` CLI's reader contract (lib.rs run_cli_search):
        // snapshot reader, no on-search reload, no warm worker. The CLI exits
        // after one query and re-opens for the next; long-lived FFI consumers
        // (FlashbacksEngine) follow the same pattern by close+reopen-ing the
        // engine when they want to see content indexed since open. With
        // `enable_reload: true` (the previous default), every search
        // re-acquired tantivy's META_LOCK, and any background indexer that
        // ran `run_index` concurrently could rename the live index directory
        // out from under the reader — surfacing as
        // `Failed to acquire Lockfile: IoError(Os { code: 2, kind: NotFound })`
        // (the bare `From<LockError>` path in tantivy's reader/mod.rs:194).
        // Snapshot semantics avoid the race entirely.
        let search = SearchClient::open_with_options(
            &layout.index_path,
            Some(&layout.db_path),
            SearchClientOptions {
                enable_reload: false,
                enable_warm: false,
            },
        )
        .map_err(CassError::from)?
        .ok_or_else(|| CassError::data_dir_missing_index(&layout.index_path, &layout.db_path))?;
        let runtime = Runtime::new().map_err(|err| CassError::internal(err.to_string()))?;

        Ok(Arc::new(Self {
            runtime,
            search: Mutex::new(Some(Arc::new(search))),
            storage: Mutex::new(Some(storage)),
            data_dir,
        }))
    }

    /// Close the engine: drop the SearchClient (Tantivy reader + SQLite
    /// fallback), then drop the storage worker handle so its thread sees
    /// the channel close, runs `FrankenStorage::close` (SQLite checkpoint),
    /// and exits. Idempotent; subsequent method calls return a clean
    /// `engine-closed` error.
    pub fn close(&self) -> Result<(), CassError> {
        let _ = self.search.lock().take();
        let _ = self.storage.lock().take();
        Ok(())
    }

    /// Lexical / hybrid / semantic search across the indexed corpus.
    /// Currently only `Lexical` is wired; the other variants return
    /// `not-yet-implemented` until their dispatch lands in subsequent
    /// passes.
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

    /// List every workspace cass has indexed at least one conversation
    /// under. Routes through the storage worker actor.
    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>, CassError> {
        let storage = self.storage_handle()?;
        storage
            .send(|reply| StorageCmd::ListWorkspaces { reply })
            .await
    }

    /// Load a full conversation by its on-disk source path (the same
    /// `source_path` cass returns on each `SearchHit`). Returns `None`
    /// when no conversation matches that path. Routes through the
    /// storage worker actor.
    pub async fn load_conversation(
        &self,
        source_path: String,
    ) -> Result<Option<Conversation>, CassError> {
        let storage = self.storage_handle()?;
        storage
            .send(|reply| StorageCmd::LoadConversation {
                source_path,
                reply,
            })
            .await
    }

    /// Window of `before` messages preceding and `after` messages
    /// following the message at `(conversation_id, message_idx)`.
    /// Anchor not found falls back to the conversation start (returns
    /// the first `before+after+1` messages). Built atop
    /// `FrankenStorage::fetch_messages`; routes through the storage
    /// worker actor.
    pub async fn expand_around(
        &self,
        conversation_id: i64,
        message_idx: i64,
        before: u32,
        after: u32,
    ) -> Result<Vec<Message>, CassError> {
        let storage = self.storage_handle()?;
        storage
            .send(|reply| StorageCmd::ExpandAround {
                conversation_id,
                message_idx,
                before,
                after,
                reply,
            })
            .await
    }

    /// Returns the on-disk path the engine was opened against. Useful
    /// for diagnostics and for the curation flows that need the
    /// bundle-scoped data dir without re-resolving it.
    pub fn data_dir(&self) -> String {
        self.data_dir.to_string_lossy().into_owned()
    }

    /// Trigger a one-shot incremental indexing pass against this engine's
    /// data_dir. Useful when the caller already has an open engine and wants
    /// to catch up newly written sessions without paying the full archive scan
    /// used by first-launch bootstrap.
    pub async fn start_index(
        &self,
        force_rebuild: bool,
    ) -> Result<Arc<IndexRun>, CassError> {
        let data_dir = self.data_dir.clone();
        start_index_inner(data_dir, false, force_rebuild, Some(&self.runtime)).await
    }
}

/// Bootstrap-or-incremental indexing pass that does NOT require an open
/// `CassEngine`. The caller passes a `data_dir` (the path to where cass's
/// data lives or should live); cass creates `agent_search.db` and the
/// Tantivy index on first run.
///
/// cass's connectors auto-discover known agent directories
/// (`~/.codex/sessions`, `~/.claude/projects`, and the other agents
/// registered in `coding_agent_search::connectors::*`) — callers don't
/// enumerate paths themselves.
///
/// `force_rebuild=false` runs an incremental scan that skips already-
/// indexed sessions (cheap on subsequent launches). `true` forces a
/// from-scratch rebuild.
///
/// Returns an `IndexRun` handle the caller polls via `snapshot()` for
/// UI updates or awaits via `wait_for_completion()`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn start_index_at_path(
    data_dir: String,
    force_rebuild: bool,
) -> Result<Arc<IndexRun>, CassError> {
    start_index_inner(PathBuf::from(data_dir), true, force_rebuild, None).await
}

/// Shared implementation for the engine-bound `start_index` and the
/// free-standing `start_index_at_path`. When `runtime_override` is
/// `Some`, dispatches `spawn_blocking` through the engine's runtime
/// (so the engine's tokio reactor is the one driving the indexer);
/// otherwise uses the ambient UniFFI-managed tokio runtime via
/// `tokio::task::spawn_blocking`.
async fn start_index_inner(
    data_dir: PathBuf,
    full: bool,
    force_rebuild: bool,
    runtime_override: Option<&Runtime>,
) -> Result<Arc<IndexRun>, CassError> {
    let progress = Arc::new(IndexingProgress::default());
    let opts = index_options_for_ffi(data_dir, full, force_rebuild, Some(progress.clone()));

    let join = if let Some(runtime) = runtime_override {
        runtime.spawn_blocking(move || run_index(opts, None).map_err(CassError::from))
    } else {
        tokio::task::spawn_blocking(move || run_index(opts, None).map_err(CassError::from))
    };

    Ok(Arc::new(IndexRun {
        progress,
        join_handle: tokio::sync::Mutex::new(Some(join)),
    }))
}

fn index_options_for_ffi(
    data_dir: PathBuf,
    full: bool,
    force_rebuild: bool,
    progress: Option<Arc<IndexingProgress>>,
) -> IndexOptions {
    let db_path = data_dir.join("agent_search.db");
    IndexOptions {
        full,
        force_rebuild,
        watch: false,
        watch_once_paths: None,
        db_path,
        data_dir,
        // v1 is lexical-only — semantic indexing requires a separate
        // embedder install flow that lands later.
        semantic: false,
        build_hnsw: false,
        embedder: "hash".to_string(),
        progress,
        watch_interval_secs: 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_bound_index_uses_incremental_mode() {
        let opts = index_options_for_ffi(PathBuf::from("/tmp/cass-ffi-test"), false, false, None);
        assert!(
            !opts.full,
            "CassEngine.start_index must use the incremental indexer path"
        );
        assert!(
            !opts.force_rebuild,
            "default app-triggered indexing should not force a rebuild"
        );
    }

    #[test]
    fn path_bootstrap_index_uses_full_mode() {
        let opts = index_options_for_ffi(PathBuf::from("/tmp/cass-ffi-test"), true, false, None);
        assert!(
            opts.full,
            "start_index_at_path bootstraps missing data dirs with a full scan"
        );
    }

    #[test]
    fn probe_version_is_cargo_pkg_version() {
        assert_eq!(CassEngine::probe().version(), env!("CARGO_PKG_VERSION"));
    }
}

impl CassEngine {
    /// Snapshot the storage handle out of its mutex without holding the
    /// guard across an await. The clone here is on the channel sender,
    /// not the worker — cheap.
    fn storage_handle(&self) -> Result<StorageHandle, CassError> {
        match self.storage.lock().as_ref() {
            Some(worker) => Ok(worker.handle()),
            None => Err(CassError::engine_closed()),
        }
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

impl From<CassWorkspace> for Workspace {
    fn from(ws: CassWorkspace) -> Self {
        Self {
            id: ws.id,
            path: ws.path.to_string_lossy().into_owned(),
            display_name: ws.display_name,
        }
    }
}

impl From<CassMessageRole> for MessageRole {
    fn from(role: CassMessageRole) -> Self {
        match role {
            CassMessageRole::User => MessageRole::User,
            CassMessageRole::Agent => MessageRole::Agent,
            CassMessageRole::Tool => MessageRole::Tool,
            CassMessageRole::System => MessageRole::System,
            CassMessageRole::Other(label) => MessageRole::Other { label },
        }
    }
}

impl From<CassMessage> for Message {
    fn from(msg: CassMessage) -> Self {
        Self {
            id: msg.id,
            idx: msg.idx,
            role: MessageRole::from(msg.role),
            author: msg.author,
            created_at_ms: msg.created_at,
            content: msg.content,
        }
    }
}

impl Conversation {
    /// Project a cass `ConversationView` (convo + loaded messages +
    /// workspace metadata) into the FFI value type. Uses the View's
    /// own `messages` rather than `convo.messages` because the loader
    /// puts the resolved messages on the View; the inner `Conversation`
    /// may carry them empty.
    pub(crate) fn from_view(view: ConversationView) -> Self {
        let ConversationView { convo, messages, .. } = view;
        let CassConversation {
            id,
            agent_slug,
            workspace,
            external_id,
            title,
            source_path,
            started_at,
            ended_at,
            approx_tokens,
            source_id,
            origin_host,
            ..
        } = convo;

        Self {
            id,
            agent_slug,
            workspace_path: workspace.map(|p| p.to_string_lossy().into_owned()),
            external_id,
            title,
            source_path: source_path.to_string_lossy().into_owned(),
            started_at_ms: started_at,
            ended_at_ms: ended_at,
            approx_tokens,
            messages: messages.into_iter().map(Message::from).collect(),
            source_id,
            origin_host,
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
