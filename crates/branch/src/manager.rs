use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use golemdb_storage::Database;

use crate::{
    BranchError, BranchId, BranchInfo, CellRead, CellWrite, CommitId, OperationError, Result,
    head::read_head, overlay::CellOverlay,
};

// A process-wide counter prevents handle aliasing between independent managers,
// including a manager reopened over the same head after the old one was dropped.
static NEXT_BRANCH_ID: AtomicU64 = AtomicU64::new(0);

struct BranchState {
    commit_id: CommitId,
    overlay: CellOverlay,
}

type Entry = Arc<Mutex<Option<BranchState>>>;

struct Inner<D> {
    database: D,
    branches: Mutex<BTreeMap<BranchId, Entry>>,
}

/// Head-only branch lifecycle, metadata, and guarded access to cell working state.
///
/// Clones share the registry and database. Each branch has its own lock, held
/// for an entire operation; independent branches can execute concurrently when
/// the database supports sharing between threads. No storage snapshot is kept
/// between calls. Every call on a branch ID checks the head in a fresh snapshot
/// acquired *after* locking the branch, then uses that snapshot for cell reads.
/// An in-flight operation may finish over its snapshot if head moves meanwhile;
/// the next call rejects the stale ID. Stale branches are reclaimed when
/// accessed; dropping the last manager clone releases all remaining branches.
///
/// Callbacks should use their supplied view, not re-enter branch operations:
/// the current branch lock is already held and nested calls can deadlock. They
/// can return owned results, but cannot retain views or borrowed scans. Callback
/// panics unwind atomic writes before the lock is released and are then resumed,
/// leaving the manager usable if the caller catches the panic.
///
/// This increment does not initialize genesis, validate format/hash settings,
/// authenticate roots, seal, commit, or rewind the database. External writers
/// must publish cells and a strictly increasing head atomically; changing cells
/// under an unchanged head or rewinding it violates this manager's contract.
pub struct Branches<D> {
    inner: Arc<Inner<D>>,
}

impl<D> Clone for Branches<D> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<D: Database> Branches<D> {
    /// Open a manager over an initialized head. Performs no writes and fails
    /// if the head is missing, malformed, or unreadable.
    pub fn new(database: D) -> Result<Self> {
        {
            let tx = database.begin_read()?;
            read_head(&tx)?;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                database,
                branches: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    pub fn head(&self) -> Result<CommitId> {
        read_head(&self.inner.database.begin_read()?)
    }

    /// Open a new independent overlay and its first frame over the current head.
    pub fn begin(&self) -> Result<BranchId> {
        let state = BranchState {
            commit_id: self.head()?,
            overlay: CellOverlay::new(),
        };
        let branch_id = allocate_branch_id(&NEXT_BRANCH_ID)?;
        self.inner
            .branches
            .lock()
            .map_err(|_| BranchError::Poisoned("registry"))?
            .insert(branch_id, Arc::new(Mutex::new(Some(state))));
        Ok(branch_id)
    }

    /// Read metadata under the branch lock and the usual head validation.
    /// Version counts retained undo entries. Seal is a later increment, so all
    /// currently live branches are open and report `sealed: false`.
    pub fn branch_info(&self, branch_id: BranchId) -> Result<BranchInfo> {
        self.with_branch(branch_id, |slot, _| {
            let state = slot.as_ref().unwrap();
            BranchInfo {
                commit_id: state.commit_id,
                branch_id,
                version: state.overlay.version(),
                sealed: false,
            }
        })
    }

    /// Read cells from one validated branch snapshot. Callback errors are kept
    /// separate from errors admitting the operation (such as a stale ID).
    pub fn read<T, E>(
        &self,
        branch_id: BranchId,
        operation: impl FnOnce(&CellRead<'_, D::Read<'_>>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, OperationError<E>> {
        self.with_branch(branch_id, |slot, origin| {
            operation(&CellRead::new(&slot.as_ref().unwrap().overlay, origin))
        })
        .map_err(OperationError::Branch)?
        .map_err(OperationError::Operation)
    }

    /// Atomically apply one cell operation group after validating its ID.
    /// Callback failure restores the overlay without consuming a checkpoint.
    pub fn write<T, E>(
        &self,
        branch_id: BranchId,
        operation: impl FnOnce(&mut CellWrite<'_, D::Read<'_>>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, OperationError<E>> {
        self.with_branch(branch_id, |slot, origin| {
            slot.as_mut().unwrap().overlay.write(origin, operation)
        })
        .map_err(OperationError::Branch)?
        .map_err(OperationError::Operation)
    }

    pub fn checkpoint(&self, branch_id: BranchId) -> Result<()> {
        self.with_branch(branch_id, |slot, _| {
            slot.as_mut().unwrap().overlay.checkpoint()
        })
    }

    /// Validate the head, then undo one frame in memory. Only admission reads
    /// storage; the inverse replay itself performs no storage access.
    pub fn rollback(&self, branch_id: BranchId) -> Result<()> {
        self.with_branch(branch_id, |slot, _| {
            slot.as_mut().unwrap().overlay.rollback()
        })?
    }

    /// Consume a live branch without reverse replay. A stale or already
    /// discarded ID returns `HandleInvalid`; stale state is still released.
    pub fn discard(&self, branch_id: BranchId) -> Result<()> {
        self.with_branch(branch_id, |slot, _| {
            *slot = None;
        })?;
        self.remove(branch_id)
    }

    fn remove(&self, branch_id: BranchId) -> Result<()> {
        self.inner
            .branches
            .lock()
            .map_err(|_| BranchError::Poisoned("registry"))?
            .remove(&branch_id);
        Ok(())
    }

    fn with_branch<T>(
        &self,
        branch_id: BranchId,
        operation: impl FnOnce(&mut Option<BranchState>, &D::Read<'_>) -> T,
    ) -> Result<T> {
        let entry = self
            .inner
            .branches
            .lock()
            .map_err(|_| BranchError::Poisoned("registry"))?
            .get(&branch_id)
            .cloned()
            .ok_or(BranchError::HandleInvalid)?;
        // Never wait for a branch while holding the registry lock.
        let mut slot = entry.lock().map_err(|_| BranchError::Poisoned("branch"))?;
        if slot.is_none() {
            return Err(BranchError::HandleInvalid);
        }
        let origin = self.inner.database.begin_read()?;
        if read_head(&origin)? != slot.as_ref().unwrap().commit_id {
            *slot = None;
            self.remove(branch_id)?;
            return Err(BranchError::HandleInvalid);
        }
        // CellOverlay's write guard restores all partial mutations on unwind.
        // Unlock before resuming a callback panic so it cannot poison the lock.
        let result = catch_unwind(AssertUnwindSafe(|| operation(&mut slot, &origin)));
        drop(slot);
        match result {
            Ok(value) => Ok(value),
            Err(panic) => resume_unwind(panic),
        }
    }
}

fn allocate_branch_id(counter: &AtomicU64) -> Result<u64> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .map_err(|_| BranchError::BranchNumberExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_counter_never_wraps_or_reuses_numbers() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_branch_id(&counter).unwrap(), u64::MAX - 1);
        for _ in 0..2 {
            assert!(matches!(
                allocate_branch_id(&counter),
                Err(BranchError::BranchNumberExhausted)
            ));
        }
    }
}
