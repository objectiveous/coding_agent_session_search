# cass-ffi surface (v1)

Source of truth for the Swift-facing FFI surface decided in bd
`one-jn0t4w`. As of swift-bridge 0.1.59 not every type below crosses
cleanly — see [swift-bridge 0.1.59 gaps](#swift-bridge-0159-gaps) and
[workaround pattern](#workaround-pattern). The natural surface is
documented here; the bridge module in `src/lib.rs` lands its
flattened workarounds incrementally as real methods are wired up in
bd `one-vtg5eh` (actor wrap).

---

## Functions (15)

### Engine lifecycle (2)

| # | Signature | Wraps |
|---|---|---|
| 1 | `open(data_dir: String) -> Result<CassEngine, CassError>` | `FrankenStorage::open` (src/storage/sqlite.rs:3395) |
| 2 | `close(self: &mut CassEngine) -> Result<(), CassError>` | `FrankenStorage::close` (sqlite.rs:3664) |

### Search (3)

| # | Signature | Wraps |
|---|---|---|
| 3 | `search(query: String, opts: SearchOpts) -> Result<Vec<SearchHit>, CassError>` | dispatches to `search` / `search_hybrid` / `search_semantic` (src/search/query.rs:3381/5455/5125) on `opts.mode` |
| 4 | `search_in_session(session_id: String, query: String) -> Result<Vec<SearchHit>, CassError>` | same as (3) with a SessionFilter pre-applied |
| 5 | `start_progressive_search(query: String, opts: SearchOpts) -> SearchHandle` | wraps `search_progressive_with_callback` (query.rs:4920); progress streamed via SearchHandle.events() |

### Conversation reads (3)

| # | Signature | Wraps |
|---|---|---|
| 6 | `load_conversation(source_path: String) -> Result<Option<Conversation>, CassError>` | `ui::data::load_conversation` (src/ui/data.rs:481) |
| 7 | `load_conversation_for_hit(hit: SearchHit) -> Result<Option<Conversation>, CassError>` | `ui::data::load_conversation_for_hit` (data.rs:669) |
| 8 | `expand_around(conversation_id: i64, message_idx: i64, before: u32, after: u32) -> Result<Vec<Message>, CassError>` | built atop `FrankenStorage::fetch_messages` (sqlite.rs:6885) with slice arithmetic; not currently a stand-alone fn in data.rs |

### Catalogues (3)

| # | Signature | Wraps |
|---|---|---|
| 9 | `list_sessions(filter: SessionFilter) -> Result<Vec<SessionSummary>, CassError>` | `FrankenStorage::list_conversations` (sqlite.rs:6428) with optional filtering, projected to SessionSummary (no message bodies) |
| 10 | `list_workspaces() -> Result<Vec<Workspace>, CassError>` | `list_workspaces` (sqlite.rs:6410) |
| 11 | `list_agents() -> Result<Vec<AgentSource>, CassError>` | `list_agents` (sqlite.rs:6290) joined with provenance `Source` (src/sources/provenance.rs:48) |

### Health & lifecycle (4)

| # | Signature | Wraps |
|---|---|---|
| 12 | `health() -> Result<HealthReport, CassError>` | synthesized from `daily_stats_health` (sqlite.rs:9696) + `get_last_indexed_at` (6267) + `total_conversation_count` (6314) + on-disk model file presence |
| 13 | `triage() -> Result<TriageReport, CassError>` | mirrors `cass triage --json`; includes next_command + recommended_commands |
| 14 | `start_install_model(name: String) -> ModelInstallHandle` | drives the same logic as `cass models install`; progress streamed via ModelInstallHandle.events() |
| 15 | `start_reindex(scope: ReindexScope) -> ReindexHandle` | drives the embedding-job queue (sqlite.rs:9431/9540) and `rebuild_fts` (sqlite.rs:8974) per scope; progress streamed |

---

## Opaque handles (4)

| Type | From | Lifecycle |
|---|---|---|
| `CassEngine` | `open()` | Owns Arc<FrankenStorage>; closed via `close()` |
| `SearchHandle` | `start_progressive_search()` | `events() -> AsyncStream<SearchEvent>`, `cancel()` |
| `ModelInstallHandle` | `start_install_model()` | `events() -> AsyncStream<ModelInstallEvent>`, `cancel()` |
| `ReindexHandle` | `start_reindex()` | `events() -> AsyncStream<ReindexEvent>`, `cancel()` |

Cancellation = dropping the handle on the Swift side. Each handle holds an `mpsc::UnboundedReceiver<Event>` whose sender lives in the worker tokio task; on drop the channel closes, the task notices at its next yield point and tears down gracefully.

---

## Value types

```rust
// All field types are FFI-safe: no Arc, no parking_lot, no serde_json::Value.
// Translation from upstream cass types (which DO carry those) happens via
// From impls at the bridge edge.

struct SearchHit {
    title: String,
    snippet: String,
    content: String,
    score: f32,
    source_path: String,
    agent: String,
    workspace: String,
    workspace_original: Option<String>,
    created_at_ms: Option<i64>,
    line_number: Option<u64>,
    match_type: String,
    source_id: String,
    origin_kind: String,
    origin_host: Option<String>,
}

struct Conversation {
    id: Option<i64>,
    agent_slug: String,
    workspace_path: Option<String>,
    external_id: Option<String>,
    title: Option<String>,
    source_path: String,
    started_at_ms: Option<i64>,
    ended_at_ms: Option<i64>,
    approx_tokens: Option<i64>,
    messages: Vec<Message>,
    source_id: String,
    origin_host: Option<String>,
}

struct Message {
    id: Option<i64>,
    idx: i64,
    role: MessageRole,
    author: Option<String>,
    created_at_ms: Option<i64>,
    content: String,
}

enum MessageRole {
    User,
    Agent,
    Tool,
    System,
    Other(String),
}

struct SessionSummary {
    id: i64,
    source_id: String,
    agent_slug: String,
    workspace_path: Option<String>,
    title: Option<String>,
    started_at_ms: Option<i64>,
    ended_at_ms: Option<i64>,
    message_count: i64,
    source_path: String,
}

struct Workspace {
    id: Option<i64>,
    path: String,
    display_name: Option<String>,
}

struct AgentSource {
    agent_id: Option<i64>,
    agent_slug: String,
    agent_name: String,
    agent_version: Option<String>,
    source_id: String,
    host_label: Option<String>,
    platform: Option<String>,
}

struct HealthReport {
    last_indexed_at_ms: Option<i64>,
    total_conversations: u64,
    total_messages: u64,
    fallback_mode: String,   // one of "lexical", "hybrid", "unavailable"
    model_installed: Option<String>,
    fts_ready: bool,
    semantic_ready: bool,
}

struct TriageReport {
    readiness: String,
    next_command: Option<String>,
    recommended_commands: Vec<String>,
    notes: Vec<String>,
}

struct TimeFilter {
    after_ms: Option<i64>,
    before_ms: Option<i64>,
}

struct SessionFilter {
    workspace_path: Option<String>,
    agent_slug: Option<String>,
    time_filter: Option<TimeFilter>,
    limit: u32,
    offset: u32,
}

enum SearchOpts {
    Hybrid   { limit: u32, time_filter: Option<TimeFilter> },
    Lexical  { limit: u32 },
    Semantic { limit: u32, model: Option<String> },
}

enum ReindexScope {
    All,
    SemanticOnly,
    LexicalOnly,
    SinceTimestamp(i64),
}

struct CassError {
    code: i32,
    kind: String,       // stable branch point; kebab-case from cli_error_kind.rs
    message: String,
    hint: Option<String>,
    retryable: bool,
}

enum SearchEvent {
    Started,
    Partial(Vec<SearchHit>),
    Complete(Vec<SearchHit>),
    Failed(CassError),
}

enum ModelInstallEvent {
    Connecting,
    Downloading { bytes_done: u64, bytes_total: u64 },
    Verifying,
    Installing,
    Ready,
    Failed(CassError),
}

enum ReindexEvent {
    Scanning,
    Processing { done: u64, total: u64 },
    Phase(String),
    Complete,
    Failed(CassError),
}
```

---

## swift-bridge 0.1.59 gaps

Empirically discovered while wiring the bridge:

1. **`Result<T, SharedStruct>`** panics in `BuiltInResult::custom_c_struct_name` reaching `to_alpha_numeric_underscore_name` on the error type. Affects every fallible method in the surface above.
2. **`Option<SharedStruct>` as a struct field** panics in `bridged_option.rs:399`. Affects `time_filter: Option<TimeFilter>` inside `SearchOpts` and `SessionFilter`.
3. **Enums with `Vec<SharedStruct>` or struct payload variants** (e.g. `SearchEvent::Partial(Vec<SearchHit>)`) panic in `bridged_type.rs:1986`. Affects every `*Event` enum.
4. **Doc comments (`/// ...`) on shared structs** are rejected outright by `parse_struct.rs:143` ("unsupported attribute `doc`").

## Workaround pattern

For each affected shape, the bridge in `lib.rs` lands a flattened form:

- `Result<T, CassError>` → envelope struct `{ value: T, error_kind: String, error_message: String, error_code: i32, error_hint: String, error_retryable: bool }`. Empty `error_kind` = success.
- `Option<SharedStruct>` field → either flatten the inner struct's fields up one level, or replace with a "present" sentinel bool + an always-present struct whose fields can express "absent" themselves (e.g. `TimeFilter { after_ms: None, before_ms: None }` means "no filter").
- `enum Foo { A(X), B(Y) }` with complex payload → flatten to a struct `{ kind: FooKind, x_payload: X, y_payload: Y }`. Swift callers branch on `.kind`.
- Doc comments → use `//` line comments above each declaration.

The Swift consumer wraps these flattened envelopes back into native Swift enums / Results at the call site (one small generated wrapper layer in OneApp's Swift code).

Upstream-fix candidates (any of these obsoletes the workaround):

- swift-bridge gains a Result-error-as-SharedStruct codegen path
- swift-bridge gains nested-SharedStruct-in-Option codegen
- swift-bridge gains arbitrary-payload enum variant codegen
- We fork swift-bridge and patch the three `unimplemented!()` panic sites

Tracked as a follow-up to the cass embedding epic (`one-2yofmw`).
