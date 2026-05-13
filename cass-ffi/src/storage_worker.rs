//! Dedicated worker-thread actor that owns the cass `FrankenStorage`
//! handle. `FrankenStorage` is `!Send` because its frankensqlite
//! `Connection` carries `Rc<RefCell<…>>` internals — it cannot live
//! inside a UniFFI `Object` (which requires `Send + Sync` via
//! `FfiConverterArc<UT>`). The actor pattern pins the storage to one
//! OS thread and routes async `CassEngine` method calls through an
//! mpsc command channel + per-call oneshot reply.
//!
//! This implements the dedicated-actor half of the concurrency
//! decision in bd `one-lf8lu8`. The other half — `tokio::spawn_blocking`
//! for short search calls — lives next to `CassEngine::search` in
//! `lib.rs`. The two halves coexist: search goes through the engine's
//! `Runtime`'s blocking pool; storage methods go through this worker.

use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::thread;

use coding_agent_search::storage::sqlite::FrankenStorage;
use coding_agent_search::ui::data as cass_ui_data;
use tokio::sync::{mpsc, oneshot};

use crate::types::{CassError, Conversation, Message, Workspace};

/// Bounded capacity for the command channel. Mostly arbitrary — the
/// worker drains the queue as fast as cass calls return; backpressure
/// here just bounds memory under sustained burst load. 32 is plenty
/// for interactive UI use.
const CMD_CHANNEL_CAPACITY: usize = 32;

/// Commands routed to the storage worker. Each carries its own
/// `oneshot::Sender` for the reply, typed to the natural FFI return.
/// New variants land here as additional FrankenStorage-backed methods
/// wire up under bd `one-tybc4w`.
pub(crate) enum StorageCmd {
    ListWorkspaces {
        reply: oneshot::Sender<Result<Vec<Workspace>, CassError>>,
    },
    LoadConversation {
        source_path: String,
        reply: oneshot::Sender<Result<Option<Conversation>, CassError>>,
    },
    ExpandAround {
        conversation_id: i64,
        message_idx: i64,
        before: u32,
        after: u32,
        reply: oneshot::Sender<Result<Vec<Message>, CassError>>,
    },
}

/// Owns the worker's lifecycle: holds the original mpsc Sender plus
/// the `JoinHandle` so Drop tears the worker down cleanly. Engine
/// async methods don't go through this directly — they grab a cheap
/// `StorageHandle` via `handle()` and use that, so the worker mutex
/// isn't held across awaits.
pub(crate) struct StorageWorker {
    tx: mpsc::Sender<StorageCmd>,
    /// Joined on `Drop` so the worker thread exits before the engine
    /// tears down. Not exposed.
    _join: Option<thread::JoinHandle<()>>,
}

/// Cheap, cloneable engine-side handle. Holds an `mpsc::Sender` clone;
/// dropping a handle does not shut the worker down — only dropping
/// the parent `StorageWorker` does.
#[derive(Clone)]
pub(crate) struct StorageHandle {
    tx: mpsc::Sender<StorageCmd>,
}

