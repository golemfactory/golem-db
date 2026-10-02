use crate::{CELL_BRANCH_DOMAIN, CELL_LEAF_DOMAIN};
use golemdb_merkle::{Hash, HashProvider, LeafRef, RootRef, Trie};
use golemdb_storage::{ReadCursor, ReadTransaction, WriteTransaction};

use crate::{CELL_TRIE_PATH_BYTES, CellError, CellKey, CellValue, Result, tables};

/// Cell-specific routing, leaf hashing and recovery over branch-only storage.
pub(crate) struct CellTrie<'h, H: HashProvider> {
    trie: Trie<'h, H, CELL_TRIE_PATH_BYTES>,
    hasher: &'h H,
}

impl<'h, H: HashProvider> CellTrie<'h, H> {
    pub(crate) fn new(hasher: &'h H) -> Self {
        Self {
            trie: Trie::new(tables::CELL_TRIE, CELL_BRANCH_DOMAIN, hasher),
            hasher,
        }
    }

    fn leaf(&self, path: Hash, value: &CellValue) -> LeafRef<CELL_TRIE_PATH_BYTES> {
        LeafRef {
            path,
            hash: self
                .hasher
                .hash_parts(&[&[CELL_LEAF_DOMAIN], &path, value.encoded_bytes()]),
        }
    }

    pub(crate) fn check(
        &self,
        tx: &impl ReadTransaction,
        root: RootRef<CELL_TRIE_PATH_BYTES>,
        key: &[u8],
        before: Option<&CellValue>,
    ) -> Result<()> {
        let path = self.hasher.hash(key);
        if self.trie.get(tx, root, &path)? != before.map(|value| self.leaf(path, value)) {
            return Err(CellError::RootMismatch);
        }
        Ok(())
    }

    pub(crate) fn set(
        &self,
        tx: &mut impl WriteTransaction,
        root: RootRef<CELL_TRIE_PATH_BYTES>,
        key: &[u8],
        value: Option<&CellValue>,
    ) -> Result<RootRef<CELL_TRIE_PATH_BYTES>> {
        let path = self.hasher.hash(key);
        Ok(match value {
            Some(value) => self.trie.insert(tx, root, self.leaf(path, value))?,
            None => self.trie.remove(tx, root, &path)?,
        })
    }

    pub(crate) fn reopen(
        &self,
        tx: &impl ReadTransaction,
        hash: Hash,
    ) -> Result<RootRef<CELL_TRIE_PATH_BYTES>> {
        if hash == self.hasher.hash(&[]) {
            if tx.cursor(tables::CELL, b"")?.next()?.is_some() {
                return Err(CellError::RootMismatch);
            }
            return Ok(RootRef::Empty);
        }
        if tx.get(tables::CELL_TRIE, &hash)?.is_some() {
            let root = RootRef::Branch(hash);
            self.trie.validate_root(tx, root)?;
            return Ok(root);
        }
        // A missing branch is a singleton only when exactly one current row
        // independently reconstructs the requested digest.
        let mut cursor = tx.cursor(tables::CELL, b"")?;
        let (key, value) = cursor.next()?.ok_or(CellError::RootMismatch)?;
        if cursor.next()?.is_some() {
            return Err(CellError::RootMismatch);
        }
        CellKey::decode(&key)?;
        let value = CellValue::parse(value)?;
        let leaf = self.leaf(self.hasher.hash(&key), &value);
        if leaf.hash != hash {
            return Err(CellError::RootMismatch);
        }
        Ok(RootRef::Leaf(leaf))
    }
}
