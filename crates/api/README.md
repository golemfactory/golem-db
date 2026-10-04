# Public Rust API

`golemdb-api` defines one synchronous, dyn-compatible `Api` trait:

- Records: create, get, patch, delete.
- Branches: head, begin, branch_info, checkpoint, rollback, seal, commit, discard.

Consumers accept `&dyn Api` or `Arc<dyn Api + Send + Sync>`. The trait does not
impose threading bounds; consumers add them when needed. Adding required methods
requires updating concrete implementations and mocks.

`Database` implements `Api` and hides store and hash-provider types. Clones share
one branch registry, so handles work across clones and threads. The hash is
selected on opening and stays a concrete type within the database. Import
`Api` to use its methods on `Database`.

The examples use persistent MDBX storage. Enable the `mdbx` feature on
`golemdb-api`; `Database::open(path, &config)` opens an MDBX store in that directory.
Use a fresh database directory for the catalogue examples.

```rust
use golemdb_api::{Api, Config, Database, Genesis, ReadTarget, RecordKey, RecordOp};

// Genesis::DEV is for development and examples; deployments load their own genesis.
let db = Database::open("./golemdb-quickstart", &Config::new(Genesis::DEV))?;
let branch = db.begin()?;
let key = RecordKey([0x42; 32]);
db.create(branch, RecordOp::create().key(key)
    .attribute("price", 50i32)
    .field("description", "A product")).into_result()?;
let pending = db.clone().get(ReadTarget::Branch(branch), RecordOp::get(key)).into_result()?;
db.commit(branch)?;
assert_eq!(db.get(ReadTarget::Head, RecordOp::get(key)).into_result()?, pending);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Only `commit` publishes changes. Dropping the last clone discards pending work;
committed MDBX state survives reopening. Handles belong to one open database:
separate opens over a shared store have separate registries. Use `clone` to share
handles.

## Record operations and receipts

Each record call takes a `RecordOp` built for it: `RecordOp::create()`,
`RecordOp::patch(key)`, `RecordOp::get(key)` or `RecordOp::delete(key)`. The
operation decides which methods exist: only a create takes `.key(k)`, only a patch
can `.remove(name)`, only a get can narrow its cells with `.only([names])`. Every call
returns a `Metered` outcome: the result together with a receipt.

```rust
use golemdb_api::{Api, Database, Genesis, RecordKey, RecordOp};

let db = Database::open_memory(&Genesis::DEV)?;
let branch = db.begin()?;
let key = RecordKey([1; 32]);
let created = db.create(branch, RecordOp::create().key(key)
    .attribute("price", 50i32)
    .field("description", "A product"));
// The receipt is there whether the call succeeded or failed.
assert_eq!(created.receipt.cost, 0); // not metered yet
assert_eq!(created.receipt.details.cells_created, 2);
created.into_result()?;
db.patch(branch, RecordOp::patch(key).attribute("price", 75i32).remove("description"))
    .into_result()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Every write declares its kind: `attribute` for an indexed cell, `field` for a stored
one. Values come from Rust types, and the type decides the cell type, so write
`50i32` or `50i64` explicitly; a `CellValue` can be passed too and keeps its type.
Builder steps never fail: the first invalid name, duplicate name (including a set and
a removal of the same name) or invalid value is reported by the call as
`InvalidArgument`. Deployment limits and record rules are checked by the call as well.
A create must name its key: every database uses caller-assigned keys today, and a
create without one fails with `KeyModeMismatch`.

`Metered::into_result()` drops the receipt for callers that do not charge; Rust's `?`
works only on `Result`, so use `.into_result()?` or `.result?`. A `Receipt` has:

- `cost`: always 0 until metering is implemented.
- `priced_at`: the commit whose cost schedule prices the call: a write's or branch
  read's base commit, or the commit a read targets.
