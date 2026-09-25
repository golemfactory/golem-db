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
  traits, bidirectional cursors, range scans, memory and optional MDBX backends
- `crates/index` (package `golemdb-index`) — transactional posting updates across
  bitmap/index tries, ordered terms and scans, canonical chunks, and query bitmaps
- `crates/merkle` (package `golemdb-merkle`) — branch-only persistent Merkle trie,
  canonical compact branches, shared hashing and YAML hash configuration

## Documentation

- [Technical design](docs/golem-db-design.md): storage architecture, data model,
  commitments, history, and query design.
- [API](docs/golem-db-api.md): the caller-facing interface and operation semantics.

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

## CI

See [performance benchmarks](internal_docs/benchmarks.md) for the cells, Merkle and index
Criterion suites, workload filters, timing boundaries and baseline comparisons.

`.github/workflows/ci.yml` builds the workspace, runs the test suite with
[cargo-nextest](https://nexte.st/) across the full feature powerset via
cargo-hack, and checks that benchmarks compile.
