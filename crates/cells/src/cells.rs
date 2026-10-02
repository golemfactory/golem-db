use std::iter::FusedIterator;

use golemdb_merkle::{Hash, HashProvider, RootRef};
use golemdb_storage::{ReadCursor, ReadTransaction, Scan, WriteTransaction, scan_prefix};

use crate::{CELL_TRIE_PATH_BYTES, CellKey, CellValue, Result, cell_trie::CellTrie, tables};

/// Cell storage and commitment over caller-owned transactions.
///
/// Use one hash provider and a root matching the transaction's flat cell state.
/// `apply` belongs at seal/commit, after the engine has formed a net batch. Seal
/// requires a staged transaction; this interface does not create an overlay.
/// Abort the transaction after any mutation error and publish roots only after
/// commit. This layer does not update the index, history or head, enforce record
/// lifecycle rules, or apply deployment limits: those belong to the engine.
pub struct Cells<'h, H: HashProvider> {
    trie: CellTrie<'h, H>,
}

/// Final requested cell writes. Repeated keys in a batch use the last operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellChange {
    Put { key: CellKey, value: CellValue },
    Delete { key: CellKey },
}

/// An actual change, retaining complete tagged values for index/history work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellValueChange {
    pub key: CellKey,
    pub before: Option<CellValue>,
    pub after: Option<CellValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellsUpdate {
    pub root: RootRef<CELL_TRIE_PATH_BYTES>,
    /// Actual changes only, ordered by complete key, spanning the input batch.
    pub changed_cells: Vec<CellValueChange>,
}

impl<'h, H: HashProvider> Cells<'h, H> {
    pub fn new(hasher: &'h H) -> Self {
        Self {
            trie: CellTrie::new(hasher),
        }
    }

    /// Read the snapshot's flat value. The owned result outlives the transaction.
    /// This is not a proof or an authenticated read against a supplied root.
    pub fn get(&self, tx: &impl ReadTransaction, key: &CellKey) -> Result<Option<CellValue>> {
        read_value(tx, &key.encode())
    }

    /// Scan one record lazily in name-byte order, including raw reserved names.
    /// Every returned row is owned; only the iterator borrows the transaction.
    pub fn scan_record<'tx, T: ReadTransaction>(
        &self,
        tx: &'tx T,
        record_id: u64,
    ) -> Result<CellScan<T::Cursor<'tx>>> {
        Ok(CellScan {
            scan: scan_prefix(tx, tables::CELL, record_id.to_be_bytes().to_vec())?,
            done: false,
        })
    }

    /// Recover root metadata. Empty/singleton recovery uses the flat table from
    /// the same snapshot; a branch root's own row is validated, not its entire
    /// subtree or correspondence with all flat rows. Historical singleton
    /// recovery needs historical values or retained `RootRef` metadata.
    pub fn reopen(
        &self,
        tx: &impl ReadTransaction,
        hash: Hash,
    ) -> Result<RootRef<CELL_TRIE_PATH_BYTES>> {
        self.trie.reopen(tx, hash)
    }

    /// Apply each key's final value once, skipping no-ops and absent deletions.
    /// Affected commitments are checked against the original flat values even
    /// for no-ops. This is not a whole-state audit; the caller must supply the
    /// matching root. Errors require aborting the enclosing transaction.
    pub fn apply(
        &self,
        tx: &mut impl WriteTransaction,
        mut root: RootRef<CELL_TRIE_PATH_BYTES>,
        changes: impl IntoIterator<Item = CellChange>,
    ) -> Result<CellsUpdate> {
        let mut changes: Vec<_> = changes
            .into_iter()
            .map(|change| match change {
                CellChange::Put { key, value } => (key, Some(value)),
                CellChange::Delete { key } => (key, None),
            })
            .collect();
        // Stable sorting preserves input order for each key. The value must
        // not affect ordering: the final input operation wins.
        changes.sort_by(|a, b| a.0.cmp(&b.0));
        changes.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                // dedup_by keeps the earlier entry; move the last value into it.
                earlier.1 = later.1.take();
                true
            } else {
                false
            }
        });
        let mut changed_cells = Vec::new();
        for (key, after) in changes {
            let encoded_key = key.encode();
            let before = read_value(tx, &encoded_key)?;
            self.trie.check(tx, root, &encoded_key, before.as_ref())?;
            if before == after {
                continue;
            }
            root = self.trie.set(tx, root, &encoded_key, after.as_ref())?;
            match &after {
                Some(value) => tx.put(tables::CELL, &encoded_key, value.encoded_bytes())?,
                None => {
                    tx.delete(tables::CELL, &encoded_key)?;
                }
            }
            changed_cells.push(CellValueChange { key, before, after });
        }
        Ok(CellsUpdate {
            root,
            changed_cells,
        })
    }
}

fn read_value(tx: &impl ReadTransaction, key: &[u8]) -> Result<Option<CellValue>> {
    tx.get(tables::CELL, key)?
        .map(CellValue::parse)
        .transpose()
        .map_err(Into::into)
}

/// Lazy record scan. Storage or decoding errors are returned once and terminate
/// iteration. Missing records produce an empty iterator.
pub struct CellScan<C> {
    scan: Scan<C>,
    done: bool,
}

impl<C: ReadCursor> Iterator for CellScan<C> {
    type Item = Result<(CellKey, CellValue)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = self
            .scan
            .next()?
            .map_err(Into::into)
            .and_then(|(key, value)| Ok((CellKey::decode(&key)?, CellValue::parse(value)?)));
        if result.is_err() {
            self.done = true;
        }
        Some(result)
    }
}

impl<C: ReadCursor> FusedIterator for CellScan<C> {}
