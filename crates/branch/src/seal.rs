use golemdb_cells::{
    CellChange, CellKey, CellNameRef, CellType, CellValueRef, Cells, CellsUpdate, system, tables,
};
use golemdb_index::{Index, IndexUpdate};
use golemdb_merkle::{Hash, HashProvider, RootRef};
use golemdb_storage::ReadTransaction;

use crate::{
    BranchError, CommitId, Result,
    buffer::{Buffered, Writes},
    metadata::read_head_state,
    overlay::CellOverlay,
};

/// Computed next state, retained in memory until commit or discard.
/// The commit number is provisional: sealing does not reserve or advance head.
/// Returned through `Arc` so observing the result does not copy the cell diff.
#[derive(Debug)]
pub struct SealedCommit {
    pub commit_id: CommitId,
    pub state_root: Hash,
    pub index_root: Hash,
    pub cells: CellsUpdate,
    pub index: IndexUpdate,
    // Replayed by commit inside the head-checked writer.
    pub(crate) writes: Writes,
    pub(crate) origin_cells: RootRef<32>,
    pub(crate) origin_index: RootRef<32>,
}

pub(crate) fn compute(
    origin: &impl ReadTransaction,
    overlay: &CellOverlay,
    hasher: &impl HashProvider,
) -> Result<SealedCommit> {
    let head = read_head_state(origin)?;
    crate::metadata::require_format(origin)?;
    let commit_id = head
        .commit_id
        .checked_add(1)
        .ok_or(BranchError::CommitNumberExhausted)?;
    let cells = Cells::new(hasher);
    let index = Index::new(hasher);
    // Reopen against the committed origin, before applying ANY staged cell.
    let cell_root = cells.reopen(origin, head.state_root)?;
    let index_root = index.reopen(origin, head.index_root)?;
    let roots_key = CellKey::new(
        system::ROOTS.id,
        CellNameRef::raw(&head.commit_id.to_be_bytes()),
    );
    if origin.get(tables::CELL, &roots_key.encode())?.is_some() {
        return Err(BranchError::RootsCellExists);
    }
    let roots_value = CellValueRef::new(
        CellType::Bytes,
        &[head.state_root, head.index_root].concat(),
        false,
    )?
    .into();
    // The engine's system write is last and authoritative if a low-level caller
    // staged this key. Record-layer authorization remains outside branch.
    let changes = overlay.changes().chain([CellChange::Put {
        key: roots_key,
        value: roots_value,
    }]);
    let mut tx = Buffered::new(origin);
    let (cells_update, index_update) =
        crate::state::apply(&mut tx, hasher, cell_root, index_root, changes)?;
    Ok(SealedCommit {
        commit_id,
        state_root: cells_update.root.hash(hasher),
        index_root: index_update.root.hash(hasher),
        cells: cells_update,
        index: index_update,
        writes: tx.into_writes(),
        origin_cells: cell_root,
        origin_index: index_root,
    })
}
