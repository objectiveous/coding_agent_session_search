# cass-ffi surface (v1)

Source of truth for the Swift-facing FFI surface decided in bd `one-jn0t4w`.
Implemented on UniFFI 0.31 (proc-macro mode). UniFFI natively handles every
type below — no flat-envelope workarounds, no wrapping layers on the Swift
side. Methods declared with `#[uniffi::export]`; types with `#[derive(uniffi::Object)]`
(opaque handles) or `#[derive(uniffi::Record)]` (value-type structs).

The bridge module in `src/lib.rs` lands methods incrementally as real impls
arrive in bd `one-vtg5eh` (actor wrap). Each new method ships against the
natural shape documented here.

---

## Functions (15)

### Engine lifecycle (2)

| # | Signature | Wraps |
|---|---|---|
| 1 | `CassEngine::open(data_dir: String) -> Result<Arc<CassEngine>, CassError>` (UniFFI `#[uniffi::constructor]`) | `FrankenStorage::open` (src/storage/sqlite.rs:3395) |
| 2 | `close(&self) -> Result<(), CassError>` | `FrankenStorage::close` (sqlite.rs:3664) |

### Search (3)

| # | Signature | Wraps |
|---|---|---|
| 3 | `async fn search(&self, query: String, opts: SearchOpts) -> Result<Vec<SearchHit>, CassError>` | dispatches to `search` / `search_hybrid` / `search_semantic` (src/search/query.rs:3381/5455/5125) on `opts.mode` |
| 4 | `async fn search_in_session(&self, session_id: String, query: String) -> Result<Vec<SearchHit>, CassError>` | same as (3) with a SessionFilter pre-applied |
| 5 | `fn start_progressive_search(&self, query: String, opts: SearchOpts) -> Arc<SearchHandle>` | wraps `search_progressive_with_callback` (query.rs:4920); progress streamed via `SearchHandle.next_event().await` |

### Conversation reads (3)

| # | Signature | Wraps |
|---|---|---|
| 6 | `async fn load_conversation(&self, source_path: String) -> Result<Option<Conversation>, CassError>` | `ui::data::load_conversation` (src/ui/data.rs:481) |
| 7 | `async fn load_conversation_for_hit(&self, hit: SearchHit) -> Result<Option<Conversation>, CassError>` | `ui::data::load_conversation_for_hit` (data.rs:669) |
| 8 | `async fn expand_around(&self, conversation_id: i64, message_idx: i64, before: u32, after: u32) -> Result<Vec<Message>, CassError>` | built atop `FrankenStorage::fetch_messages` (sqlite.rs:6885) |

### Catalogues (3)

| # | Signature | Wraps |
|---|---|---|
| 9 | `async fn list_sessions(&self, filter: SessionFilter) -> Result<Vec<SessionSummary>, CassError>` | `FrankenStorage::list_conversations` (sqlite.rs:6428), projected to SessionSummary (no message bodies) |
| 10 | `async fn list_workspaces(&self) -> Result<Vec<Workspace>, CassError>` | `list_workspaces` (sqlite.rs:6410) |
| 11 | `async fn list_agents(&self) -> Result<Vec<AgentSource>, CassError>` | `list_agents` (sqlite.rs:6290) joined with provenance `Source` (src/sources/provenance.rs:48) |

### Health & lifecycle (4)

| # | Signature | Wraps |
|---|---|---|
| 12 | `async fn health(&self) -> Result<HealthReport, CassError>` | synthesized from `daily_stats_health` + `get_last_indexed_at` + `total_conversation_count` + on-disk model file presence |
| 13 | `async fn triage(&self) -> Result<TriageReport, CassError>` | mirrors `cass triage --json` |
| 14 | `fn start_install_model(&self, name: String) -> Arc<ModelInstallHandle>` | drives the same logic as `cass models install`; progress streamed |
| 15 | `fn start_reindex(&self, scope: ReindexScope) -> Arc<ReindexHandle>` | drives embedding-job queue (sqlite.rs:9431/9540) + `rebuild_fts` (sqlite.rs:8974) per scope |

---

## Opaque handles (4) — `#[derive(uniffi::Object)]`

| Type | From | Lifecycle |
|---|---|---|
| `CassEngine` | `CassEngine::open()` | Owns Arc<FrankenStorage>; closed via `close()` |
| `SearchHandle` | `start_progressive_search()` | `next_event() -> Option<SearchEvent>`, `cancel()` |
| `ModelInstallHandle` | `start_install_model()` | `next_event() -> Option<ModelInstallEvent>`, `cancel()` |
| `ReindexHandle` | `start_reindex()` | `next_event() -> Option<ReindexEvent>`, `cancel()` |

Cancellation: dropping the Arc on the Swift side triggers Rust-side teardown via a `CancellationToken` held by the worker tokio task. swift consumers wrap `next_event` in an `AsyncStream` for `for await event in handle.events()` ergonomics.

---

## Value types — `#[derive(uniffi::Record)]` (structs) / `#[derive(uniffi::Enum)]` (enums)

