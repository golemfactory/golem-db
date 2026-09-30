/// Canonical commit number: zero is genesis.
pub type CommitId = u64;

/// Volatile, process-local branch identifier.
///
/// An ID is valid only in the manager that issued it (or one of its clones),
/// while that branch exists and its origin remains the head. Do not persist
/// IDs or reuse them after a process restart. IDs are allocated
/// monotonically across managers in this process and are never reused.
/// The origin is retained by the manager and returned by `branch_info`.
pub type BranchId = u64;

/// A consistent snapshot of one live branch's metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    /// Head over which this branch was opened.
    pub commit_id: CommitId,
    pub branch_id: BranchId,
    /// Current number of undo entries, not a monotonically increasing revision.
    /// Rollback reduces this count; failed operations restore it. An identical
    /// write to an existing overlay entry adds no undo entry.
    pub version: u64,
    /// Whether seal has computed and frozen the branch.
    pub sealed: bool,
}
