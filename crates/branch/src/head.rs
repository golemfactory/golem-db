use golemdb_storage::{ReadTransaction, Table};

use crate::{BranchError, CommitId, Result};

const SUPERBLOCK: Table = Table("Superblock");
const HEAD_KEY: &[u8] = b"head";
const HEAD_BYTES: usize = 8 + 32 + 32;

/// Read the commit number from the design's head row. Roots are opaque in this
/// increment, but the complete row must have its exact fixed-width encoding.
pub(crate) fn read_head(tx: &impl ReadTransaction) -> Result<CommitId> {
    let bytes = tx
        .get(SUPERBLOCK, HEAD_KEY)?
        .ok_or(BranchError::MissingHead)?;
    if bytes.len() != HEAD_BYTES {
        return Err(BranchError::InvalidHead {
            actual: bytes.len(),
        });
    }
    Ok(u64::from_be_bytes(bytes[..8].try_into().unwrap()))
}
