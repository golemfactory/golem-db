use golemdb_merkle::{BranchDomain, Hash, HashProvider, LeafRef, RootRef, Trie};
use golemdb_storage::{ReadCursor, ReadTransaction, WriteTransaction};

use crate::{IndexError, IndexTerm, Result, tables};

pub(crate) struct IndexTrie<'h, H: HashProvider + ?Sized> {
    trie: Trie<'h, H, 32>,
    hasher: &'h H,
}

impl<'h, H: HashProvider + ?Sized> IndexTrie<'h, H> {
    pub(crate) fn new(hasher: &'h H) -> Self {
        Self {
            trie: Trie::new(tables::INDEX_TRIE, BranchDomain::Index, hasher)
                .expect("32-byte paths are valid"),
            hasher,
        }
    }

    fn leaf(&self, term: &IndexTerm, bitmap: Hash) -> LeafRef<32> {
        LeafRef {
            path: term.routing_path(self.hasher),
            hash: term.leaf_hash(&bitmap, self.hasher),
        }
    }

    pub(crate) fn check(
        &self,
        tx: &impl ReadTransaction,
        root: RootRef<32>,
        term: &IndexTerm,
        before: Option<Hash>,
    ) -> Result<()> {
        if self.trie.get(tx, root, &term.routing_path(self.hasher))?
            != before.map(|hash| self.leaf(term, hash))
        {
            return Err(IndexError::RootMismatch);
        }
        Ok(())
    }

    pub(crate) fn set(
        &self,
        tx: &mut impl WriteTransaction,
        root: RootRef<32>,
        term: &IndexTerm,
        bitmap: Option<Hash>,
    ) -> Result<RootRef<32>> {
        Ok(match bitmap {
            Some(hash) => self.trie.insert(tx, root, self.leaf(term, hash))?,
            None => self
                .trie
                .remove(tx, root, &term.routing_path(self.hasher))?,
        })
    }

    pub(crate) fn reopen(&self, tx: &impl ReadTransaction, hash: Hash) -> Result<RootRef<32>> {
        if hash == self.hasher.hash(&[]) {
            if tx.cursor(tables::INDEX, b"")?.next()?.is_some() {
                return Err(IndexError::RootMismatch);
            }
            return Ok(RootRef::Empty);
        }
        if tx.get(tables::INDEX_TRIE, &hash)?.is_some() {
            let root = RootRef::Branch(hash);
            self.trie.validate_root(tx, root)?;
            return Ok(root);
        }
        // Recover a virtual singleton only from exactly one verified current row.
        let mut cursor = tx.cursor(tables::INDEX, b"")?;
        let (key, value) = cursor.next()?.ok_or(IndexError::RootMismatch)?;
        if cursor.next()?.is_some() {
            return Err(IndexError::RootMismatch);
        }
        let term = IndexTerm::decode(&key)?;
        let bitmap = decode_root(&value, self.hasher)?;
        let leaf = self.leaf(&term, bitmap);
        if leaf.hash != hash {
            return Err(IndexError::RootMismatch);
        }
        Ok(RootRef::Leaf(leaf))
    }
}

pub(crate) fn decode_root(bytes: &[u8], hasher: &(impl HashProvider + ?Sized)) -> Result<Hash> {
    let hash: Hash = bytes
        .try_into()
        .map_err(|_| IndexError::Corruption("bitmap root is not 32 bytes"))?;
    if hash == hasher.hash(&[]) {
        return Err(IndexError::Corruption("empty bitmap root in Index row"));
    }
    Ok(hash)
}