- `details`: the call's effects on user cells, as in the metering spec: cells
  created, updated and deleted, index joins and leaves, and cell and index bytes
  written and deleted. Effects, not requests: setting a cell to its current value
  counts nothing, and failed calls and reads report zero. Details are always present
  in this version; a later version may make them optional per call.

`RecordOp::budget(n)` caps a call's cost. It is not enforced yet: every call costs 0
until metering is implemented, so `ApiError::OutOfBudget` is not returned yet.

`ReadTarget::Branch(id)` reads work in progress. `ReadTarget::Head` resolves head
and reads the record in one storage snapshot. `ReadTarget::Commit(id)` currently
accepts only that snapshot's head; historical reads are deferred.
A get reads all cells unless narrowed with `.only([names])`, which accepts text or raw
byte names for reserved records; an empty list still checks that the record exists.

`ApiError` exposes common failures directly and retains diagnostic sources for
invalid input and internal failures. A live stale branch's commit returns
`Conflict` and consumes the handle. Subsequent calls, unknown handles, and handles
already invalidated by another operation return `HandleInvalid`. `SealInfo`
contains only the candidate commit ID and the state/index roots.

## Example: two commits over a small product catalogue

This complete example creates three records, reads committed data, then changes
the catalogue in a second branch. The second commit updates a price, adds a cell,
removes a cell, creates a record, and deletes another record. Prices are integer
cents, and every record has an explicit caller-provided key. `CreateNew` requires
uninitialized storage at `./golemdb-catalogue`, so the two commits start at 1 and 2.
Use a different fresh directory to run this example again.

Both commits explicitly call `seal` to compute their roots before publication.
Sealing is optional (`commit` seals automatically if needed). It freezes branch
record access without advancing the head or reserving the candidate commit ID;
another branch can still win the commit race.

