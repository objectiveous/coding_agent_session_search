//! Live FFI round-trip tests for the cass-ffi bridge.
//!
//! No mocks: tests open a real `CassEngine` against a real fixture
//! (copied at test setup into a tempdir so the fixture itself stays
//! immutable). The fixture mirrors `tests/fixtures/search_demo_data/`
//! from upstream cass — a 6-message, 2-conversation SQLite DB plus a
//! Tantivy index under `index/v1/`.

use std::fs;
use std::path::Path;

use cass_ffi::{CassEngine, SearchOpts};

fn current_thread_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime")
}

fn copy_fixture_to(temp_dir: &Path) {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("search_demo_data");
    copy_dir_recursive(&fixture_root, temp_dir);
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("mkdir dst");
    for entry in fs::read_dir(src).expect("read_dir src") {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type().expect("file_type").is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy file");
        }
    }
}

#[test]
fn open_returns_a_working_engine() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned())
        .expect("opening the demo fixture should succeed");
    drop(engine);
}

#[test]
fn lexical_search_returns_at_least_one_hit_for_known_term() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned())
        .expect("opening the demo fixture should succeed");

    // The fixture's first conversation contains an `aider` system message.
    // A lexical search for that term must round-trip through the FFI boundary
    // and come back as a non-empty Vec<SearchHit>.
    let runtime = current_thread_runtime();
    let hits = runtime
        .block_on(engine.search("aider".to_string(), SearchOpts::Lexical { limit: 10 }))
        .expect("lexical search should succeed");

    assert!(
        !hits.is_empty(),
        "expected at least one hit for 'aider' in the demo fixture, got 0"
    );
    let first = &hits[0];
    assert!(
        first.content.to_lowercase().contains("aider"),
        "first hit's content should contain the query term; got: {:?}",
        first.content
    );
    assert!(
        !first.source_path.is_empty(),
        "every hit should carry a source_path"
    );
}

#[test]
fn close_makes_subsequent_calls_fail_cleanly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned()).expect("open");
    engine.close().expect("close");

    let runtime = current_thread_runtime();
    let err = runtime
        .block_on(engine.search("aider".to_string(), SearchOpts::Lexical { limit: 10 }))
        .expect_err("search after close must fail, not succeed");
    assert_eq!(
        err.kind(),
        "engine-closed",
        "expected kind 'engine-closed', got: {:?}",
        err
    );
}

#[test]
fn list_workspaces_returns_indexed_workspaces_via_storage_actor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned()).expect("open");
    let runtime = current_thread_runtime();
    let workspaces = runtime
        .block_on(engine.list_workspaces())
        .expect("list_workspaces should succeed");

    // The fixture indexes at least one workspace (the aider transcript dir).
    assert!(
        !workspaces.is_empty(),
        "expected at least one workspace from the demo fixture, got 0"
    );
    let first = &workspaces[0];
    assert!(
        !first.path.is_empty(),
        "every Workspace should carry a non-empty path"
    );
}

#[test]
fn load_conversation_round_trips_via_storage_actor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned()).expect("open");
    let runtime = current_thread_runtime();

    // Use a real source_path produced by search() so the test exercises the
    // search → load_conversation flow end-to-end (the actual Flashbacks v1 + Session Replay path).
    let hits = runtime
        .block_on(engine.search("aider".to_string(), SearchOpts::Lexical { limit: 1 }))
        .expect("search");
    assert!(!hits.is_empty(), "fixture must yield at least one hit");
    let source_path = hits[0].source_path.clone();

    let conv = runtime
        .block_on(engine.load_conversation(source_path.clone()))
        .expect("load_conversation should succeed")
        .expect("conversation should exist for the search hit's source_path");

    assert_eq!(conv.source_path, source_path);
    assert!(
        !conv.messages.is_empty(),
        "loaded conversation should carry its messages"
    );
    assert!(
        !conv.agent_slug.is_empty(),
        "conversation should carry an agent_slug"
    );
}

#[test]
fn expand_around_returns_a_window_of_messages_via_storage_actor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned()).expect("open");
    let runtime = current_thread_runtime();

    // Resolve a real conversation_id from a search hit.
    let hits = runtime
        .block_on(engine.search("aider".to_string(), SearchOpts::Lexical { limit: 1 }))
        .expect("search");
    let source_path = hits[0].source_path.clone();
    let conv = runtime
        .block_on(engine.load_conversation(source_path))
        .expect("load_conversation")
        .expect("conversation present");
    let conversation_id = conv.id.expect("conversation id should be set after a load");
    let anchor_idx = conv.messages[0].idx;

    let window = runtime
        .block_on(engine.expand_around(conversation_id, anchor_idx, 0, 1))
        .expect("expand_around");
    assert!(
        !window.is_empty(),
        "expand_around should return at least the anchor message"
    );
    assert_eq!(
        window[0].idx, anchor_idx,
        "anchor message should appear first in the window"
    );
}

#[test]
fn list_workspaces_after_close_returns_engine_closed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    copy_fixture_to(tmp.path());

    let engine = CassEngine::open(tmp.path().to_string_lossy().into_owned()).expect("open");
    engine.close().expect("close");

    let runtime = current_thread_runtime();
    let err = runtime
        .block_on(engine.list_workspaces())
        .expect_err("list_workspaces after close must fail");
    assert_eq!(err.kind(), "engine-closed", "got: {:?}", err);
}
