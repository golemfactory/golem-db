# Record CRUD

`golemdb-record` provides `create`, `get`, `patch`, and `delete`. Construct
`Records::new(branches.clone())` with the existing branch manager; use that manager
for begin, checkpoints, rollback, seal, commit, and discard. Facade clones share
the manager, with no separate record state. See the crate rustdoc for a usage example.

## Supported contract

- Keys are exactly 32 caller-provided bytes. Creation requires at least one user
  cell. Admission checks user names and values against the initialized limits.
- Create updates user cells, the non-indexed bytes32 `#key`, the non-indexed u64
  binding in `#recordKeys`, and `#alloc.#nextRecordID` in one branch operation.
  Allocation starts at 64; exhaustion is checked before mutation.
- `ReadTarget::Branch` sees uncommitted operations. `ReadTarget::Head` selects and
  reads the current head in one snapshot. `ReadTarget::Commit` accepts
  only the head in the acquired snapshot. Older and future commits return
  `CommitUnavailable`. A concurrent commit cannot mix a lookup's binding,
  identity, and contents across snapshots. Committed reads use the storage
  `CellReader` directly, with head metadata read from the same transaction.
- Full reads include `#key`. Projections omit missing names, collapse duplicate
  names, and return cells in byte order. An empty projection still checks record
  existence and returns its logical key. Reserved projections accept binary names.
- `RecordPatch` maps names to `CellPatch::Set` or `CellPatch::Remove`. This is caller
  intent; the cells crate's `CellChange` addresses complete storage keys.
  Patches may change type and indexed status. Empty patches, identical sets, and
  removal of absent cells do not add journal entries. Removals must leave at least
  one user cell after the entire patch; `#key` does not count. Removal-only patches
  scan the record to check this invariant.
- Delete tombstones all live cells and removes the binding. IDs are not reclaimed;
  recreation gets a fresh ID, including within the same branch. Rollback restores
  uncommitted allocations and bindings.
- Exact reserved keys reject CRUD writes with `Reserved` before branch access.
  Other requests first validate the branch. Create checks inputs, key availability,
  and allocation; patch resolves identity before validating changes. Invalid user
  names, including writes to `#key`, yield `InvalidArgument`. Inconsistent bindings
  or identities yield `CorruptState`.
- Errors retain their source through `RecordError::Branch`, `Cells`, or `Storage`.
  Failed mutations restore the overlay and version. Branch sealing derives index
  updates from actual cell changes; commit remains the durability point.

## Initialization and deferred work

Connection opening owns YAML loading and initialization. Supply the records in
`golemdb_cells::system::ALL`, their `#key` cells and `#recordKeys` bindings, three
u32 `#params` limits, and the u64 allocator. System cells are non-indexed fields.
Initialize the cell trie and head consistently and validate limits against the
store's physical ceilings. This crate provides no defaults or lazy initialization.

Engine-assigned keys, historical reads, budgets, receipts, debug options, record
versions, and metering administration are deferred. Root history belongs to commit
machinery; the shared `#rootIndex` entry does not implement its future updates.

Raw branch cell access is trusted engine access and can bypass record invariants.
The higher-level data API should expose records, not unrestricted cell writes.
