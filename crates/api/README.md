# Public Rust API

`golemdb-api` defines one synchronous, dyn-compatible `Api` trait:

- Records: create, get, patch, delete.
- Branches: head, begin, branch_info, checkpoint, rollback, seal, commit, discard.

Consumers accept `&dyn Api` or `Arc<dyn Api + Send + Sync>`. The trait does not
impose threading bounds; consumers add them when needed. Adding required methods
requires updating concrete implementations and mocks.

`Database` implements `Api` and hides store and hash-provider types. Clones share
one engine and branch registry, so handles work across clones and threads. The
hash is selected on opening and stays a concrete type within the engine. Import
`Api` to use its methods on `Database`.

The examples use persistent MDBX storage. Enable the `mdbx` feature on
`golemdb-api`; `Database::open_database(path, &config)` opens the store internally.
Use a fresh database directory for the catalogue examples.

```rust
use golemdb_api::{Api, CellLimits, CellValue, Database, GenesisConfig,
    HashAlgorithm, OpenConfig, Projection, ReadTarget, RecordInput, RecordKey};

let config = OpenConfig::new(GenesisConfig {
    hash_function: HashAlgorithm::Keccak256,
    cell_limits: CellLimits {
        max_cell_name_len: 32, max_str_len: 64, max_bytes_len: 128,
    },
});
let db = Database::open_database("./golemdb-quickstart", &config)?;
let branch = db.begin()?;
let key = RecordKey([0x42; 32]);
db.create(branch, key, RecordInput::new()
    .attribute("price", CellValue::from_i32(50))?
    .field("description", CellValue::from_str("A product"))?)?;
let pending = db.clone().get(ReadTarget::Branch(branch), key, Projection::All)?;
db.commit(branch)?;
assert_eq!(db.get(ReadTarget::Head, key, Projection::All)?, pending);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Only `commit` publishes changes. Dropping the last clone discards pending work;
committed MDBX state survives reopening. Handles belong to one engine: separate
opens over shared storage have separate registries. Use `clone` to share handles.

## Inputs and reads

```rust
use golemdb_api::{CellValue, PatchInput, Projection, RecordInput};

let record = RecordInput::new()
    .attribute("price", CellValue::from_i32(50))?
    .field("description", CellValue::from_str("A product"))?;
let patch = PatchInput::new()
    .attribute("price", CellValue::from_i32(75))?
    .remove("description")?;
let projection = Projection::only(["price", "#key"]);
# Ok::<(), golemdb_api::ApiError>(())
```

Explicit cell constructors own value encoding. The input helpers choose field or
attribute kind; `RecordInput::insert` and `PatchInput::set` preserve the supplied
value's kind. Builders reject invalid names, unindexable attributes, and duplicate
names, including a set and removal for the same name. Conversion from existing
record maps checks name syntax. Deployment limits and record invariants remain
execution-time checks in record.

`ReadTarget::Branch(id)` reads work in progress. `ReadTarget::Head` resolves head
and reads the record in one storage snapshot. `ReadTarget::Commit(id)` currently
accepts only that snapshot's head; historical reads are deferred.
`Projection::All` reads all cells. `Projection::only` accepts text or raw byte names
for reserved records; an empty projection still checks existence.

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
    Api, ApiError, CellLimits, CellValue, Database, GenesisConfig, HashAlgorithm,
    OpenConfig, OpenMode, PatchInput, Projection, ReadTarget, RecordInput, RecordKey,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = OpenConfig::new(GenesisConfig {
        hash_function: HashAlgorithm::Keccak256,
        cell_limits: CellLimits {
            max_cell_name_len: 32,
            max_str_len: 128,
            max_bytes_len: 1024,
        },
    });
    config.mode = OpenMode::CreateNew;
    let db = Database::open_database("./golemdb-catalogue", &config)?;
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
            key,
            RecordInput::new()
                .attribute("name", CellValue::from_str(name))?
                .attribute("price_cents", CellValue::from_i32(price_cents))?
                .field("description", CellValue::from_str(description))?,
        )?;
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
    let saved_laptop = db.get(ReadTarget::Head, laptop, Projection::All)?;
    assert_eq!(saved_laptop.cells[b"price_cents".as_slice()].as_i32(), Some(120_000));
    assert_eq!(saved_laptop.cells[b"#key".as_slice()].as_bytes32(), Some(laptop.0));

    // An explicit commit target is supported while that commit is the head.
    let saved_keyboard = db.get(
        ReadTarget::Commit(first_commit),
        keyboard,
        Projection::only(["name", "price_cents"]),
    )?;
    assert_eq!(saved_keyboard.cells.len(), 2);
    assert_eq!(saved_keyboard.cells[b"name".as_slice()].as_str(), Some("Keyboard"));

    // 3. Start a new branch: change a price, add stock, remove a description,
    //    add a monitor record, and delete the mouse record.
    let branch = db.begin()?;
    db.patch(
        branch,
        laptop,
        PatchInput::new()
            .attribute("price_cents", CellValue::from_i32(110_000))?
            .field("stock", CellValue::from_u32(5))?
            .remove("description")?,
    )?;
    db.create(
        branch,
        monitor,
        RecordInput::new()
            .attribute("name", CellValue::from_str("Monitor"))?
            .attribute("price_cents", CellValue::from_i32(30_000))?
            .field("description", CellValue::from_str("27-inch monitor"))?,
    )?;
    db.delete(branch, mouse)?;

    // Branch reads see the changes immediately; head still contains commit 1.
    let pending_laptop = db.get(ReadTarget::Branch(branch), laptop, Projection::All)?;
    assert_eq!(pending_laptop.cells[b"price_cents".as_slice()].as_i32(), Some(110_000));
    assert_eq!(db.get(ReadTarget::Head, laptop, Projection::All)?, saved_laptop);
    assert!(matches!(
        db.get(ReadTarget::Branch(branch), mouse, Projection::All),
        Err(ApiError::NotFound)
    ));
    assert!(db.get(ReadTarget::Head, mouse, Projection::All).is_ok());

    // Finish pending reads before sealing: a sealed branch rejects record
    // reads/writes, checkpoints, and rollback. Head reads remain available.
    let second_seal = db.seal(branch)?;
    println!("Candidate commit: {}", second_seal.commit_id);
    println!("State root: {:02x?}", second_seal.state_root);
    println!("Index root: {:02x?}", second_seal.index_root);
    assert_eq!(db.head()?, first_commit);
    assert_eq!(db.get(ReadTarget::Head, laptop, Projection::All)?, saved_laptop);
    assert_eq!(db.seal(branch)?, second_seal); // Repeated seal returns the same result.

    // Commit publishes the sealed changes and consumes the branch handle.
    let second_commit = db.commit(branch)?;
    assert_eq!(second_commit, second_seal.commit_id);
    assert_eq!(second_commit, 2);
    assert_eq!(db.head()?, second_commit);

    // 4. Read again: laptop changed, keyboard stayed the same, monitor appeared,
    //    and mouse disappeared. Use Head now; historical commit 1 is deferred.
    let updated_laptop = db.get(ReadTarget::Head, laptop, Projection::All)?;
    assert_eq!(updated_laptop, pending_laptop);
    assert_eq!(updated_laptop.cells[b"stock".as_slice()].as_u32(), Some(5));
    assert!(!updated_laptop.cells.contains_key(b"description".as_slice()));

    let unchanged_keyboard = db.get(
        ReadTarget::Head,
        keyboard,
        Projection::only(["name", "price_cents"]),
    )?;
    assert_eq!(unchanged_keyboard, saved_keyboard);

    let new_monitor = db.get(ReadTarget::Head, monitor, Projection::All)?;
    assert_eq!(new_monitor.cells[b"name".as_slice()].as_str(), Some("Monitor"));
    assert_eq!(new_monitor.cells[b"price_cents".as_slice()].as_i32(), Some(30_000));
    assert!(matches!(
        db.get(ReadTarget::Head, mouse, Projection::All),
        Err(ApiError::NotFound)
    ));

    Ok(())
}
```

