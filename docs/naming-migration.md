# Naming migration: instructions for the documentation

Instructions for bringing the design documents in line with the naming decisions in
[implementation-review.md](implementation-review.md) Part 0 (N1–N10). The code side is done on
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

**Tasks**, in the *Tasks* table:

| ID | Status | Task | Origin | Link |
|---|---|---|---|---|
| T07 | open | With the GitHub repo rename to `golemfactory/golemdb` (after P10): rename `docs/golem-db-*.md` → `docs/golemdb-*.md` and update every link to them; replace `golem-db` in READMEs; rename the `.devcontainer` volumes (`golem-db-target` etc.) | P10; implementation-review.md N2 | naming-migration.md section 8 |

**Triage log**, newest first, in the existing style:

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

If D11's removal has not landed, also check that `machineId` became `nodeId` in the API spec and
§13 of the design. After section 8: `grep -rn 'Golem DB' docs/` prints only quoted historical
text.
