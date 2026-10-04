//! Record CRUD over guarded branch cell views.
//!
//! Caller-assigned keys only. Each mutation is one atomic branch operation,
//! including allocator and binding changes. Checkpoints, rollback and commit
//! remain on the shared [`Branches`](golemdb_branch::Branches) manager supplied to [`Records::new`].
//! Direct low-level cell writes are trusted library operations and can bypass
//! these invariants; expose Records, not raw branch writes, to data clients.
//!
//! Opening/initialization belongs to the caller: required `#params` cells,
//! `#alloc.#nextRecordID`, and reserved records (including their `#key` cells
//! and `#recordKeys` bindings) must already exist. No YAML loading, genesis
//! writes, local parameter defaults, or parameter cache lives here.
//!
//! This iteration is unmetered: no budgets, receipts, debug options, or OCC.
//! [`ReadTarget::Head`] selects and reads the current head in one snapshot.
//! Commit-targeted reads support the head only and explicitly reject other
//! commits until history is available. Deletes enumerate and tombstone every
//! live cell; sealing derives index changes from the resulting cell diff.
//!
//! ```
//! use golemdb_branch::Branches;
//! use golemdb_cells::{CellNameRef, CellType, CellValueRef};
//! use golemdb_merkle::HashProvider;
//! use golemdb_record::{ReadTarget, RecordKey, Records};
//! use golemdb_storage::Store;
//!
//! // The connection layer has already initialized the database and parameters.
//! fn example<S: Store, H: HashProvider>(branches: Branches<S, H>)
//!     -> Result<(), Box<dyn std::error::Error>>
//! {
//!     let records = Records::new(branches.clone());
//!     let branch = branches.begin()?;
//!     let key = RecordKey([7; 32]);
//!     let name = CellNameRef::parse_user(b"status", 32)?.into();
//!     let value = CellValueRef::new(CellType::Str, b"ready", true)?.into();
//!     records.create(branch, key, [(name, value)].into())?;
//!     let pending = records.get(ReadTarget::Branch(branch), key, None)?;
//!     let commit = branches.commit(branch)?;
//!     let stored = records.get(ReadTarget::Commit(commit), key, None)?;
//!     assert_eq!(pending, stored);
//!     Ok(())
//! }
//! ```

mod crud;
mod details;
mod error;
mod meta;
mod state;
mod types;

pub use crud::Records;
pub use details::Details;
pub use error::{RecordError, Result};
pub use meta::RecordMeta;
pub use types::{CellPatch, ReadTarget, Record, RecordCells, RecordKey, RecordPatch};
