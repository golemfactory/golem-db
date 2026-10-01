use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use golemdb_merkle::HashProvider;
use golemdb_storage::{Database, ReadTransaction};

use crate::{
    BranchError, BranchId, BranchInfo, CellRead, CellWrite, CommitId, OperationError, Result,
    SealedCommit, head::read_head, overlay::CellOverlay,
};

// A process-wide counter prevents handle aliasing between independent managers,
// including a manager reopened over the same head after the old one was dropped.
static NEXT_BRANCH_ID: AtomicU64 = AtomicU64::new(0);

struct BranchState {
    commit_id: CommitId,
    overlay: CellOverlay,
    sealed: Option<Arc<SealedCommit>>,
}

impl BranchState {
    fn require_open(&self) -> Result<()> {
        if self.sealed.is_some() {
            Err(BranchError::Sealed)
        } else {
            Ok(())
        }
    }

    fn seal(
        &mut self,
        origin: &impl ReadTransaction,
        hasher: &impl HashProvider,
    ) -> Result<Arc<SealedCommit>> {
        if let Some(sealed) = &self.sealed {
            return Ok(Arc::clone(sealed));
        }
        let sealed = Arc::new(crate::seal::compute(origin, &self.overlay, hasher)?);
        self.sealed = Some(Arc::clone(&sealed));
        Ok(sealed)
    }
}

type Entry = Arc<Mutex<Option<BranchState>>>;