```rust
use golemdb_api::{
    Api, ApiError, CellLimits, Config, Database, Genesis, HashAlgorithm, OpenMode,
    ReadTarget, RecordKey, RecordOp,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::new(Genesis {
        hash_function: HashAlgorithm::Keccak256,
        cell_limits: CellLimits {
            max_cell_name_len: 32,
            max_str_len: 128,
            max_bytes_len: 1024,
        },
    });
    config.mode = OpenMode::CreateNew;
    let db = Database::open("./golemdb-catalogue", &config)?;
    let laptop = RecordKey([1; 32]);
    let keyboard = RecordKey([2; 32]);
    let mouse = RecordKey([3; 32]);
    let monitor = RecordKey([4; 32]);

    // 1. Create three records in one branch and publish them together.
    let branch = db.begin()?;
    for (key, name, price_cents, description) in [
        (laptop, "Laptop", 120_000, "14-inch laptop"),
        (keyboard, "Keyboard", 8_000, "Mechanical keyboard"),
        (mouse, "Mouse", 3_000, "Wireless mouse"),
    ] {
        db.create(
            branch,
            RecordOp::create()
                .key(key)
                .attribute("name", name)
                .attribute("price_cents", price_cents)
                .field("description", description),
        )
        .into_result()?;
    }
    // Seal computes roots and freezes the branch; nothing is published yet.
    let first_seal = db.seal(branch)?;
    assert_eq!(first_seal.commit_id, 1);
    assert_eq!(db.head()?, 0);
    assert!(db.branch_info(branch)?.sealed);
    let first_commit = db.commit(branch)?;
    assert_eq!(first_commit, first_seal.commit_id);
    assert_eq!(first_commit, 1); // Opening created genesis at commit 0.

    // 2. Read a full record and a projection from committed state.
    let saved_laptop = db.get(ReadTarget::Head, RecordOp::get(laptop)).into_result()?;
    assert_eq!(saved_laptop.cells[b"price_cents".as_slice()].as_i32(), Some(120_000));
    assert_eq!(saved_laptop.cells[b"#key".as_slice()].as_bytes32(), Some(laptop.0));

    // An explicit commit target is supported while that commit is the head.
    let saved_keyboard = db
        .get(
            ReadTarget::Commit(first_commit),
            RecordOp::get(keyboard).only(["name", "price_cents"]),
        )
        .into_result()?;
    assert_eq!(saved_keyboard.cells.len(), 2);
    assert_eq!(saved_keyboard.cells[b"name".as_slice()].as_str(), Some("Keyboard"));

    // 3. Start a new branch: change a price, add stock, remove a description,
    //    add a monitor record, and delete the mouse record.
    let branch = db.begin()?;
    db.patch(
        branch,
        RecordOp::patch(laptop)
            .attribute("price_cents", 110_000i32)
            .field("stock", 5u32)
            .remove("description"),
    )
    .into_result()?;
    db.create(
        branch,
        RecordOp::create()
            .key(monitor)
            .attribute("name", "Monitor")
            .attribute("price_cents", 30_000i32)
            .field("description", "27-inch monitor"),
    )
    .into_result()?;
    db.delete(branch, RecordOp::delete(mouse)).into_result()?;

    // Branch reads see the changes immediately; head still contains commit 1.
    let pending_laptop = db.get(ReadTarget::Branch(branch), RecordOp::get(laptop)).into_result()?;
    assert_eq!(pending_laptop.cells[b"price_cents".as_slice()].as_i32(), Some(110_000));
    assert_eq!(db.get(ReadTarget::Head, RecordOp::get(laptop)).into_result()?, saved_laptop);
    assert!(matches!(
        db.get(ReadTarget::Branch(branch), RecordOp::get(mouse)).into_result(),
        Err(ApiError::NotFound)
    ));
    assert!(db.get(ReadTarget::Head, RecordOp::get(mouse)).into_result().is_ok());

    // Finish pending reads before sealing: a sealed branch rejects record
    // reads/writes, checkpoints, and rollback. Head reads remain available.
    let second_seal = db.seal(branch)?;
    println!("Candidate commit: {}", second_seal.commit_id);
    println!("State root: {:02x?}", second_seal.state_root);
    println!("Index root: {:02x?}", second_seal.index_root);
    assert_eq!(db.head()?, first_commit);
    assert_eq!(db.get(ReadTarget::Head, RecordOp::get(laptop)).into_result()?, saved_laptop);
    assert_eq!(db.seal(branch)?, second_seal); // Repeated seal returns the same result.

    // Commit publishes the sealed changes and consumes the branch handle.
    let second_commit = db.commit(branch)?;
    assert_eq!(second_commit, second_seal.commit_id);
    assert_eq!(second_commit, 2);
    assert_eq!(db.head()?, second_commit);

    // 4. Read again: laptop changed, keyboard stayed the same, monitor appeared,
    //    and mouse disappeared. Use Head now; historical commit 1 is deferred.
    let updated_laptop = db.get(ReadTarget::Head, RecordOp::get(laptop)).into_result()?;
    assert_eq!(updated_laptop, pending_laptop);
    assert_eq!(updated_laptop.cells[b"stock".as_slice()].as_u32(), Some(5));
    assert!(!updated_laptop.cells.contains_key(b"description".as_slice()));

    let unchanged_keyboard = db.get(ReadTarget::Head, RecordOp::get(keyboard).only(["name", "price_cents"])).into_result()?;
    assert_eq!(unchanged_keyboard, saved_keyboard);

    let new_monitor = db.get(ReadTarget::Head, RecordOp::get(monitor)).into_result()?;
    assert_eq!(new_monitor.cells[b"name".as_slice()].as_str(), Some("Monitor"));
    assert_eq!(new_monitor.cells[b"price_cents".as_slice()].as_i32(), Some(30_000));
    assert!(matches!(
        db.get(ReadTarget::Head, RecordOp::get(mouse)).into_result(),
        Err(ApiError::NotFound)
    ));

    Ok(())
}
```

## Opening and genesis

