# Record operations: concept and implementation plan

The concept behind the next reshaping of the record API, and the plan for implementing it on branch
`matthiaszimmermann/feat/record-ops` (stacked on `matthiaszimmermann/refactor/namings`). It builds
on the vocabulary and API shape of [implementation-review.md](implementation-review.md) Part 0 and
takes its record model and counting rules from the metering spec (`golem-db-metering.md`, Record
Model, D4, D5).

Status: concept agreed 2026-10-04; **implemented** on the branch in three commits: `e476b8e`
(Group 1), `d7effe8` (Group 2), `6ada31e` (Group 3). Section 9 lists where the implementation
adds to or differs from this concept.

## 1. Record model

A record is a **key**, **0–n user cells** and **metadata**. Stored, it is a set of n + 2 cells,
plus one cell elsewhere:

| Cell | Owner | Written by |
| --- | --- | --- |
| n user cells | the record | the caller, through `create` and `patch` |
| `#key` | the record | the database: the record key |
| `#meta` | the record | the database: counts over the user cells (section 2) |
| binding `recordKey → recordID` | reserved record `#recordKeys` | the database |

So `create` writes n + 3 cells and `delete` removes n + 3.

**Empty records are legal.** `create` may supply zero user cells, and `patch` may remove the last
one. A record exists from `create` until `delete`, independent of its content. This replaces the
current rule that a record needs at least one user cell.

## 2. `#meta`

A fixed-width encoding of four counts over the record's **user cells only** (system cells are
fixed-size and accounted separately), as defined by the metering spec (D4):

| Field | Value |
| --- | --- |
| `cells` | number of user cells |
| `cell_bytes` | Σ over user cells of `(8 + \|name\|) + (1 + \|value\|)` |
| `indexed_cells` | number of user cells that are attributes (one index entry each) |
| `index_bytes` | Σ over indexed cells of `\|name\| + 2 + \|value\|` |

Encoding: the four fields in this order, each `u64` big-endian, 32 bytes, stored as a
`FixedBytes(W32)` cell like `#key`. Normative (the layout is under the state root), so it belongs in
D09 and Appendix A. `#meta` is part of `get`'s full result, like `#key`.

The layout is meant to be complete once metering is implemented. Versioning is not needed until
then; if a later layout adds fields, every record's `#meta` changes, which is a full-state migration.

### What `#meta` enables

**Completeness proofs for full-record reads.** A client holding a trusted state root receives the
binding, `#meta` and `cells + 2` distinct cells of the record, each with an inclusion proof. Proofs
cannot be forged and `#meta` is committed state, so if every proof verifies, no cell was suppressed.
This works although cells are scattered across the trie by hashed keys, where a range proof is not
possible (D06 treats range completeness as a non-goal). Conditions:

1. every mutation maintains the counts exactly (create, patch including no-op removes and type
   changes, delete, rollback); a bug is consensus-visible, so it fails loudly, not silently;
