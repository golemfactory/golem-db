use crate::INDEX_BRANCH_DOMAIN;
use golemdb_merkle::{Hash, HashProvider, LeafRef, RootRef, Trie};
use golemdb_storage::{ReadCursor, ReadTransaction, WriteTransaction};

use crate::{INDEX_TRIE_PATH_BYTES, IndexError, IndexTerm, Result, tables};

pub(crate) struct IndexTrie<'h, H: HashProvider> {
    trie: Trie<'h, H, INDEX_TRIE_PATH_BYTES>,
    hasher: &'h H,
}

impl<'h, H: HashProvider> IndexTrie<'h, H> {
    pub(crate) fn new(hasher: &'h H) -> Self {
        Self {
            trie: Trie::new(tables::INDEX_TRIE, INDEX_BRANCH_DOMAIN, hasher),
            hasher,
        }
    }

    fn leaf(&self, term: &IndexTerm, bitmap: Hash) -> LeafRef<INDEX_TRIE_PATH_BYTES> {
        LeafRef {
            path: term.routing_path(self.hasher),
            hash: term.leaf_hash(&bitmap, self.hasher),
        }
    }

    pub(crate) fn check(
        &self,
        tx: &impl ReadTransaction,
        root: RootRef<INDEX_TRIE_PATH_BYTES>,
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

    /// Set each term's bitmap root, removing the term for `None`, in one
    /// batched trie update.
    pub(crate) fn set<'t>(
        &self,
        tx: &mut impl WriteTransaction,
        root: RootRef<INDEX_TRIE_PATH_BYTES>,
        changes: impl IntoIterator<Item = (&'t IndexTerm, Option<Hash>)>,
    ) -> Result<RootRef<INDEX_TRIE_PATH_BYTES>> {
        let edits = changes.into_iter().map(|(term, bitmap)| match bitmap {
            Some(hash) => {
                let leaf = self.leaf(term, hash);
                (leaf.path, Some(leaf.hash))
            }
            None => (term.routing_path(self.hasher), None),
        });
        Ok(self.trie.apply(tx, root, edits)?)
    }

    pub(crate) fn reopen(
        &self,
        tx: &impl ReadTransaction,
        hash: Hash,
    ) -> Result<RootRef<INDEX_TRIE_PATH_BYTES>> {
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

pub(crate) fn decode_root(bytes: &[u8], hasher: &impl HashProvider) -> Result<Hash> {
    let hash: Hash = bytes
        .try_into()
        .map_err(|_| IndexError::Corruption("bitmap root is not 32 bytes"))?;
    if hash == hasher.hash(&[]) {
        return Err(IndexError::Corruption("empty bitmap root in Index row"));
    }
    Ok(hash)
}
