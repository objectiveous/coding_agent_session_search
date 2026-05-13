//! `IndexRun` — a handle to an in-flight (or just-finished) call into
//! cass's `indexer::run_index`. cass's progress model is poll-based: the
//! indexer thread shares an `Arc<IndexingProgress>` with atomic counters
//! + mutex-wrapped status strings. `snapshot()` projects that live state
//! into a value type the Swift side can store without holding locks.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use coding_agent_search::indexer::IndexingProgress;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::types::{CassError, IndexProgressSnapshot};

#[derive(uniffi::Object)]
pub struct IndexRun {
    /// Shared with the cass indexer thread; counters update live as the
    /// run progresses.
    pub(crate) progress: Arc<IndexingProgress>,

    /// `None` once `wait_for_completion` has consumed it. Mutex so the
    /// `await` happens outside any synchronous lock, satisfying
    /// `FfiConverterArc<UT>: Send + Sync` for the uniffi::Object.
    pub(crate) join_handle: Mutex<Option<JoinHandle<Result<(), CassError>>>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl IndexRun {
    /// Cheap, non-blocking snapshot of the live progress counters. Safe to
    /// call from a Swift polling loop at any rate (every few hundred ms is
    /// typical for the UI footer).
    pub fn snapshot(&self) -> IndexProgressSnapshot {
        // cass uses std::sync::Mutex (lock() returns Result with a
        // PoisonError on the failure path). Recover the inner value on
        // poison rather than panicking — snapshot() is best-effort
        // status reporting and a poisoned mutex shouldn't cascade into a
        // UI crash.
        let discovered_agents = self
            .progress
            .discovered_agent_names
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        let last_error = self
            .progress
            .last_error
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());

        IndexProgressSnapshot {
            phase: phase_label(self.progress.phase.load(Ordering::Relaxed)),
            current: self.progress.current.load(Ordering::Relaxed) as u64,
            total: self.progress.total.load(Ordering::Relaxed) as u64,
            discovered_agents,
            last_error,
        }
    }

    /// Awaits the cass run_index task. Returns the final snapshot, or the
    /// CassError cass produced. Idempotent — calls after completion return
    /// `Ok(self.snapshot())` immediately without blocking.
    pub async fn wait_for_completion(&self) -> Result<IndexProgressSnapshot, CassError> {
        let mut guard = self.join_handle.lock().await;
        if let Some(handle) = guard.take() {
            match handle.await {
                Ok(Ok(())) => Ok(self.snapshot()),
                Ok(Err(err)) => Err(err),
                Err(join_err) => Err(CassError::internal(format!(
                    "cass-ffi indexer task panicked or was cancelled: {join_err}"
                ))),
            }
        } else {
            Ok(self.snapshot())
        }
    }
}

/// Map cass's `IndexingProgress.phase` discriminator (0=Idle, 1=Scanning,
/// 2=Indexing per the cass source comment) to a stable string the FFI
/// exposes. Unknown values fall through to "idle" rather than failing —
/// progress data is best-effort, not a contract.
fn phase_label(raw: usize) -> String {
    match raw {
        1 => "scanning".to_string(),
        2 => "indexing".to_string(),
        _ => "idle".to_string(),
    }
}