2. the definition is fixed: user cells only; the verifier adds 2 for `#key` and `#meta`;
3. the client has a trusted root (the host's job).

Not covered: projections (absence of a named cell needs a non-inclusion proof per name), whole
records withheld from a read (non-inclusion of the binding) or from a query (index proofs).

**A deletion-cost bound.** By D5 the cost of deleting a record depends only on fixed terms,
`cells` and `indexed_cells`, the active weights and the modeled trie depth (code per metering model
version). It is exact at current weights; the actual deletion can only cost less. A host can
compute it from `#meta` alone.

## 3. Key modes

A database has exactly one key mode for its whole life, fixed in genesis:

| Mode | `create` | Key |
| --- | --- | --- |
| **caller-assigned** | must name the key | the caller's; an existing key fails with `AlreadyExists` |
| **generated** | must not name a key | `H("golemdb/record-key/v1" ‖ seed ‖ id)`; `AlreadyExists` cannot occur |

- `H` is the deployment's hash function; `id` is the new record's ID (`#nextRecordID` before it is
  incremented) as `u64` big-endian; `seed` is 32 bytes from genesis. Normative: D09 and Appendix A.
- Deterministic (seed and allocator are committed), unique (IDs are never reused; of two branches
  allocating the same ID only one commits), and distinct across deployments with different seeds.
- **The seed is not secret.** It is readable like every `#params` cell, so keys are predictable.
  That is safe because the modes are exclusive: in the generated mode there are no caller-assigned
  keys that could claim a future generated key first. This is why the modes cannot be mixed.
- A `create` that does not match the mode fails with `KeyModeMismatch` before anything is written.
- Genesis records the mode in `#params`: `#keyMode` (`u32`: 0 caller-assigned, 1 generated) and, in
  the generated mode only, `#keySeed` (`bytes32`). Both change `genesis_id`; existing databases are
  disposable at this stage (review item 1).
- The genesis file must name the mode; there is no default:

  ```yaml
  record_keys: caller_assigned
  # or
  record_keys:
    generated:
      seed: "0x…"            # 32 bytes
  ```

  `Genesis::DEV` uses `caller_assigned`.

## 4. `RecordOp<Op>`

One builder type for all four record calls. The type parameter names the operation and decides
which methods exist.

```rust
pub struct RecordOp<Op> { … }
pub enum Create {}
pub enum Patch {}
pub enum Get {}
pub enum Delete {}
```

| Op | Constructor | Methods | Notes |
| --- | --- | --- | --- |
| `RecordOp<Create>` | `RecordOp::create()` | `.key(k)`, `.attribute(n, v)`, `.field(n, v)` | key optional (key mode) |
| `RecordOp<Patch>` | `RecordOp::patch(k)` | `.attribute(n, v)`, `.field(n, v)`, `.remove(n)` | key mandatory |
| `RecordOp<Get>` | `RecordOp::get(k)` | `.only([names])` | default: all cells |
| `RecordOp<Delete>` | `RecordOp::delete(k)` | – | |
| all | | `.budget(n)` | not enforced until metering (section 5) |

- **Compile-time checks:** no `remove` on a create; a patch, get or delete always has its key;
  `.key()` only on a create; a projection only on a get.
- **Runtime checks, at the call:** key mode (`KeyModeMismatch`); invalid names, duplicate names, a
  set and a remove of the same name, `.key()` twice (`InvalidArgument`). Builder steps are
  infallible; the first problem is reported by the call, so call sites have no `?` per step. The
  error names the step (`field("price")`) and keeps a typed cause as its source; `.validate()`
  reports it early. Each operation holds only its own data, and the kept error is a cloneable
  internal type, so `RecordOp` is `Clone + PartialEq + Eq`.
- **Values from Rust types:** `.attribute("price", 50i32)`, `.field("name", "Laptop")`. A conversion
  trait (`IntoCellValue`) covers integers, strings, bytes, booleans and `CellValue` itself;
  fallible conversions (a NaN float) are recorded and reported at the call like other errors. The
  Rust type decides the cell type, so examples write `50i32` / `50i64` explicitly.
- **Every write declares its kind**, attribute or field, as the spec requires ("typing is per
  write"). There is no kind-preserving `set`: a call site shows which writes touch the index.
  `.attribute` and `.field` also accept a `CellValue` (for example one read with `get`) and
  override its kind. A kind-preserving method can be added later without breaking callers if
  generic tooling needs it.
- `RecordInput`, `PatchInput` and `Projection` are replaced. The record crate's internal types
  (`RecordPatch`, `CellPatch`, `RecordCells`) are no longer re-exported.

## 5. `Metered<T>` and `Receipt`

Every record call returns its outcome together with a receipt. The receipt is present on success
and on failure, so a host can charge for failed calls (a transaction keeps its fee through
`OutOfBudget`). EVM implementations do the same: in revm every outcome carries `gas_used`.

```rust
#[must_use]
pub struct Metered<T> {
    pub result: Result<T, ApiError>,
    pub receipt: Receipt,
}

#[non_exhaustive]
pub struct Receipt {
    pub cost: u64,              // 0 until metering is implemented
    pub priced_at: Option<CommitId>, // writes and branch reads: the branch's base; head reads: the
                                     // snapshot's head; get at a commit: that commit. None only for
                                     // an unknown or consumed handle or a failed head read
    pub details: Details,
}

#[non_exhaustive]
pub struct Details {            // effects on live state, user cells only (D4)
    pub cells_created: u64,
    pub cells_updated: u64,
    pub cells_deleted: u64,
    pub index_joins: u64,
    pub index_leaves: u64,
    pub cell_bytes_written: u64,
    pub cell_bytes_deleted: u64,
    pub index_bytes_written: u64,
    pub index_bytes_deleted: u64,
}
```

- **Details are always present.** The metering spec makes them opt-in per call (R9) to keep receipts
  small on a transport; in the Rust API they are a small fixed struct computed anyway. The docs say
  they may become optional in a later version.
- **Details describe effects, not charges.** A set to an identical value changes nothing and counts
  nothing; a failed call applied nothing, so its details are all zero. Reads report zero details.
  Arkiv computes lifetime fees from the bytes written and deleted.
- **Not yet in `Details`:** index terms created. Whether a join creates a new term needs an index
  lookup at call time, which the record layer does not do today. `Details` is `#[non_exhaustive]`,
  so the field can follow.
- **Budget:** `RecordOp::budget(n)` is accepted and stored. Until metering exists, cost is 0, so a
  budget is never exceeded; the docs say so on `budget`. `ApiError::OutOfBudget` exists for the same
  reason and is not returned yet; when it is, the amount spent is the receipt's `cost`.
- **Not yet in `Receipt`:** the per-op-class ledger. It needs the metering counting rules (D2, D3)
  and comes with metering, as an additional field.
- `Metered::into_result()` drops the receipt for callers that do not charge:
  `db.get(target, RecordOp::get(k)).into_result()?`. Rust's `?` only works on `Result`, so either
  `.into_result()?` or `.result?` is needed.
- Branch operations (`begin`, `checkpoint`, `rollback`, `seal`, `commit`, `discard`), `head` and
  `branch_info` are unmetered and keep `Result<T>`. The immutable-data calls also keep `Result<T>`
  until the metering spec covers them (its finding F2).

## 6. Before and after

```rust
// before
fn create(&self, branch: BranchId, key: RecordKey, cells: RecordInput) -> Result<RecordKey>;
fn get(&self, target: ReadTarget, key: RecordKey, projection: Projection) -> Result<Record>;
fn patch(&self, branch: BranchId, key: RecordKey, patch: PatchInput) -> Result<()>;
fn delete(&self, branch: BranchId, key: RecordKey) -> Result<()>;

// after
fn create(&self, branch: BranchId, op: RecordOp<Create>) -> Metered<RecordKey>;
fn get(&self, target: ReadTarget, op: RecordOp<Get>) -> Metered<Record>;
fn patch(&self, branch: BranchId, op: RecordOp<Patch>) -> Metered<()>;
fn delete(&self, branch: BranchId, op: RecordOp<Delete>) -> Metered<()>;
```

```rust
// before
db.create(branch, key, RecordInput::new()
    .attribute("price", CellValue::from_i32(50))?
    .field("description", CellValue::from_str("A product"))?)?;

// after: caller-assigned keys
db.create(branch, RecordOp::create().key(k)
    .attribute("price", 50i32)
    .field("description", "A product")).into_result()?;

// after: generated keys, and a host that charges
let m = db.create(branch, RecordOp::create().attribute("price", 50i32));
charge(m.receipt.cost, &m.receipt.details);
let key = m.result?;

db.patch(branch, RecordOp::patch(key).attribute("price", 75i32).remove("description"))
    .into_result()?;
let record = db.get(ReadTarget::Head, RecordOp::get(key).only(["price"])).into_result()?;
db.delete(branch, RecordOp::delete(key)).into_result()?;
```

## 7. Implementation plan

Three commit groups, in this order; each builds and passes all tests on its own.

**Group 1: `RecordOp` and `Metered`**
1. `IntoCellValue` in `golemdb-cells` for integers, strings, bytes, booleans, floats (fallible) and
   `CellValue`.
2. `RecordOp<Op>` in `golemdb-api` with the methods of section 4, collecting the first error.
3. The record layer returns the effects of each write (section 5 details). `patch` already reads each
   affected cell's old value, so byte deltas need no extra reads; `delete` already scans the record.
4. `Metered<T>`, `Receipt`, `Details`; `ApiError::OutOfBudget`; the `Api` trait's four record calls
   change signature; `Inner` builds receipts (`cost` 0, `priced_at` from the branch or the target).
5. Tests: a details table per mutation path (create, set new / changed / identical, remove present /
   absent, delete), details zero on every error path, receipts present on failures, compile-fail
   checks for the typestate rules (doctests with `compile_fail`).
6. Migrate the tests, the consumer mock (`Metered::unmetered(result, commit)`), README and doc
   examples; uses of `RecordInput::insert` and `PatchInput::set` become explicit `attribute` /
   `field`; drop `RecordInput`, `PatchInput`, `Projection` and the internal re-exports.

**Group 2: empty records and `#meta`**
1. `reserved::META` (`#meta`) and its encoding/decoding in `golemdb-cells`.
2. `create` allows zero user cells and writes `#meta`; `patch` may remove the last user cell and
   updates `#meta` from the same effects as the receipt details; `delete` removes it.
3. `get` returns `#meta` with the full record.
4. Tests: after every mutation path and after rollback, `#meta` equals a recount of the record's
   user cells (a helper used throughout the record tests); empty create, emptying patch, delete of an
   empty record.

**Group 3: key modes**
1. `Genesis` gains `record_keys` (`CallerAssigned` | `Generated { seed }`), required in YAML;
   `Genesis::DEV` is caller-assigned.
2. Genesis writes `#keyMode` and, when generated, `#keySeed`; reopening validates them like the
   other `#params` cells.
3. The record layer reads the mode, checks the op's key against it (`KeyModeMismatch` in
   `RecordError` and `ApiError`) and derives generated keys.
4. Tests: both modes end to end, mismatches in both directions, determinism (the same seed and
   operations give the same keys on two databases), different seeds give different keys, YAML forms
   (missing mode rejected), genesis identity changes with mode and seed.

Verification for every group: build with and without features, the api crate's feature powerset,
workspace tests and doctests, benches, and the README examples run verbatim.

## 8. Spec impact

For the spec branch, to be added to [naming-migration.md](naming-migration.md) or a follow-up:

- **API spec:** `create` takes an optional key decided by the mode, as today, but the modes are
  named `CallerAssigned` / `Generated` and fixed in genesis; the receipt shape (`Metered` is the
  Rust form of "value plus receipt; errors still report cost"); details always returned in the Rust
  API; empty records (the spec branch already allows them).
- **Design §3/§4:** `#meta` layout and the completeness argument; key modes with `#keyMode`,
  `#keySeed` and the derivation, including why the modes are exclusive.
- **D09 / Appendix A:** the `#meta` encoding and type tag; the key derivation and its domain tag.
- **Metering spec:** `Details` matches R9/D4 except index terms created (later); budget and
  `OutOfBudget` exist in the API before enforcement.

## 9. Implementation notes

Where the implementation adds to or deviates from sections 1–7:

- **Operation markers** live in a module: `RecordOp<op::Create>`, `op::Patch`, `op::Get`,
  `op::Delete`, so generic names like `Get` stay out of the crate root. Callers rarely write them;
  inference picks them from the constructor.
- **`KeyModeMismatch` exists from Group 1:** before key modes, a create without a key was already
  a mismatch with the (implicit) caller-assigned mode, so no interim error was needed.
- **Accessors for mocks:** `RecordOp::record_key()`, `max_cost()`, `value(name)` (create and
  patch) and `removes(name)` (patch), and `Metered::unmetered(result, commit)`.
- **`RecordMeta`** is a public type (re-exported by `golemdb-api`) with `to_value` /
  `from_value`, and `Record::meta()` decodes `#meta` from a full read or from
  `RecordOp::get(key).only(["#meta"])`, so clients can check completeness or the delete bound.
- **`Details` and `RecordMeta` share one counting path:** the record layer computes a write's
  effects once; the receipt reports them and `#meta` applies them. A patch writes `#meta` only
  when it changed, so no-op patches still add no undo entries.
- **Strictness:** a user record without `#meta`, a count that would fall below zero, a missing or
  invalid `#keyMode` / `#keySeed`, and a generated key that is already bound are reported as
  corrupt state, not repaired.
- **`Branches::hasher()`** gives the record layer the deployment's hash function for the key
  derivation; `Records::create_generated` sits next to `Records::create`.
- **The key mode is checked first** in a create, before names, values and allocation, so a
  mismatch writes nothing and allocates no ID.
- **`Genesis` gained a required field** (`record_keys`), so callers building `Genesis { … }` by
  hand must add it (or use `..Genesis::DEV`). `Genesis` will grow further (`#minRetention`,
  segments); making it `#[non_exhaustive]` with a constructor would stop such breaks and is an
  open decision.
- **Every database created before Group 3 fails to reopen** with `GenesisMismatch`, because
  genesis now contains `#keyMode`. Databases are disposable at this stage (review item 1).
