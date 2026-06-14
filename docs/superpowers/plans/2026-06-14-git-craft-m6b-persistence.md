---
title: git-craft M6b — region save/load persistence
date: 2026-06-14
domain: world-io
type: feature
priority: high
breaking: false
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [git-craft/src/world/section.rs, git-craft/src/world/region.rs, git-craft/src/world/persistence.rs, git-craft/src/world/mod.rs, git-craft/src/world/chunks.rs, git-craft/src/app.rs, git-craft/CHANGELOG.md, git-craft/AGENTS.md]
---

# git-craft M6b — region save/load persistence

**Goal:** Player edits survive a restart. Blocks the player breaks/places are saved to
region files on disk and reloaded when the player returns, instead of being overwritten by
fresh worldgen. Streaming integrates load/save asynchronously so the frame loop never blocks
on I/O.

**Key insight — only edits are persisted, and light is never serialized.** Worldgen is
deterministic (`same_seed_is_bit_identical`), so an untouched column is reproduced exactly by
regenerating it; only columns the player *edited* need saving. And lighting is a pure function
of blocks (+ neighbors), so a loaded column recomputes its light with `light_new_column` — the
exact path a generated column takes — and the existing seam-healing on insert handles
cross-column light. The on-disk payload is therefore just the 8 sections' block data.

**Architecture:**
1. **`section.rs` — section (de)serialization (pure, TDD).** `Section::write_bytes(&self, out)` and
   `Section::read_bytes(&[u8]) -> Option<(Section, consumed)>` over the existing palette form
   (`[palette_len u16][palette × u16][bits u8][data × u64]`, data word count derived from `bits`).
   Kept in `section.rs` so the private palette invariants stay local.
2. **`region.rs` — region container + on-disk store (pure core + thin fs, TDD).**
   - Column payload = the 8 sections written back-to-back; `serialize_column` / `deserialize_column`.
   - `region_of(col)` / `local_index(col)` (32×32 columns per region via arithmetic shift) and the
     inverse for enumeration; `serialize_region` / `parse_region` over a `BTreeMap<u16, Vec<u8>>`
     with a `GCR1` magic + version header (deterministic byte order → reproducible files).
   - `RegionStore { dir }`: `save_column` (read-modify-write the region file, temp-file + atomic
     rename), `load_column`, and `scan_saved() -> HashSet<ColumnPos>` (enumerate every persisted
     column by parsing region headers). All fs ops unit-tested against a temp dir.
3. **`persistence.rs` — async worker.** A single background thread owns the `RegionStore` (so region
   files are never written concurrently). `Persistence::new(dir) -> (Self, HashSet<ColumnPos>)`
   scans the saved set up front, then spawns the worker. `request_load(pos)` and `request_save(pos,
   sections)` send over a channel; the worker serializes saves and deserializes loads off the main
   thread, recomputes light with `light_new_column`, and returns `Loaded::Column { pos, data, light }`
   or `Loaded::Failed { pos }` over a result channel drained each frame. `shutdown` flushes queued
   saves (FIFO) then joins.
4. **`app.rs` — streaming integration.** `App` gains `persistence: Option<Persistence>` (None in
   bench mode — determinism), `saved_columns: HashSet<ColumnPos>`, and `edited_columns:
   HashSet<ColumnPos>`. A successful `set_block` marks the column edited. In `update_world`: drain
   loaded columns alongside gen results (insert + seam-heal; `Failed` falls back to `spawn_gen`);
   when requesting a column, `request_load` it if it is in `saved_columns`, else `spawn_gen` (one
   shared in-flight budget); before `unload_outside`, save any edited column leaving the keep radius
   and move it from `edited` to `saved`. `exiting` flushes the still-loaded edited columns and shuts
   the worker down. Save dir: `saves/region/` (already in `.gitignore`).