Provide deployment settings explicitly, as `Genesis` or a caller-selected YAML file:

```yaml
hash_function: keccak-256
cell_limits:
  max_cell_name_len: 32
  max_str_len: 64
  max_bytes_len: 128
```

`blake3` is also supported. Missing, duplicate, and unknown fields are rejected.
Use `Genesis::load(path)` or `Genesis::from_yaml(text)`; the loader
does not search for files or read environment overrides.

```rust
use golemdb_api::{Config, Database, Genesis};

let genesis = Genesis::load("genesis.yaml")?;
let config = Config::new(genesis); // CreateIfMissing
let db = Database::open("./golemdb", &config)?;
# Ok::<(), golemdb_api::OpenError>(())
```

There are three constructors:

- `Database::open(store, &config)` opens a built-in store. `store` is anything that
  converts into a `StoreConfig`: a path means MDBX with default options (with the `mdbx`
  feature), `StoreConfig::Memory` a throwaway database, and
  `StoreConfig::Mdbx { path, options }` MDBX with tuned capacity. The default
  `MdbxOptions` cap the store at 1 GiB. A larger `max_map_size` takes effect on reopening, a smaller one is ignored.
- `Database::open_memory(&genesis)` opens a fresh in-memory database, the standard for
  tests. Each call starts empty, so it takes no `OpenMode`.
- `Database::from_store(store, &config)` opens a caller-supplied store, for custom or
  wrapped stores and tests. The store is trusted as supplied, and the path guarantees
  of `open` below do not apply to it.

A node sizes its store explicitly:

```rust
use golemdb_api::{Config, Database, Genesis, MdbxOptions, StoreConfig};

let config = Config::new(Genesis::load("genesis.yaml")?);
let mut options = MdbxOptions::default();
options.max_map_size = 8 << 30; // 8 GiB
let store = StoreConfig::Mdbx { path: "./golemdb-node".into(), options };
let db = Database::open(&store, &config)?;
# Ok::<(), golemdb_api::OpenError>(())
```

The same settings can live in a store file, kept separate from the genesis file:
genesis is identical on every node and may never change, while store settings are
local to a node and may change between restarts.

```yaml
mdbx:
  path: data                     # relative to this file's directory
  options:                       # optional; omitted options keep their defaults
    max_map_size: 8589934592     # 8 GiB
```

`memory` alone selects the in-memory store. Unknown fields are rejected, and errors
are reported as `OpenError::StoreYaml`. `StoreConfig::load(path)` resolves a relative
`path` against the store file's directory; `StoreConfig::from_yaml(text)` has no file
location and leaves it relative to the current directory.

```rust
use golemdb_api::{Config, Database, Genesis, StoreConfig};

let config = Config::new(Genesis::load("genesis.yaml")?);
let db = Database::open(StoreConfig::load("store.yaml")?, &config)?;
# Ok::<(), golemdb_api::OpenError>(())
```

`Config` holds what applies to every store: the `Genesis` and the `OpenMode`. Store
settings are local to a node and never part of the genesis identity. Open an MDBX
directory at most once per process and share it with `clone`; another process may
open it too, with separate branches and `Conflict` for the losing commit. A store
should back one open database at a time; reopening it after the database is dropped
is fine.

| Mode | Pristine storage | Initialized storage |
| --- | --- | --- |
| `CreateIfMissing` (default) | Create commit 0 | Validate and reopen |
| `ExistingOnly` | `NotInitialized` | Validate and reopen |
| `CreateNew` | Create commit 0 | `AlreadyInitialized` after validation |

Pristine means no tables or rows. A pre-existing empty directory is allowed for
creation. Tables without a head, including empty or foreign tables, are rejected
as incomplete state. `ExistingOnly` never writes genesis; through `Database::open` it
also never creates a missing directory, but opening an existing empty directory can
create MDBX environment files.

