use golemdb_merkle::{Hash, HashProvider};
use golemdb_storage::ReadTransaction;

use crate::{
    IndexTerm, Result,
    bitmap_trie::BitmapTrie,
    index_trie::{IndexTrie, decode_root},
    tables,
};

mod reader;
mod writer;

pub use reader::TermScan;
pub use writer::{IndexUpdate, PostingChange, TermRootChange};

/// Index operations over caller-owned transactions. Use the same hash provider
/// and snapshot for the flat table, both tries, and the supplied root. Errors
/// during `apply` require aborting the transaction. Publish roots only on commit.
pub struct Index<'h, H: HashProvider + ?Sized> {
    hasher: &'h H,
    bitmaps: BitmapTrie<'h, H>,
    trie: IndexTrie<'h, H>,
}

impl<'h, H: HashProvider + ?Sized> Index<'h, H> {
    pub fn new(hasher: &'h H) -> Self {
        Self {
            hasher,
            bitmaps: BitmapTrie::new(hasher),
            trie: IndexTrie::new(hasher),
        }
    }

    fn bitmap_root(&self, tx: &impl ReadTransaction, term: &IndexTerm) -> Result<Option<Hash>> {
        tx.get(tables::INDEX, term.as_bytes())?
            .map(|bytes| decode_root(&bytes, self.hasher))
            .transpose()
    }
}
