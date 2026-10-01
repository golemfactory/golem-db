use std::collections::BTreeSet;

use crate::{CellName, CellNameRef, CommitId};

/// Full records include #key; explicit projections contain only requested cells.
/// Raw binary names are valid for reserved records and are never normalized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Projection {
    #[default]
    All,
    Only(Vec<CellName>),
}

impl Projection {
    /// Select text or raw byte names, deduplicated in byte order. No write-name
    /// grammar applies: #key and reserved records' binary keys are readable.
    pub fn only<I, N>(names: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: AsRef<[u8]>,
    {
        Self::Only(
            names
                .into_iter()
                .map(|name| CellName::from(CellNameRef::raw(name.as_ref())))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        )
    }

    /// The record layer's projection shape. None means full record; Some([])
    /// returns no cells while still checking the record's identity and existence.
    pub fn as_names(&self) -> Option<&[CellName]> {
        match self {
            Self::All => None,
            Self::Only(names) => Some(names),
        }
    }
}

/// Public seal result. The commit ID is provisional until successful commit;
/// internal cell/index diffs and buffered storage rows are not exposed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealInfo {
    pub commit_id: CommitId,
    pub state_root: [u8; 32],
    pub index_root: [u8; 32],
}

impl From<&golemdb_branch::SealedCommit> for SealInfo {
    fn from(sealed: &golemdb_branch::SealedCommit) -> Self {
        Self {
            commit_id: sealed.commit_id,
            state_root: sealed.state_root,
            index_root: sealed.index_root,
        }
    }
}
