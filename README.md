# golemdb

A Cargo workspace for GolemDB. Each crate lives under `crates/` and is
registered as both a workspace member and a `[workspace.dependencies]` path
entry in the root [Cargo.toml](Cargo.toml), so crates depend on each other via
`{ workspace = true }` rather than repeating paths/versions.

## Layout

- `crates/cells` (package `golemdb-cells`) — the wire format for "Cells",
  GolemDB's typed value encoding
- `crates/indexes` — indexing structures, built on `golemdb-cells`

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

## CI

`.github/workflows/ci.yml` builds the workspace, runs the test suite with
[cargo-nextest](https://nexte.st/), and checks that benchmarks compile.