## Opening and genesis

Provide deployment settings explicitly, as `GenesisConfig` or a caller-selected YAML file:

```yaml
hash_function: keccak-256
cell_limits:
  max_cell_name_len: 32
  max_str_len: 64
  max_bytes_len: 128
```

`blake3` is also supported. Missing, duplicate, and unknown fields are rejected.
Use `GenesisConfig::load(path)` or `GenesisConfig::from_yaml(text)`; the loader
does not search for files or read environment overrides.

```rust
use golemdb_api::{Database, GenesisConfig, OpenConfig};

let genesis = GenesisConfig::load("genesis.yaml")?;
let config = OpenConfig::new(genesis); // CreateIfMissing
let db = Database::open_database("./golemdb", &config)?;
# Ok::<(), golemdb_api::OpenError>(())
```

With the `mdbx` feature, `Database::open_database(path, &config)` opens persistent
storage (`Database::open` remains an alias). `Database::open_with_options` additionally
accepts local `MdbxOptions`. For a custom store or a shared store instance,
use `Database::from_store(store, &config)`.
Do not independently open the same MDBX directory twice within one process.

| Mode | Pristine storage | Initialized storage |
| --- | --- | --- |
| `CreateIfMissing` (default) | Create commit 0 | Validate and reopen |
| `ExistingOnly` | `NotInitialized` | Validate and reopen |
| `CreateNew` | Create commit 0 | `AlreadyInitialized` after validation |

Pristine means no tables or rows. A pre-existing empty directory is allowed for
creation. Tables without a head, including empty or foreign tables, are rejected
as incomplete state. `ExistingOnly` never writes genesis or creates a missing
directory; opening an existing empty directory can create MDBX environment files.

All detection, validation, and initialization occurs under one storage writer.
Creation writes the fixed reserved-record catalogue from cells, identities and
bindings, the configured limits, allocator at 64, and a real cell trie. Every
genesis cell is a field, so the index root is empty. The transaction writes format,
hash, Roaring, genesis identity, and head metadata before publishing commit 0.
`#roots` and `#rootIndex` contain their identity cells but no history entries.
The two metering records have identities and bindings only; metering is deferred.

The opener validates physical store ceilings, including complete cell/index
keys and value tags. Format 1 also requires room for 16 KiB engine values, covering
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

The free opening functions remain lower-level setup helpers returning
`OpenedStore`: `open_database` accepts an MDBX path, while `open_store`
accepts a store instance. Use `into_database()` to consume one as a facade, or
`into_store()` to hand validated storage to trusted engine code. Ordinary
consumers use the associated `Database` constructors and the `Api` methods.

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
Budgets, cost receipts, generated keys, OCC, and transport serialization are deferred.

The [consumer test](tests/consumer.rs) demonstrates a single facade mock and
commit failure injection behind `Arc<dyn Api + Send + Sync>`. Use scripted mocks
for consumer error paths. For tests, replace the storage opening with
`Database::open_memory(&config)?` to get a fresh in-memory database. Arkiv can pass `Arc::new(Database::open_memory(&config)?)`
as `Arc<dyn Api + Send + Sync>` to run behavioral tests with real database semantics.
The [facade contract tests](tests/facade.rs) run the same scenarios against memory
and MDBX with both hash algorithms. The [error tests](tests/facade_errors.rs)
exercise store diagnostics and retries through the real facade.
CLI/HTTP/TCP adapters own transport parsing and serialization.
