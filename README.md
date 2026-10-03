# golemdb

A Cargo workspace for GolemDB. Each crate lives under `crates/` and is
registered as both a workspace member and a `[workspace.dependencies]` path
entry in the root [Cargo.toml](Cargo.toml), so crates depend on each other via
`{ workspace = true }` rather than repeating paths/versions.

## Layout

- `docs/` - project-wide design and API documentation
- `crates/cells` (package `golemdb-cells`) — the wire format for "Cells",
  GolemDB's typed value encoding
- `crates/storage` (package `golemdb-storage`) — transactional ordered key/value
  traits, bidirectional cursors, range scans, memory and optional MDBX store implementations
- `crates/index` (package `golemdb-index`) — transactional posting updates across
  bitmap/index tries, ordered terms and scans, canonical chunks, and query bitmaps
- `crates/merkle` (package `golemdb-merkle`) — branch-only persistent Merkle trie,
  canonical compact branches, shared hashing and YAML hash configuration
- `crates/branch` (package `golemdb-branch`) — head-validated branch IDs and guarded cell
  read/write views, encoded-key prefix scans, atomic writes, checkpoints and undo;
  seal buffers cell/index updates and roots; commit atomically persists them and
  advances head, with history deferred
- `crates/record` (package `golemdb-record`) — caller-keyed record CRUD, allocation,
  identity and bindings; branch work-in-progress and committed-head reads
  ([scope and usage](crates/record/README.md))
- `crates/api` (package `golemdb-api`) — one public `Api` trait implemented by the
  cloneable `GolemDb` facade, explicit input builders, projections, errors, and
  atomic memory/MDBX opening from typed or YAML genesis
  ([scope and usage](crates/api/README.md))

## Documentation

- [Technical design](docs/golem-db-design.md): storage architecture, data model,
  commitments, history, and query design.
- [API](docs/golem-db-api.md): the caller-facing interface and operation semantics.
- [API implementation plan](docs/api-implementation-plan.md): iterations for the
  public Rust facade, typed inputs, and database opening.

## Adding a crate

1. `cargo new --lib crates/<name>`
2. Add it to `members` and `[workspace.dependencies]` in the root
   `Cargo.toml`.
3. In the new crate's `Cargo.toml`, set `version.workspace = true`,
   `edition.workspace = true`, `rust-version.workspace = true`,
   `license.workspace = true`.

## Build

```sh
cargo build --workspace
cargo nextest run --workspace
cargo bench --workspace --no-run  # compile benches without running them
```

To run the test suite across every feature combination a crate defines (e.g.
`golemdb-cells`'s `custom_types`), use
[cargo-hack](https://github.com/taiki-e/cargo-hack):

```sh
cargo hack nextest run --workspace --feature-powerset
```
