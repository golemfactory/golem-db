use std::collections::BTreeMap;

use golemdb_merkle::{Hash, HashProvider, RootRef};
use golemdb_storage::WriteTransaction;

use super::Index;
use crate::{IndexTerm, Result, path, tables};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostingChange {
    Add { term: IndexTerm, record_id: u64 },
    Remove { term: IndexTerm, record_id: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermRootChange {
    pub term: IndexTerm,
    pub before: Option<Hash>,
    pub after: Option<Hash>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexUpdate {
    pub root: RootRef<32>,
    /// Actual changes only, in term order. Roots span the complete input batch.
    pub changed_terms: Vec<TermRootChange>,
}

impl<H: HashProvider + ?Sized> Index<'_, H> {
    /// Apply postings with last-operation-wins semantics for each (term, ID).
    /// Coalesces containers and updates each affected term leaf once. The caller
    /// must supply the root matching this transaction's flat state; affected
    /// term commitments are checked, but this is not a whole-index audit.
    pub fn apply(
        &self,
        tx: &mut impl WriteTransaction,
        mut root: RootRef<32>,
        changes: impl IntoIterator<Item = PostingChange>,
    ) -> Result<IndexUpdate> {
        let mut grouped = BTreeMap::<IndexTerm, BTreeMap<u64, BTreeMap<u16, bool>>>::new();
        for change in changes {
            let (term, id, present) = match change {
                PostingChange::Add { term, record_id } => (term, record_id, true),
                PostingChange::Remove { term, record_id } => (term, record_id, false),
            };
            let (hi, lo) = path::split(id);
            grouped
                .entry(term)
                .or_default()
                .entry(hi)
                .or_default()
                .insert(lo, present);
        }
        let mut changed_terms = Vec::new();
        for (term, changes) in grouped {
            let before = self.bitmap_root(tx, &term)?;
            self.trie.check(tx, root, &term, before)?;
            let after = self.bitmaps.apply(tx, before, changes)?;
            if before == after {
                continue;
            }
            root = self.trie.set(tx, root, &term, after)?;
            match after {
                Some(hash) => tx.put(tables::INDEX, term.as_bytes(), &hash)?,
                None => {
                    tx.delete(tables::INDEX, term.as_bytes())?;
                }
            }
            changed_terms.push(TermRootChange {
                term,
                before,
                after,
            });
        }
        Ok(IndexUpdate {
            root,
            changed_terms,
        })
    }
}
