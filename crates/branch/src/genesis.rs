use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use golemdb_cells::CellChange;
use golemdb_merkle::{HashProvider, RootRef};
use golemdb_storage::{Database, ReadCursor, WriteTransaction};

use crate::{
    BranchError, Result, gc,
    metadata::{ENGINE_TABLES, Head, write_format, write_head},
};

/// Create commit zero and its head-only GC metadata in empty engine tables.
///
/// The caller supplies genesis cells, including any reserved record bindings,
/// allocation and deployment parameters required by its record layer. This
/// function builds their cell/index commitments and reference counts atomically;
/// it does not enforce record-layer genesis policy or import an existing state.
/// Any nonempty engine table is rejected before writing. Existing databases
/// must be opened with [`crate::Branches::new`]; no migration is provided.
/// Errors abort creation. An unwinding panic drops the writer before propagating.
pub fn create_genesis(
    database: &impl Database,
    hasher: &impl HashProvider,
    cells: impl IntoIterator<Item = CellChange>,
) -> Result<()> {
    let mut tx = database.begin_write()?;
    match catch_unwind(AssertUnwindSafe(|| stage(&mut tx, hasher, cells))) {
        Ok(result) => result?,
        Err(panic) => {
            drop(tx);
            resume_unwind(panic);
        }
    }
    tx.commit()?;
    Ok(())
}

fn stage(
    tx: &mut impl WriteTransaction,
    hasher: &impl HashProvider,
    changes: impl IntoIterator<Item = CellChange>,
) -> Result<()> {
    for table in ENGINE_TABLES {
        if tx.cursor(table, b"")?.next()?.is_some() {
            return Err(BranchError::DatabaseNotEmpty(table));
        }
    }
    let (cells, index) = crate::state::apply(tx, hasher, RootRef::Empty, RootRef::Empty, changes)?;
    gc::seed_counts(
        tx,
        cells.root,
        index.root,
        index.changed_terms.iter().filter_map(|term| term.after),
    )?;
    write_format(tx)?;
    write_head(
        tx,
        &Head {
            commit_id: 0,
            state_root: cells.root.hash(hasher),
            index_root: index.root.hash(hasher),
        },
    )
}
