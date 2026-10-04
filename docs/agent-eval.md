# Task: address review findings on golem-db PR #30 (record ops / receipts / key modes)

## Context
- Repo: https://github.com/golemfactory/golem-db
- Branch under review: PR #30 head `833c8d0` (stacked on `feature/golem-db-api` @ `ad609cf`).
- Normative specs in-repo: `docs/golem-db-design.md`, `docs/golem-db-api.md`.
- Design notes for this PR: `docs/record-ops.md`, `docs/implementation-review.md`, `docs/naming-migration.md`.
- Review was static analysis only. All tests are assumed to pass today; keep them passing
  (`cargo nextest run --workspace`, `cargo hack nextest run --workspace --feature-powerset`, doctests, benches compile).
- Keep: the renames, the 3-constructor opening API + `internals` feature, `Metered`/`Receipt` shape,
  the `RecordOp<Op>` typestate builder, `IntoCellValue`, `StoreFull`. These are good; do not regress them.

Work through the items in priority order. For each, make the change, add or adjust tests, and update
README/docs where behavior or claims change.

---

## P0: correctness

### 1. Receipt `priced_at` is computed outside the operation (race + side effect)
Files: `crates/api/src/database.rs`
- `Inner::priced_at` (~L212) and `Inner::write_receipt` (~L221) call `branches.branch_info(branch)`
  *after* the write, as a separate lock acquisition and a separate read transaction.
- Consequences:
  a. If another branch commits between the write and `branch_info`, the stale check inside
     `Branches::with_slot` (`crates/branch/src/manager.rs` ~L290–296) **removes the branch from the
     registry**. The caller's next `commit` then returns `HandleInvalid` instead of `Conflict`.
  b. `priced_at` then falls back to the *new* head, so the receipt reports the wrong base commit.
  c. Each record write costs 2 branch-lock acquisitions + 2 read txns.
- `get(ReadTarget::Head, ..)` (~L247) has the same problem: `priced_at` comes from
  `self.branches.head()` in one snapshot, while the record is read in another (`Records::get` opens
  its own txn). This contradicts the README claim that a head read resolves in one snapshot.
- Fix: determine `priced_at` inside the same locked operation / snapshot that performs the call.
  - Branch-targeted calls: the origin is already in `BranchState.commit_id`. Expose it from
    `Branches::read`/`write`, or have `Records::{create,patch,delete,get}` return it alongside the
    result.
  - Head/Commit-targeted `get`: return the head read from the same txn used to read the record
    (`Records::get` already reads `read_head(&tx)`).
  - Failed calls on unknown/stale handles: decide and document what `priced_at` is. Do NOT trigger
    registry removal just to compute a receipt.
- Tests:
  - Interleave a write on branch A with a commit of branch B before receipt construction (use a test
    hook or a store wrapper). Assert A's receipt has A's origin, and that A's later `commit` returns
    `Conflict`.
  - Assert a Head read's `priced_at` equals the head of the snapshot the record came from.

### 2. Silent default on storage errors in `priced_at`
Files: `crates/api/src/database.rs` ~L217, ~L247 (`.unwrap_or_default()`).
- A storage error yields `priced_at = 0`, a consensus-relevant field per the API spec
  ("Priced at branch base"). This becomes moot if item 1 is fixed properly. Make sure no path
  fabricates a commit id. Either propagate the error into `result` or make the field `Option`.

---

## P1: spec fidelity (state-root-affecting changes not in the in-repo design doc)

These change committed bytes or genesis identity, but `docs/golem-db-design.md` does not specify them:
- Empty records (contradicts api spec `create`: "≥ 1 cell"; `patch`: "may not remove the last cell").
- `#meta` cell on every user record (contradicts design §4 "One meta cell, `#key`"). Layout: 4× u64 BE
  stored as `bytes32` (`crates/record/src/meta.rs`).
- `#keyMode` / `#keySeed` in `#params`, and key derivation `H("golemdb/record-key/v1" ‖ seed ‖ id_be)`
  (`crates/record/src/crud.rs` `generated_key`, `crates/api/src/genesis.rs` `initial_cells`).
- `docs/record-ops.md` justifies these via metering spec items D4/R9, but `docs/golem-db-metering.md`
  was deleted relative to `main` and is absent from this branch.

### 3. Land the spec with the code
Pick one, and state in the PR description which was chosen:
- (a) Update `docs/golem-db-design.md` (§3, §4 `#params`, §4 reserved-record common properties,
  genesis section) and `docs/golem-db-api.md` (Records, Record keys, create/patch, errors) in this PR.
  Specify normatively: the `#meta` layout and type tag; the user-cells-only counting rules and the byte
  formulas `(8+|name|)+(1+|value|)` and `|name|+2+|value|`; empty-record semantics; `#keyMode` values;
  `#keySeed`; the derivation with its domain tag; and why the modes are exclusive. Restore or add the
  metering spec that D4/R9 refer to, or inline the needed definitions.
- (b) Split those three features out of this PR into a follow-up stacked on the spec change. Keep
  `RecordOp`, `Metered`, `Receipt`, `Details`, and the opening refactor here.
- Also rename `EngineAssigned` (spec) vs `Generated` (code) consistently, in one direction.

---

## P2: Rust design / code quality

### 4. Typestate controls methods but not data shape
File: `crates/api/src/record_op.rs`
- `RecordOp<Op>` carries `key: Option<RecordKey>`, `changes`, `projection`, `budget`, `error` for every
  op. This forces `expect("a patch/get/delete always has its key")` (~L221, ~L251, ~L262) and
  `unreachable!("a create has no remove method")` (~L195).
