use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use golemdb_merkle::HashProvider;
use golemdb_storage::WriteTransaction;

use crate::{
    BranchError, CommitId, Result, SealedCommit,
    head::{Head, read_head_state, write_head},
};

/// The writer has serialized with other publishers before this guard runs.
/// Any error drops the transaction, including all partially replayed rows.
pub(crate) fn persist(
    mut tx: impl WriteTransaction,
    origin: CommitId,
    sealed: &SealedCommit,
    hasher: &impl HashProvider,
    overlay: &crate::overlay::CellOverlay,
) -> Result<CommitId> {
    // Drop a failed writer outside unwinding so backend mutexes are not poisoned
    // by a panic while reading head or replaying rows. Nothing has been published.
    match catch_unwind(AssertUnwindSafe(|| {
        stage(&mut tx, origin, sealed, hasher, overlay)
    })) {
        Ok(result) => result?,
        Err(panic) => {
            drop(tx);
            resume_unwind(panic);
        }
    }
    tx.commit()?;
    Ok(sealed.commit_id)
}

fn stage(
    tx: &mut impl WriteTransaction,
    origin: CommitId,
    sealed: &SealedCommit,
    hasher: &impl HashProvider,
    overlay: &crate::overlay::CellOverlay,
) -> Result<()> {
    let head = read_head_state(tx)?;
    if head.commit_id != origin {
        return Err(BranchError::HandleInvalid);
    }
    let already_initialized = crate::gc::initialized(tx)?;
    let initialization = if already_initialized {
        Default::default()
    } else {
        crate::gc::initialize(tx, &head, hasher)?
    };
    // Activation can delete historical nodes a pre-activation seal deduplicated
    // against, even through a different manager. Refresh its physical write set
    // under the writer, after validating head; its commitments are unchanged.
    let refreshed =
        if !sealed.gc_initialized && (already_initialized || initialization.removed_rows > 0) {
            let refreshed = crate::seal::compute(tx, overlay, hasher)?;
            if refreshed.commit_id != sealed.commit_id
                || refreshed.state_root != sealed.state_root
                || refreshed.index_root != sealed.index_root
            {
                return Err(BranchError::GarbageCollection(
                    "bootstrap changed sealed commitments",
                ));
            }
            Some(refreshed)
        } else {
            None
        };
    let sealed = refreshed.as_ref().unwrap_or(sealed);
    let gc = crate::gc::changes(tx, sealed)?;
    for (table, rows) in &sealed.writes {
        for (key, value) in rows {
            match value {
                Some(value) => tx.put(*table, key, value)?,
                None => {
                    tx.delete(*table, key)?;
                }
            }
        }
    }
    crate::gc::update(tx, hasher, gc, sealed)?;
    // History and change-set tables remain deferred. Allocator/binding changes
    // already staged as cells participate in the same sealed write set.
    write_head(
        tx,
        &Head {
            commit_id: sealed.commit_id,
            state_root: sealed.state_root,
            index_root: sealed.index_root,
        },
    )
}
