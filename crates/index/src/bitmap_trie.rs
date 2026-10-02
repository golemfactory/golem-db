use crate::{BITMAP_BRANCH_DOMAIN, BITMAP_LEAF_DOMAIN};
use golemdb_merkle::{Hash, HashProvider, LeafRef, RootRef, Trie};
use golemdb_storage::{ReadTransaction, StorageError, WriteTransaction};

use crate::{BITMAP_TRIE_PATH_BYTES, Bitmap, BitmapContainer, IndexError, Result, path, tables};

pub(crate) struct BitmapTrie<'h, H: HashProvider> {
    trie: Trie<'h, H, BITMAP_TRIE_PATH_BYTES>,
    hasher: &'h H,
}

impl<'h, H: HashProvider> BitmapTrie<'h, H> {
    pub(crate) fn new(hasher: &'h H) -> Self {
        Self {
            trie: Trie::new(tables::BITMAP_TRIE, BITMAP_BRANCH_DOMAIN, hasher),
            hasher,
        }
    }

    fn decode(&self, hash: Hash, bytes: &[u8]) -> Result<BitmapContainer> {
        if self.hasher.hash_parts(&[&[BITMAP_LEAF_DOMAIN], bytes]) != hash {
            return Err(IndexError::Corruption("container hash mismatch"));
        }
        Ok(BitmapContainer::decode(bytes)?)
    }

    fn load(
        &self,
        tx: &impl ReadTransaction,
        leaf: LeafRef<BITMAP_TRIE_PATH_BYTES>,
    ) -> Result<BitmapContainer> {
        let bytes = tx
            .get(tables::BITMAP_CONTAINER, &leaf.hash)?
            .ok_or(IndexError::Corruption("missing container"))?;
        let container = self.decode(leaf.hash, &bytes)?;
        if path::path_bytes(container.path())? != leaf.path {
            return Err(IndexError::Corruption(
                "container path disagrees with leaf reference",
            ));
        }
        Ok(container)
    }

    // Keep a reopened singleton payload so the operation does not load it twice.
    fn reopen(
        &self,
        tx: &impl ReadTransaction,
        hash: Hash,
    ) -> Result<(RootRef<BITMAP_TRIE_PATH_BYTES>, Option<BitmapContainer>)> {
        if hash == self.hasher.hash(&[]) {
            return Err(IndexError::Corruption("empty bitmap root in Index row"));
        }
        if let Some(bytes) = tx.get(tables::BITMAP_CONTAINER, &hash)? {
            let container = self.decode(hash, &bytes)?;
            return Ok((
                RootRef::Leaf(LeafRef {
                    path: path::path_bytes(container.path())?,
                    hash,
                }),
                Some(container),
            ));
        }
        // A branch miss remains a Merkle error; it never becomes an empty trie.
        let root = RootRef::Branch(hash);
        self.trie.validate_root(tx, root)?;
        Ok((root, None))
    }

    pub(crate) fn bitmap(&self, tx: &impl ReadTransaction, hash: Hash) -> Result<Bitmap> {
        let (root, cached) = self.reopen(tx, hash)?;
        if let Some(container) = cached {
            return Ok(Bitmap::from_containers([container])?);
        }
        let containers = self
            .trie
            .walk(tx, root)
            .map(|leaf| self.load(tx, leaf?))
            .collect::<Result<Vec<_>>>()?;
        Ok(Bitmap::from_containers(containers)?)
    }

    /// Changes must be sorted by (container path, offset), with one final desired
    /// membership per posting. The index writer sorts and deduplicates the batch.
    /// Consecutive changes share one container load and finalization.
    pub(crate) fn apply(
        &self,
        tx: &mut impl WriteTransaction,
        before: Option<Hash>,
        changes: impl IntoIterator<Item = (u64, u16, bool)>,
    ) -> Result<Option<Hash>> {
        let (mut root, mut cached) = match before {
            Some(hash) => self.reopen(tx, hash)?,
            None => (RootRef::Empty, None),
        };
        let mut changes = changes.into_iter().peekable();
        while let Some(&(hi, _, _)) = changes.peek() {
            let path = path::path_bytes(hi)?;
            let old = self.trie.get(tx, root, &path)?;
            let mut container = match old {
                Some(leaf) => match cached.take() {
                    Some(container) => container,
                    None => self.load(tx, leaf)?,
                },
                None => BitmapContainer::empty(hi)?,
            };
            let mut changed = false;
            while let Some((_, offset, present)) = changes.next_if(|&(path, _, _)| path == hi) {
                changed |= if present {
                    container.insert(offset)
                } else {
                    container.remove(offset)
                };
            }
            if !changed {
                continue;
            }
            if container.is_empty() {
                root = self.trie.remove(tx, root, &path)?;
            } else {
                let bytes = container.canonical_bytes()?;
                let hash = self.hasher.hash_parts(&[&[BITMAP_LEAF_DOMAIN], &bytes]);
                match tx.insert(tables::BITMAP_CONTAINER, &hash, &bytes) {
                    Ok(()) => (),
                    Err(StorageError::AlreadyExists) => {
                        if tx.get(tables::BITMAP_CONTAINER, &hash)?.as_deref()
                            != Some(bytes.as_slice())
                        {
                            return Err(IndexError::ConflictingContent);
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
                root = self.trie.insert(tx, root, LeafRef { path, hash })?;
            }
        }
        Ok((root != RootRef::Empty).then(|| root.hash(self.hasher)))
    }
}