```rust
#[derive(uniffi::Record)]
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

#[derive(uniffi::Record)]
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

#[derive(uniffi::Record)]
pub struct Message {
    pub id: Option<i64>,
    pub idx: i64,
    pub role: MessageRole,
    pub author: Option<String>,
    pub created_at_ms: Option<i64>,
    pub content: String,
}

#[derive(uniffi::Enum)]
pub enum MessageRole {
    User,
    Agent,
    Tool,
    System,
    Other { label: String },   // named-field variant; UniFFI supports
}

#[derive(uniffi::Record)]
pub struct SessionSummary {
    pub id: i64,
    pub source_id: String,
    pub agent_slug: String,
    pub workspace_path: Option<String>,
    pub title: Option<String>,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub message_count: i64,
    pub source_path: String,
}

#[derive(uniffi::Record)]
pub struct Workspace {
    pub id: Option<i64>,
    pub path: String,
    pub display_name: Option<String>,
}

#[derive(uniffi::Record)]
pub struct AgentSource {
    pub agent_id: Option<i64>,
    pub agent_slug: String,
    pub agent_name: String,
    pub agent_version: Option<String>,
    pub source_id: String,
    pub host_label: Option<String>,
    pub platform: Option<String>,
}

#[derive(uniffi::Record)]
pub struct HealthReport {
    pub last_indexed_at_ms: Option<i64>,
    pub total_conversations: u64,
    pub total_messages: u64,
    pub fallback_mode: String,   // "lexical" | "hybrid" | "unavailable"
    pub model_installed: Option<String>,
    pub fts_ready: bool,
    pub semantic_ready: bool,
}

#[derive(uniffi::Record)]
pub struct TriageReport {
    pub readiness: String,
    pub next_command: Option<String>,
    pub recommended_commands: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct TimeFilter {
    pub after_ms: Option<i64>,
    pub before_ms: Option<i64>,
}

#[derive(uniffi::Record)]
pub struct SessionFilter {
    pub workspace_path: Option<String>,
    pub agent_slug: Option<String>,
    pub time_filter: Option<TimeFilter>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(uniffi::Enum)]
pub enum SearchOpts {
    Hybrid   { limit: u32, time_filter: Option<TimeFilter> },
    Lexical  { limit: u32 },
    Semantic { limit: u32, model: Option<String> },
}

#[derive(uniffi::Enum)]
pub enum ReindexScope {
    All,
    SemanticOnly,
    LexicalOnly,
    SinceTimestamp { ms: i64 },
}

// CassError is an *interface-style* UniFFI error so Swift sees fields,
// not just a message string. `kind` is the stable branch point;
// kebab-case from cass's src/model/cli_error_kind.rs (golden-tested).
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
#[error("{kind}: {message}")]
pub struct CassError {
    pub code: i32,
    pub kind: String,
    pub message: String,
    pub hint: Option<String>,
    pub retryable: bool,
}

#[derive(uniffi::Enum)]
pub enum SearchEvent {
    Started,
    Partial { hits: Vec<SearchHit> },
    Complete { hits: Vec<SearchHit> },
    Failed { error: CassError },
}

#[derive(uniffi::Enum)]
pub enum ModelInstallEvent {
    Connecting,
    Downloading { bytes_done: u64, bytes_total: u64 },
    Verifying,
    Installing,
    Ready,
    Failed { error: CassError },
}

#[derive(uniffi::Enum)]
pub enum ReindexEvent {
    Scanning,
    Processing { done: u64, total: u64 },
    Phase { name: String },
    Complete,
    Failed { error: CassError },
}
```

---

## What UniFFI gives us across the boundary

- `Result<T, CassError>` → Swift `throws` method. Call site: `let hits = try await engine.search(query: q, opts: opts)`. Catch `CassError` with full field access (`kind`, `message`, `hint`, etc.).
- `Option<T>` → Swift `T?`. Including `Option<Conversation>` for "not found" — no sentinel patterns needed.
- `Vec<T>` → Swift `[T]`. Including `Vec<SearchHit>` in struct fields and as method returns.
- Enums with associated-data variants → Swift `enum SearchEvent { case partial(hits: [SearchHit]); case complete(hits: [SearchHit]); case failed(error: CassError) }`. Pattern match directly.
- `async fn` → Swift `async throws` methods. Tokio runtime threading happens internally; Swift just `await`s.
- Doc comments on every type and method propagate as Swift doc comments visible in Xcode Quick Help.

---

## Build pipeline

```bash
# Inside cass-ffi/, with rustup nightly active:

# 1. Compile staticlib + cdylib + bindgen binary
cargo build --release

# 2. Generate Swift bindings against the cdylib
./target/release/uniffi-bindgen generate \
    --library target/release/libcass_ffi.dylib \
    --language swift \
    --out-dir generated/

# Outputs:
#   generated/cass_ffi.swift            -- Swift module (CassEngine, all types)
#   generated/cass_ffiFFI.h             -- C header
#   generated/cass_ffiFFI.modulemap     -- Clang module map
#
# The xcframework script (bd one-4di061) bundles:
#   - target/release/libcass_ffi.a        as the static archive
#   - generated/cass_ffi.swift            as the Swift source for the framework Modules/
#   - generated/cass_ffiFFI.h + .modulemap as the C side
```

For consumers (the Swift target inside OneApp / a CassSearch plugin):

```swift
import CassFFI   // The xcframework's module name

// async throws is real:
let engine = try await CassEngine.open(dataDir: cassDir)
let hits = try await engine.search(query: "gist meetings",
                                    opts: .hybrid(limit: 20, timeFilter: nil))
for hit in hits {
    print(hit.sessionId, hit.score, hit.snippet)
}

// Pattern matching works:
let stream = AsyncStream { continuation in
    Task {
        let handle = engine.startProgressiveSearch(query: q, opts: opts)
        while let event = await handle.nextEvent() {
            continuation.yield(event)
            if case .complete = event { break }
        }
        continuation.finish()
    }
}
for await event in stream {
    switch event {
    case .started: print("starting")
    case .partial(let hits): updateUI(hits)
    case .complete(let hits): finalizeUI(hits)
    case .failed(let error): show(error)
    }
}
```