struct Inner<D, H> {
    hasher: H,
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
/// authenticate entire tries, maintain history, or rewind the database. The supplied hash
/// provider must match the deployment and is shared by all branch seals. External writers
/// must publish cells and a strictly increasing head atomically; changing cells
/// under an unchanged head or rewinding it violates this manager's contract.
pub struct Branches<D, H> {
    inner: Arc<Inner<D, H>>,
}

impl<D, H> Clone for Branches<D, H> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<D: Database, H: HashProvider> Branches<D, H> {
    /// Open a manager over an initialized head. Performs no writes and fails
    /// if the head is missing, malformed, or unreadable.
    pub fn new(database: D, hasher: H) -> Result<Self> {
        {
            let tx = database.begin_read()?;
            read_head(&tx)?;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                database,
                hasher,
                branches: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    pub fn head(&self) -> Result<CommitId> {
        read_head(&self.inner.database.begin_read()?)
    }

    /// The database shared by this manager. Committed readers can open a
    /// snapshot directly; no branch registration or overlay is needed.
    /// Direct writers must obey the publication contract documented on Branches.
    pub fn database(&self) -> &D {
        &self.inner.database
    }

    /// Open a new independent overlay and its first frame over the current head.
    pub fn begin(&self) -> Result<BranchId> {
        let state = BranchState {
            commit_id: self.head()?,
            overlay: CellOverlay::new(),
            sealed: None,
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
    /// Version counts retained undo entries; sealing does not add journal entries.
    pub fn branch_info(&self, branch_id: BranchId) -> Result<BranchInfo> {
        self.with_branch(branch_id, |state, _| BranchInfo {
            commit_id: state.commit_id,
            branch_id,
            version: state.overlay.version(),
            sealed: state.sealed.is_some(),
        })
    }

    /// Read cells from one validated branch snapshot. Callback errors are kept
    /// separate from errors admitting the operation (such as a stale ID).
    pub fn read<T, E>(
        &self,
        branch_id: BranchId,
        operation: impl FnOnce(&CellRead<'_, D::Read<'_>>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, OperationError<E>> {
        self.with_branch(branch_id, |state, origin| {
            state.require_open().map_err(OperationError::Branch)?;
            operation(&CellRead::new(&state.overlay, origin)).map_err(OperationError::Operation)
        })
        .map_err(OperationError::Branch)?
    }

    /// Atomically apply one cell operation group after validating its ID.
    /// Callback failure restores the overlay without consuming a checkpoint.
    pub fn write<T, E>(
        &self,
        branch_id: BranchId,
        operation: impl FnOnce(&mut CellWrite<'_, D::Read<'_>>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, OperationError<E>> {
        self.with_branch(branch_id, |state, origin| {
            state.require_open().map_err(OperationError::Branch)?;
            state
                .overlay
                .write(origin, operation)
                .map_err(OperationError::Operation)
        })
        .map_err(OperationError::Branch)?
    }

    pub fn checkpoint(&self, branch_id: BranchId) -> Result<()> {
        self.with_branch(branch_id, |state, _| {
            state.require_open()?;
            state.overlay.checkpoint();
            Ok(())
        })?
    }

    /// Validate the head, then undo one frame in memory. Only admission reads
    /// storage; the inverse replay itself performs no storage access.
    pub fn rollback(&self, branch_id: BranchId) -> Result<()> {
        self.with_branch(branch_id, |state, _| {
            state.require_open()?;
            state.overlay.rollback()
        })?
    }

    /// Compute cell/index updates and stage physical rows without opening a
    /// database writer. Failure leaves the overlay and checkpoints untouched.
    /// Success freezes cell access. Repeated sealing returns the cached result,
    /// after validating head again; it does not reserve a commit number.
    pub fn seal(&self, branch_id: BranchId) -> Result<Arc<SealedCommit>> {
        self.with_branch(branch_id, |state, origin| {
            state.seal(origin, &self.inner.hasher)
        })?
    }

    /// Seal if necessary, then atomically persist the computed rows and head.
    /// Head is checked again inside the storage writer before any row is written.
    /// Success consumes the branch; competitors over the old head become stale.
    /// A failed seal leaves the branch open. A storage failure after sealing
    /// retains the sealed result for retry or discard, subject to head validation.
    /// A live branch whose origin lost the race returns Conflict and is consumed.
    /// Unknown or already invalidated/consumed handles return HandleInvalid.
    pub fn commit(&self, branch_id: BranchId) -> Result<CommitId> {
        self.with_slot(branch_id, BranchError::Conflict, |slot, origin| {
            let state = slot.as_mut().ok_or(BranchError::HandleInvalid)?;
            let sealed = state.seal(origin, &self.inner.hasher)?;
            let tx = self.inner.database.begin_write()?;
            match crate::commit::persist(tx, state.commit_id, &sealed) {
                Ok(commit_id) => {
                    *slot = None;
                    // Publication has succeeded. Registry housekeeping must not
                    // turn that durable success into an apparent commit failure.
                    // Queued callers already holding this entry see None.
                    let _ = self.remove(branch_id);
                    Ok(commit_id)
                }
                Err(BranchError::Conflict) => {
                    *slot = None;
                    self.remove(branch_id)?;
                    Err(BranchError::Conflict)
                }
                Err(error) => Err(error),
            }
        })?
    }

    /// Consume a live branch without reverse replay. A stale or already
    /// discarded ID returns `HandleInvalid`; stale state is still released.
    pub fn discard(&self, branch_id: BranchId) -> Result<()> {
        self.with_slot(branch_id, BranchError::HandleInvalid, |slot, _| {
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

    /// Ordinary operations receive a live state, never its registry slot.
    fn with_branch<T>(
        &self,
        branch_id: BranchId,
        operation: impl FnOnce(&mut BranchState, &D::Read<'_>) -> T,
    ) -> Result<T> {
        self.with_slot(branch_id, BranchError::HandleInvalid, |slot, origin| {
            let state = slot.as_mut().ok_or(BranchError::HandleInvalid)?;
            Ok(operation(state, origin))
        })?
    }

    /// Keep slot removal, locking, head validation, and panic handling in one
    /// place. Only discard and commit need direct slot access; other operations use
    /// `with_branch` so they cannot remove or replace the state.
    fn with_slot<T>(
        &self,
        branch_id: BranchId,
        stale_error: BranchError,
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
        let commit_id = slot.as_ref().ok_or(BranchError::HandleInvalid)?.commit_id;
        let origin = self.inner.database.begin_read()?;
        if read_head(&origin)? != commit_id {
            *slot = None;
            self.remove(branch_id)?;
            return Err(stale_error);
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