impl StorageWorker {
    /// Spawn the worker thread and open `FrankenStorage` inside it.
    /// Synchronous on the calling thread until init completes — the
    /// worker reports either `Ok(())` or the open error back via a
    /// std `mpsc` channel before this function returns.
    pub(crate) fn spawn(db_path: PathBuf) -> Result<Self, CassError> {
        let (init_tx, init_rx) = std_mpsc::channel::<Result<(), CassError>>();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<StorageCmd>(CMD_CHANNEL_CAPACITY);

        let join = thread::Builder::new()
            .name("cass-ffi-storage".to_string())
            .spawn(move || {
                // FrankenStorage::open is sync and !Send; opening it on
                // this dedicated thread keeps the `Rc<RefCell<…>>` interior
                // pinned here for the lifetime of the worker.
                let storage = match FrankenStorage::open(&db_path) {
                    Ok(s) => {
                        let _ = init_tx.send(Ok(()));
                        s
                    }
                    Err(err) => {
                        let _ = init_tx.send(Err(CassError::from(err)));
                        return;
                    }
                };

                // Current-thread tokio runtime so we can `await` the mpsc
                // recv without spawning anything else on this thread.
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(r) => r,
                    Err(err) => {
                        // We already signalled init success; the engine
                        // will see worker death on the next `send`.
                        tracing::error!(error = %err, "cass-ffi worker tokio runtime build failed");
                        return;
                    }
                };

                rt.block_on(async {
                    while let Some(cmd) = cmd_rx.recv().await {
                        handle_command(cmd, &storage);
                    }
                });

                // Shutdown: explicit close runs the SQLite checkpoint
                // (FrankenStorage::close consumes self).
                if let Err(err) = storage.close() {
                    tracing::warn!(error = %err, "cass-ffi storage close failed at worker shutdown");
                }
            })
            .map_err(|err| CassError::internal(format!("spawn storage worker thread: {err}")))?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx: cmd_tx,
                _join: Some(join),
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(CassError::internal(
                "storage worker thread died before signalling init result".to_string(),
            )),
        }
    }

    /// Cheap clone of the channel sender. Engine methods grab one of
    /// these out of the storage mutex, then drop the guard before
    /// awaiting on the oneshot reply.
    pub(crate) fn handle(&self) -> StorageHandle {
        StorageHandle {
            tx: self.tx.clone(),
        }
    }
}

impl StorageHandle {
    /// Send a command and await its reply. Returns `engine-closed`
    /// when the worker has been shut down (`mpsc::Sender::send` errors
    /// because the receiver is dropped) or `worker-died` if the worker
    /// dropped the reply oneshot without replying.
    pub(crate) async fn send<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, CassError>>) -> StorageCmd,
    ) -> Result<T, CassError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = build(reply_tx);
        self.tx
            .send(cmd)
            .await
            .map_err(|_| CassError::engine_closed())?;
        match reply_rx.await {
            Ok(result) => result,
            Err(_) => Err(CassError::worker_died("dropped reply oneshot")),
        }
    }
}

fn handle_command(cmd: StorageCmd, storage: &FrankenStorage) {
    match cmd {
        StorageCmd::ListWorkspaces { reply } => {
            let result = storage
                .list_workspaces()
                .map(|ws| ws.into_iter().map(Workspace::from).collect())
                .map_err(CassError::from);
            let _ = reply.send(result);
        }
        StorageCmd::LoadConversation {
            source_path,
            reply,
        } => {
            let result = cass_ui_data::load_conversation(storage, &source_path)
                .map(|opt| opt.map(|view| Conversation::from_view(view)))
                .map_err(CassError::from);
            let _ = reply.send(result);
        }
        StorageCmd::ExpandAround {
            conversation_id,
            message_idx,
            before,
            after,
            reply,
        } => {
            let result = expand_around_inner(
                storage,
                conversation_id,
                message_idx,
                before,
                after,
            );
            let _ = reply.send(result);
        }
    }
}

fn expand_around_inner(
    storage: &FrankenStorage,
    conversation_id: i64,
    message_idx: i64,
    before: u32,
    after: u32,
) -> Result<Vec<Message>, CassError> {
    let messages = storage
        .fetch_messages(conversation_id)
        .map_err(CassError::from)?;
    if messages.is_empty() {
        return Ok(vec![]);
    }

    // Anchor index is matched against `Message::idx` (the per-conversation
    // ordinal cass writes during indexing), not the Vec's positional index.
    // Fall back to clamping if the anchor doesn't appear in the conversation
    // (e.g. caller passed a stale id) — slicing 0 messages on either side
    // is friendlier than an error here.
    let anchor_pos = messages
        .iter()
        .position(|m| m.idx == message_idx)
        .unwrap_or(0);

    let start = anchor_pos.saturating_sub(before as usize);
    let end = anchor_pos
        .saturating_add(after as usize)
        .saturating_add(1)
        .min(messages.len());

    Ok(messages[start..end]
        .iter()
        .cloned()
        .map(Message::from)
        .collect())
}

