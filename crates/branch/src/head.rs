use golemdb_merkle::Hash;
use golemdb_storage::{ReadTransaction, Table};

use crate::{BranchError, CommitId, Result};

const SUPERBLOCK: Table = Table("Superblock");
const HEAD_KEY: &[u8] = b"head";
const HEAD_BYTES: usize = 8 + 32 + 32;

pub(crate) struct Head {
    pub commit_id: CommitId,
    pub state_root: Hash,
    pub index_root: Hash,
}

pub(crate) fn read_head(tx: &impl ReadTransaction) -> Result<CommitId> {
    Ok(read_head_state(tx)?.commit_id)
}

/// Decode the design's fixed-width head row from one snapshot.
pub(crate) fn read_head_state(tx: &impl ReadTransaction) -> Result<Head> {
    let bytes = tx
        .get(SUPERBLOCK, HEAD_KEY)?
        .ok_or(BranchError::MissingHead)?;
    if bytes.len() != HEAD_BYTES {
        return Err(BranchError::InvalidHead {
            actual: bytes.len(),
        });
    }
    Ok(Head {
        commit_id: u64::from_be_bytes(bytes[..8].try_into().unwrap()),
        state_root: bytes[8..40].try_into().unwrap(),
        index_root: bytes[40..72].try_into().unwrap(),
    })
}