- Refactor so that each op stores only its own data and the invariants are type-guaranteed. For
  example, `RecordOp<Op: OpKind>` with an associated `Op::Data`, or per-op payload structs inside a
  generic wrapper. Create holds `Option<RecordKey>` + `RecordCells`; Patch holds `RecordKey` +
  `RecordPatch`; Get holds `RecordKey` + `Option<Vec<CellName>>`; Delete holds `RecordKey`.
  `budget` and `error` stay common.
- Remove all `expect`/`unreachable!` from this file. Keep the existing `compile_fail` doctests and add
  more if new misuse becomes impossible.

### 5. `RecordOp` lost `Clone`/`PartialEq`
- Proposal 1's `RecordInput`/`PatchInput` derived `Clone, PartialEq, Eq`. `RecordOp` cannot, because it
  stores `ApiError` (holds `Box<dyn Error>`).
- Store the deferred builder error as a cloneable internal type (for example an enum or `String` plus
  kind), convert it to `ApiError` at the call, and derive `Clone, PartialEq, Eq` on `RecordOp`. This
  matters for mocks (asserting received ops) and for re-executing after `Conflict`.

### 6. Deferred builder errors lose locality
- Builder steps are infallible, and only the first error is kept. Keep that design, but make the
  reported error identify the offending step: include the cell name and which method (`attribute`,
  `field`, `remove`, `key`) produced it. Optionally add a `RecordOp::validate(&self) -> Result<()>`
  so callers can check early.

### 7. `RecordMeta::apply` unchecked add
File: `crates/record/src/meta.rs` ~L32: `(count + added)` can overflow, which panics in debug builds and
wraps in release. Use `checked_add` and map overflow to `CorruptState` (or a dedicated error),
consistent with the existing `checked_sub`.

### 8. Duplicated error text (pre-existing, still present)
File: `crates/api/src/error.rs` ~L107: `RecordError::InvalidArgument(ref message)` copies `message`
into `ApiError::InvalidArgument.message` and also keeps the same error as `source`, so error-chain
reporters print the text twice. Keep one: either use a generic message plus the source, or no source.

---

## P3: shared issues inherited from the base branch (fix here or file follow-ups)

### 9. Stale-branch error depends on call order
`crates/branch/src/manager.rs` `with_slot` (~L278–296): the first access after losing the race removes
the branch. Only `commit` maps staleness to `Conflict`; every other call returns `HandleInvalid`, and
after that `commit` also returns `HandleInvalid`. A host that reads before committing never learns it
lost a race. Fix: remember "stale due to lost race" (a tombstone with reason) so `commit` returns
`Conflict` at least once, or document the contract explicitly in the spec. See review-doc C3 / D15.

### 10. Sealed-branch semantics diverge from spec
Spec (design §10, api `seal`): a sealed branch stays readable via `get`, and writes return
`HandleInvalid`. The implementation rejects reads and writes with `BranchError::Sealed`
(`manager.rs` `require_open` ~L29, called from `read` and `write`). Either allow `read` on sealed
branches (reads through the frozen overlay) and decide `Sealed` vs `HandleInvalid` for writes, or
change the spec. See review-doc C1/C2.

### 11. Handle and commit ids are interchangeable `u64` aliases
`crates/branch/src/types.rs`: `pub type CommitId = u64;` `pub type BranchId = u64;`, so
`branch_info(commit_id)` compiles. Introduce newtypes (`#[repr(transparent)]`, derive
`Copy, Eq, Ord, Hash, Debug`, with explicit `From`/`get()`). Consider the spec's
`(commitNr, branchNr)` handle shape. This is a breaking API change, so do it before external consumers
(Arkiv) depend on the crate.

### 12. Triple maintenance of the trait surface
Every `Api` method is written in the trait, in `Database`'s forwarding impl, and in `Inner`
(`crates/api/src/database.rs`). Reduce it, for example with `impl Deref<Target = dyn Api + Send + Sync>`
plus re-exported trait methods, a small forwarding macro, or by making `Database` hold the
`Arc<dyn Api>` and exposing it via `AsRef`. Keep `Database: Api` so `&dyn Api` consumers still work.

### 13. Missing spec surface (track, do not necessarily implement now)
Not implemented, and the README "Current scope" should list all of them explicitly: history and
change-sets (so no historical `get` and no `rewind`), `begin(at?)`, `branch_hash`,
`branch_info → {origin, frame_depth}` (currently `{commit_id, branch_id, version, sealed}`), `roots(at?)`,
`params()`, query/count, proofs, the metering admin API with a separate admin handle, immutable data
(stubs), and `OutOfBudget{spent}`.

---

## Minor / polish
- `Database::open_memory(&Genesis)` vs `Database::open(_, &Config)`: inconsistent parameter types.
  Consider `open_memory(&Config)`, or document why `OpenMode` is irrelevant there.
- `Genesis` will grow (`#minRetention`, segments). Decide now on `#[non_exhaustive]` + constructor
  (flagged as open in `record-ops.md` §9).
- `IntoCellValue` has no impls for `i8/i16/u8/u16` and none for `&[u8; N]` besides fixed widths, so
  `b"abc"` doesn't compile. Unsuffixed integer literals default to `i32`. Either add the impls or
  document the behavior on `RecordOp::attribute`/`field`.
- `Records::delete` classifies user cells by re-parsing names with `parse_user`
  (`crates/record/src/crud.rs` ~L223). Consider classifying by the reserved-name set (`#`/`@` prefix)
  to make intent explicit and cheaper.

## Definition of done
- P0 items fixed, with concurrency tests that reproduce the race before the fix.
- P1 resolved via option (a) or (b), with the PR description stating which.
- P2 items fixed, with no `expect`/`unreachable!` left in `record_op.rs`.
- P3 items either fixed or filed as issues and linked from README "Current scope".
- README examples still run verbatim. Full feature-powerset test run passes.