**Validation:** `cargo test` covers section (de)serialization roundtrips, column/region roundtrips,
and `RegionStore` save→load→scan against a temp dir. End-to-end: edit blocks, fly away (eviction
save) or quit (exit flush), restart, confirm the edits are present (a placed block persists, a broken
block stays broken). Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`.

**Environment:** all `cargo` from `git-craft/`; `--release` only. Branch `feat/m6b-persistence`; PR to `main`.

---

## Stage A — section + column + region serialization (pure, TDD)

- [x] **A1 `section.rs`:** `write_bytes(&self, out: &mut Vec<u8>)` and `read_bytes(&[u8]) ->
  Option<(Section, usize)>`. `read_bytes` validates `palette_len >= 1`, `bits <= 16`, derives the
  data word count from `bits`, and returns `None` on truncation. Tests (use the existing semantic
  `PartialEq`): roundtrip empty/uniform, a multi-palette section, and a 3-bit word-spanning section;
  truncated input → None; two sections roundtrip back-to-back via the returned `consumed`.
- [x] **A2 `region.rs` column + coords:** `serialize_column(&[Arc<Section>]) -> Vec<u8>` (8 sections)
  and `deserialize_column(&[u8]) -> Option<Vec<Section>>`; `region_of(ColumnPos) -> (i32,i32)`,
  `local_index(ColumnPos) -> u16`, and `column_at(region, index) -> ColumnPos` (inverse). Tests:
  column roundtrip; `column_at(region_of(c), local_index(c)) == c` for assorted (incl. negative)
  coords; local index in `0..1024`.
- [x] **A3 `region.rs` region blob:** `serialize_region(&BTreeMap<u16, Vec<u8>>) -> Vec<u8>` and
  `parse_region(&[u8]) -> Option<BTreeMap<u16, Vec<u8>>>` with a `GCR1` magic + version header.
  Tests: roundtrip an empty and a multi-column map; a bad magic / truncated blob → None.

## Stage B — RegionStore (fs, TDD against a temp dir)

- [x] **B1 `RegionStore`:** `new(dir)`, `save_column(pos, payload)` (read-modify-write the region
  file via a temp file + atomic rename, `create_dir_all` on first write), `load_column(pos) ->
  io::Result<Option<Vec<u8>>>`, `scan_saved() -> HashSet<ColumnPos>`. Tests against a unique temp
  dir: save→load roundtrips a column; two columns in the same region coexist; overwriting a column
  replaces it; `load_column` of an absent column/region is `Ok(None)`; `scan_saved` enumerates
  exactly the saved columns.

## Stage C — async worker

- [x] **C1 `persistence.rs`:** `Loaded { Column { pos, data: ColumnData, light: Box<[LightData;8]> },
  Failed { pos } }`; private `Req { Load, Save, Shutdown }`. `Persistence::new(dir) -> (Self,
  HashSet<ColumnPos>)` scans the saved set then spawns the worker thread owning the `RegionStore`.
  `request_load`/`request_save` (track `load_in_flight`), `drain_loaded() -> Vec<Loaded>`, `shutdown`
  (FIFO-drains saves, then joins). The worker recomputes light with `light_new_column`. Light smoke
  test: a save request followed by a load request returns the column's blocks intact (poll the
  result channel like the existing `jobs` test).

## Stage D — app integration + end-to-end

- [x] **D1 fields + edits:** `App` gains `persistence: Option<Persistence>` (built in `new` when not
  in bench mode, dir `saves/region`), `saved_columns`, `edited_columns`. A successful break/place in
  `update_interaction` inserts the edited block's column into `edited_columns`.
- [x] **D2 `update_world`:** drain `persistence.drain_loaded()` next to gen results — `Column`
  inserts via `insert_generated(pos, data, *light, vec![])` + the same seam-heal, `Failed` calls
  `spawn_gen`; request a column with `request_load` when it is in `saved_columns` (gated on the
  combined `gen_in_flight + load_in_flight` budget), else `spawn_gen`; before `unload_outside`, save
  every edited column now outside `UNLOAD_RADIUS` (clone its section Arcs, `request_save`, move it
  from `edited` to `saved`).
- [x] **D3 exit flush + docs:** add `ApplicationHandler::exiting` — flush still-loaded edited columns
  then `persistence.take().shutdown()`. Note the feature + `saves/` location in CHANGELOG `[Unreleased]`
  and AGENTS.md. Manual end-to-end: place/break blocks, fly out of range and back (eviction save) and
  quit→relaunch (exit flush); confirm edits persist. Commit: `feat: persist player edits to region
  files with async load/save (m6)`.

---

## Self-Review
- Spec §10 M6 "region save/load" → 32×32-column region files with atomic read-modify-write; only
  player-edited columns are persisted (untouched terrain regenerates deterministically). ✓
- Spec §4 palette storage → the on-disk payload is the section palette form verbatim; light is
  recomputed on load via the generation path (`light_new_column` + seam healing), never stored. ✓
- Async (§3 job discipline): all disk I/O runs on a dedicated worker thread; the main loop only
  enqueues requests and drains finished loads, so the frame never blocks. A single worker owns the
  store, so region files are never written concurrently.
- Determinism / safety: bench mode disables persistence; saves use temp-file + atomic rename; a
  corrupt/missing load returns `Failed` and falls back to regeneration rather than crashing.
- Engine-core discipline: section/column/region (de)serialization are pure and unit-tested first;
  `RegionStore` fs ops are tested against a temp dir; `app.rs`/`persistence.rs` hold the threading glue.