All detection, validation, and initialization occurs under one storage writer.
Creation writes the fixed reserved-record catalogue from cells, identities and
bindings, the configured limits, allocator at 64, and a real cell trie. Every
genesis cell is a field, so the index root is empty. The transaction writes format,
hash, Roaring, genesis identity, and head metadata before publishing commit 0.
`#roots` and `#rootIndex` contain their identity cells but no history entries.
The two metering records have identities and bindings only; metering is deferred.

The opener validates physical store ceilings, including complete cell/index
keys and value tags. Format 1 also requires room for 16 KiB internal values, covering
trie nodes and canonical bitmap containers. Limits are user-cell admission policy;
system root-history bytes do not consume the configured user bytes allowance.

Reopening compares the canonical genesis identity, validates format IDs, required
reserved cells, allocator, current roots, and previous-root history. Required
cells are checked against their committed trie paths. This is startup validation,
not a full audit of every user row or index subtree. It never rewrites parameters
or resets allocation. Formatting or field order in YAML does not affect identity;
changed deployment settings return `GenesisMismatch`.

`OpenError` distinguishes configuration, format, initialization, and storage
failures. `Database::info()` describes the opening snapshot; call `head()` for the
live head, or `get(ReadTarget::Head, ...)` for a consistent current record read.

Ordinary consumers use the three `Database` constructors and the `Api` methods.
Trusted tooling and this crate's own tests can enable the `internals` feature for
the lower-level layer: `open_store(store, &config)` validates or initializes a store
and returns an `OpenedStore`, whose `into_database()` opens the database and whose
`into_store()` hands the validated store back to trusted library code.

## Current scope

Immutable data has an interface only. `Api` exposes:

- `immutable_data_append(branch, segment, key, row) -> ordinal`, where `key` is
  `Option<ImmutableDataKey>` and `row` is `Vec<Vec<u8>>` (one byte array per column).
- `immutable_data_get(segment, address) -> row`, with
  `ImmutableDataAddress::Ordinal(u64)` or `ImmutableDataAddress::Key(key)`.
- `immutable_data_range_of(segment, commit) -> Range<u64>`.
- `immutable_data_rows_of(segment, commit) -> Vec<ImmutableDataRow>`.

`ImmutableDataKey` contains 32 caller-provided bytes and is distinct from a record
key. The intended contract makes keys unique within a segment, with rows and key
bindings published together from a sealed branch at commit. Keys are optional;
all rows retain ordinal addressing. Duplicate keys will never overwrite rows.

**Every immutable-data call currently returns `ApiError::NotImplemented { operation }`
immediately**, including calls with invalid handles or unknown segments. Nothing
is validated, staged, allocated, read, or written. Segment configuration, files,
key indexes, and commit integration are deferred. The
[immutable-data specification](../../docs/golem-db-api.md#immutable-data) describes
the future behavior, including pruning and rewind of key bindings.

All four iterations are implemented: typed cells, the facade contract and builders,
atomic opening/genesis, and a cloneable `Database` with shared branch state.
Metering (real costs, budget enforcement, the ledger), generated keys, OCC, and transport
serialization are deferred; receipts already report each call's effects.

The [consumer test](tests/consumer.rs) demonstrates a single facade mock and
commit failure injection behind `Arc<dyn Api + Send + Sync>`. Use scripted mocks
for consumer error paths. For tests, replace the storage opening with
`Database::open_memory(&genesis)?` to get a fresh in-memory database (`Genesis::DEV` for tests that need no particular limits). Arkiv can pass `Arc::new(Database::open_memory(&genesis)?)`
as `Arc<dyn Api + Send + Sync>` to run behavioral tests with real database semantics.
The [facade contract tests](tests/facade.rs) run the same scenarios against memory
and MDBX with both hash algorithms. The [error tests](tests/facade_errors.rs)
exercise store diagnostics and retries through the real facade.
CLI/HTTP/TCP adapters own transport parsing and serialization.
