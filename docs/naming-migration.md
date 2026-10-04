# Naming migration: instructions for the documentation

Instructions for bringing the design documents in line with the naming and API decisions in
[implementation-review.md](implementation-review.md) Part 0 (N1–N21). The code side is done on
branch `matthiaszimmermann/refactor/namings` (PR #29). This file says what to change in the
documents, where, and how to check the result. It is written so that an agent can execute it.

**Where to work.** The documents named here live on `matthiaszimmermann/docs/design-spec-iterations`
(revision `0ade839` when this was written); `golem-db-metering.md`, `CHANGES.md` and
`decisions_for_architect.md` exist only there. Line numbers below refer to that revision. **Match by
the quoted phrase, not by line number**, since the files keep changing.

**Do not change:**
- heading anchors that other documents link to, e.g. design `### What the Engine Enforces`
  (`#what-the-engine-enforces`); the design's "engine" pass is deferred (section 5);
- quoted or historical text: triage-log entries, the *Question* and *Alternatives* parts of
  decision entries, and review evidence describe past states and stay as written;
- code blocks, unless a section below names them.

## 1. Target vocabulary

| Word | Means |
| --- | --- |
| **GolemDB** | the product, in prose (pending sign-off, see section 8) |
| **`golemdb`** | the product as an identifier: repo, packages, file names, domain tags |
| **database** | what a caller opens and talks to; replaces "engine" and "the store" when they mean GolemDB as a whole |
| **store** | the transactional key/value layer underneath (memory or MDBX); replaces "storage engine" and "backend" |
| **store implementation** | one implementation of the store: memory or MDBX |
| **reserved** | records 0–63 and their `#`/`@` names, as opposed to user records and names |
| **internal** | storage rows that are neither user cells nor reserved records: trie nodes, metadata |
| **trusted library code** | privileged code that bypasses checks (the design's §4 term) |

"Engine" is not used in the API spec, the metering spec or the code. The design keeps it for now.

## 2. Code names, for reference

The documents mention none of these today. If an edit needs to name a code item, use the new name.

| Old | New |
| --- | --- |
| `golemdb_storage::Database`, `MemoryDatabase`, `MdbxDatabase` | `Store`, `MemoryStore`, `MdbxStore` |
| `GolemDb` | `Database` |
| `OpenedDatabase<D>`, `into_database()`, `into_golem_db()` | `OpenedStore<S>`, `into_store()`, `into_database()` |
| `open_backend`, `GolemDb::from_backend` | `open_store`, `Database::from_store` |
| `StorageError::Backend` | `StorageError::Implementation` |
| `CellNameRef::parse_engine` | `parse_reserved` |
| `OpenConfig`, `GenesisConfig` | `Config`, `Genesis` (`Genesis::DEV` for development) |
| `Database::open_database`, `open_with_options` | `Database::open(store, &config)` with `StoreConfig` |
| MDBX "map full" as an internal error | `StorageError::Full` / `ApiError::StoreFull` |

## 3. API spec (`golem-db-api.md`)

Every occurrence of "engine" (15), "the store" (2) and the two identifiers. Replace as shown:

| Line | Current text | Replace with |
| --- | --- | --- |
| 62 | `` **`EngineAssigned`** `` … "the store mints keys deterministically" | `` **`Generated`** `` … "the database mints keys deterministically" |
| 64 | "one of the engine's [reserved records]" | "one of the database's [reserved records]" |
| 98 | "has no special engine meaning" | "has no special meaning to the database" |
| 130 | "The engine owns a handful of records" | "The database owns a handful of records" |
| 148 | "**Not engine concerns:**" | "**Not database concerns:**" (keep the link and its anchor on line 149) |
| 195 | "a concept the engine does not have" | "a concept the database does not have" |
| 196 | "reasons the engine cannot see" | "reasons the database cannot see" |
| 247 | "— the engine merkleizes once at commit" | "— the database merkleizes once at commit" |
| 275 | "forbidden in `EngineAssigned`" | "forbidden in `Generated`" |
| 282 | "The engine additionally creates the record's `#key`" | "The database additionally creates the record's `#key`" |
| 380 | "The engine assigns them no meaning and no type." | "The database assigns them no meaning and no type." |
| 428 | "the store never reorders it" | "the database never reorders it" |
| 441 | "**The store performs no statistics-based planning**" | "**The database performs no statistics-based planning**" |
| 499, 508 | `machineId`, "opaque engine-assigned routing hint" | D11 removes this field. If it is gone when you apply this, do nothing. Otherwise rename to `nodeId`, "opaque database-assigned routing hint" |
| 626 | "It changes by upgrading the engine." | "It changes with a new release." ("upgrading the database" could be read as migrating data) |
| 663 | "The engine validates the lifecycle rules" | "The database validates the lifecycle rules" |
| 688 | "the storage engine's runtime accounting" | "the store's runtime accounting" |
| 707 | `Internal` … "engine fault" | "database fault" |

The key mode `CallerAssigned` stays. "Golem DB" (3) waits for section 8.

**Add `StoreFull` to the shared error set** (*Common conventions*, the *Errors* table). The code
reports a full store as `ApiError::StoreFull` (review N16). Insert this row before `Internal`:

| error | raised when |
| --- | --- |
| `StoreFull` | the store reached its size cap. Environmental, not deterministic: it surfaces at `commit`, the commit writes nothing, and it must never become part of a result other nodes see |

The paragraph *One class of failure surfaces late* already names "storage exhaustion" among the
failures that can only surface at `commit`; add "(`StoreFull`)" after it.

## 4. Metering spec (`golem-db-metering.md`)

- Line 417: "The host funds commit-level engine overhead" → "The host funds commit-level database
  overhead".
- "Golem DB" (26) waits for section 8.

## 5. Design (`golem-db-design.md`)

- **"engine" (104): deferred.** The design is an implementation document where "the engine" often
  means the internal machinery. Leave it, including headings and anchors. Two exceptions, because
  they collide with the API spec:
  - line 3, "the **design record** of the Golem DB storage engine": replace "storage engine" with
    "database" (the API spec uses "storage engine" for MDBX, now "store");
  - line 273, "MDBX database environment" → "MDBX environment" (§1, *Abstract Primitives to Physical MDBX
    Mapping*), so "database" is not used for MDBX.
- **`machineId`** (lines 3323, 3353 in §13; 3438 in *Open Questions*): as in the API spec, nothing to
  do if D11's removal has landed; otherwise rename to `nodeId` and "engine-assigned" to
  "database-assigned".
- "Golem DB" (27) waits for section 8.

## 6. `CHANGES.md`

Add three rows and one triage-log entry. IDs S16, P10 and T07 were free at `0ade839`; take the next
free ones if that changed.

**Structure** (kind K3), in the *Structure* table:

| ID | Status | Pri | Change | Sources | Acceptance check | Link |
|---|---|---|---|---|---|---|
| S16 | triaged | P2 | Vocabulary: database / store / reserved in the API and metering specs; "engine" dropped from both; `EngineAssigned` → `Generated`; `machineId` → `nodeId` if it survives D11; design line 3 and "MDBX database environment" fixed. Design-wide "engine" pass deferred | implementation-review.md Part 0 (N7, N8, N10) | Section 9 checks pass | naming-migration.md sections 3–5 |

**Product decisions**, in the *Product decisions* table:

| ID | Status | Decision | Sources | Owner | Default if undecided | Link |
|---|---|---|---|---|---|---|
| P10 | wai | Product name: "Golem DB" or **GolemDB** (precedents RocksDB, FoundationDB, SurrealDB, LanceDB)? Decides doc prose, doc titles, and the repo name `golemdb` (N2) | implementation-review.md Part 0 (N1, N2) | Product | "GolemDB": packages and code already use `golemdb` | naming-migration.md section 8 |

**Fix** (kind K1), once the `StoreFull` row has landed in the API spec: add this row to *Closed →
Done*, as K1 rows go straight there:

| ID | Kind | Change | Sources | Closed by | Landed in |
|---|---|---|---|---|---|
| F31 | K1 | `[API]` `StoreFull` in the shared error set: a full store surfaces at `commit`, writes nothing, and is environmental | implementation-review.md N16 | The error table lists `StoreFull`; the late-failure paragraph names it | api.md *Common conventions* |

**Tasks**, in the *Tasks* table:

| ID | Status | Task | Origin | Link |
|---|---|---|---|---|
| T07 | open | With the GitHub repo rename to `golemfactory/golemdb` (after P10): rename `docs/golem-db-*.md` → `docs/golemdb-*.md` and update every link to them; replace `golem-db` in READMEs; rename the `.devcontainer` volumes (`golem-db-target` etc.) | P10; implementation-review.md N2 | naming-migration.md section 8 |

**Triage log**, newest first, in the existing style:

> - **2026-10-04 (API shape).** Also in PR #29: `Database::open(store, &config)`,
>   `open_memory(&genesis)` and `from_store` as the only constructors; `StoreConfig` with a store
>   file (`StoreConfig::load`); `Config` / `Genesis` (with `Genesis::DEV`); the lower-level opening
>   layer behind an `internals` feature; `ApiError::StoreFull`. Spec follow-up: F31.
> - **2026-10-03 (naming).** Code renames landed in PR #29 (`matthiaszimmermann/refactor/namings`):
>   storage `Database` → `Store` (`MemoryStore`, `MdbxStore`), facade `GolemDb` → `Database`,
>   private `Engine` → `Inner`, "backend" → store / store implementation
>   (`StorageError::Implementation`), "engine" dropped from code (`parse_reserved`). Documentation
>   follow-ups: S16 (spec vocabulary), P10 (product name), T07 (file and repo renames).

## 7. `decisions_for_architect.md`

- Leave existing entries as written: their "engine" (65) is part of the record of each brief.
- `machineId`: D11 already decides its removal, so no rename is needed there.
- No new entry: naming is a structure item (S16) and a product decision (P10), not an architect
  decision.

## 8. Gated on the product name (P10) and the repo rename (T07)

Apply only once P10 is decided for "GolemDB":
- "Golem DB" → "GolemDB" in all documents: API spec (3), metering spec (26), design (27),
  `CHANGES.md` (3), `decisions_for_architect.md` (6), including titles such as "Golem DB — Technical
  Design". Quoted historical text stays (see *Do not change*).

Apply only together with the GitHub repo rename (T07):
- file renames `golem-db-*.md` → `golemdb-*.md`, all links to them, `golem-db` mentions in READMEs
  and `.devcontainer`.

## 9. Checks

Run after sections 3–5; each must print nothing:

```bash
grep -niw 'engine' docs/golem-db-api.md | grep -v 'what-the-engine-enforces'
grep -n 'EngineAssigned\|storage engine\|the store \(mints\|never\|performs\)' docs/golem-db-api.md
grep -niw 'engine' docs/golem-db-metering.md
grep -n 'storage engine\|MDBX database environment' docs/golem-db-design.md
```

`grep -c StoreFull docs/golem-db-api.md` must print at least 2 (the error row and the late-failure
paragraph). If D11's removal has not landed, also check that `machineId` became `nodeId` in the API spec and
§13 of the design. After section 8: `grep -rn 'Golem DB' docs/` prints only quoted historical
text.

## 10. Record operations (PR #30)

The record-operations work ([record-ops.md](record-ops.md), review N18–N21) decides several points
the specs leave open. Apply on the spec branch, matching by phrase as above.

**API spec (`golem-db-api.md`):**
- *Record keys* table: besides `EngineAssigned` → `Generated` (section 3), state that the mode is
  fixed in genesis (`record_keys`) and give the derivation:
  `H("golemdb/record-key/v1" ‖ seed ‖ id)` with the deployment's hash, the 32-byte genesis seed
  and the new record's ID as `u64` big-endian. Add why the modes are exclusive: generated keys
  are predictable, so a caller-assigned create could otherwise claim a future generated key.
- `create`, the sentence "The `#meta` encoding and read visibility remain to be…": replace with
  the decision: `#meta` holds four `u64` big-endian counts over the user cells (cells, cell bytes,
  indexed cells, index bytes), 32 bytes as a `bytes32` field, and a full `get` returns it like
  `#key`.
- *Common conventions*, receipt: the Rust API returns the D4 details (cells created, updated and
  deleted; index joins and leaves; cell and index bytes written and deleted) with every receipt,
  not only on request; the spec's opt-in remains valid for transports. The details count effects:
  a set to the current value and a failed call count nothing.
- `budget?` and `OutOfBudget`: present in the Rust API before metering enforces them (cost 0).

**Design (`golem-db-design.md`):**
- §3 *Record identity* / Record Model: the `#meta` layout above, and the completeness argument: a
  client with a trusted root that receives the binding, `#meta` and `cells + 2` distinct cells
  with inclusion proofs has the whole record, because `#meta` is committed state. Conditions:
  every mutation maintains the counts exactly; the count covers user cells only; the client's
  root is trusted. Not covered: projections, withheld records, query results.
- §4 `#params` table: add `#keyMode` (`u32`: 0 caller-assigned, 1 generated) and `#keySeed`
  (`bytes32`, generated mode only), both fixed at genesis and part of the genesis identity.

**D09 / Appendix A:** the `#meta` encoding and its type tag (`FixedBytes(W32)`); the record-key
derivation and its domain tag; the `#keyMode` values.

**Metering spec (`golem-db-metering.md`):** `Details` matches R9/D4 except "index terms created",
which needs an index lookup at call time and follows later.

**`CHANGES.md`:** one structure row next to S16 (take the next free ID, S17 at `0ade839`):

| ID | Status | Pri | Change | Sources | Acceptance check | Link |
|---|---|---|---|---|---|---|
| S17 | triaged | P2 | Record operations in the specs: key-mode derivation and exclusivity, `#meta` encoding and visibility, `#keyMode` / `#keySeed` in `#params`, receipt details always returned in the Rust API, the `#meta` completeness argument | record-ops.md; implementation-review.md N18–N21 | The items of naming-migration.md section 10 are in the specs; D09 lists the two encodings | naming-migration.md section 10 |
