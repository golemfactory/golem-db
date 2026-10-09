//! Durable engine header, format version and physical table identifiers.
use golemdb_merkle::Hash;
use golemdb_storage::{ReadTransaction, Table, WriteTransaction};

use crate::{BranchError, CommitId, Result};

pub(crate) const SUPERBLOCK: Table = Table("Superblock");
pub(crate) const NODE_REFS: Table = Table("NodeRefs");
pub(crate) const ENGINE_TABLES: [Table; 8] = [
    SUPERBLOCK,
    NODE_REFS,
    golemdb_cells::tables::CELL,
    golemdb_cells::tables::CELL_TRIE,
    golemdb_index::tables::INDEX,
    golemdb_index::tables::INDEX_TRIE,
    golemdb_index::tables::BITMAP_TRIE,
    golemdb_index::tables::BITMAP_CONTAINER,
];
const FORMAT_KEY: &[u8] = b"format-version";
const FORMAT_VERSION: &[u8] = &[1];
const HEAD_KEY: &[u8] = b"head";
const HEAD_BYTES: usize = 8 + 32 + 32;

pub(crate) fn require_format(tx: &impl ReadTransaction) -> Result<()> {
    match tx.get(SUPERBLOCK, FORMAT_KEY)? {
        Some(version) if version == FORMAT_VERSION => Ok(()),
        _ => Err(BranchError::UnsupportedDatabaseFormat),
    }
}

pub(crate) fn write_format(tx: &mut impl WriteTransaction) -> Result<()> {
    tx.put(SUPERBLOCK, FORMAT_KEY, FORMAT_VERSION)?;
    Ok(())
}

pub(crate) struct Head {
    pub commit_id: CommitId,
    pub state_root: Hash,
    pub index_root: Hash,
}

/// Read the head commit ID from a caller-owned snapshot. To read its committed
/// cells consistently, use this same transaction with the cell storage reader.
pub fn read_head(tx: &impl ReadTransaction) -> Result<CommitId> {
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

pub(crate) fn write_head(tx: &mut impl WriteTransaction, head: &Head) -> Result<()> {
    let mut bytes = [0; HEAD_BYTES];
    bytes[..8].copy_from_slice(&head.commit_id.to_be_bytes());
    bytes[8..40].copy_from_slice(&head.state_root);
    bytes[40..].copy_from_slice(&head.index_root);
    tx.put(SUPERBLOCK, HEAD_KEY, &bytes)?;
    Ok(())
}
