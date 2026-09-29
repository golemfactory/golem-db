//! Reversible cell working state for GolemDB write branches.
//!
//! [`Branches`] opens head-only branches and validates IDs for every read,
//! write, checkpoint, rollback, discard, and `branch_info` call. Each operation uses one storage
//! snapshot for both its head check and cell reads. The manager is cloneable;
//! clones share the registry, with operations serialized per branch.
//!
//! The private cell overlay keeps puts and tombstones in memory, reads through
//! to the validated origin snapshot, and groups changes into atomic operations
//! and checkpoint frames. Public [`CellRead`] and [`CellWrite`] views provide
//! access only within manager callbacks. Scans match encoded cell-key prefixes.
//! It does not write storage, maintain indexes, or compute commitments.
//!
//! Genesis initialization, seal, and durable commit are not implemented yet.
//! Record validation, bindings, and allocation belong to record operations;
//! their cell writes participate in the same undo journal as ordinary cells.
//!
//! # Guarded branch access
//!
//! The database must already have an initialized `Superblock/head`. This layer
//! never fabricates genesis. Reads return owned values or collect scans inside
//! the callback so they cannot outlive the head check's snapshot.
//!
//! ```
//! use golemdb_branch::{Branches, BranchError};
//! use golemdb_cells::{CellKey, CellNameRef, CellValue};
//! use golemdb_storage::Database;
//!
//! fn edit<D: Database>(database: D) -> Result<(), Box<dyn std::error::Error>> {
//!     let branches = Branches::new(database)?;
//!     let branch = branches.begin()?;
//!     let key = CellKey::new(64, CellNameRef::parse_user(b"status", 64)?);
//!     branches.write(branch, |cells| {
//!         cells.put(key.clone(), CellValue::parse(b"\x02ready".to_vec())?);
//!         Ok::<_, BranchError>(())
//!     })?;
//!     branches.checkpoint(branch)?;
//!     let info = branches.branch_info(branch)?;
//!     assert_eq!(info.branch_id, branch);
//!     assert_eq!(info.version, 1);
//!     assert!(!info.sealed);
//!     let value = branches.read(branch, |cells| cells.get(&key))?;
//!     assert_eq!(value.unwrap().as_str(), Some("ready"));
//!     let cells = branches.read(branch, |cells| {
//!         cells.scan_prefix(&64u64.to_be_bytes())?.collect::<Result<Vec<_>, _>>()
//!     })?;
//!     assert_eq!(cells.len(), 1);
//!     branches.discard(branch)?;
//!     Ok(())
//! }
//! # use golemdb_storage::{MemoryDatabase, Table, WriteTransaction};
//! # let db = MemoryDatabase::new();
//! # let mut tx = db.begin_write()?;
//! # // Minimal head fixture, not a production genesis initializer.
//! # tx.put(Table("Superblock"), b"head", &[0; 72])?;
//! # tx.commit()?;
//! # edit(db)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!

mod error;
mod head;
mod journal;
mod manager;
mod overlay;
mod scan;
mod types;

pub use error::{BranchError, OperationError, Result};
pub use manager::Branches;
pub use overlay::{CellRead, CellWrite};
pub use scan::CellScan;
pub use types::{BranchId, BranchInfo, CommitId};

#[cfg(test)]
#[path = "tests/model.rs"]
mod model_tests;
#[cfg(test)]
#[path = "tests/overlay.rs"]
mod overlay_tests;
