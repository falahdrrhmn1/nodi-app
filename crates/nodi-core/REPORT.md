> **Archived agent report from the first build phase.** Current status, tests and
> measurements: [docs/STATUS.md](../../docs/STATUS.md).

# nodi-core — Phase 1a report

Crate `crates/nodi-core` (lib, `version.workspace`, `edition.workspace`, v0.3.0).
Dependencies: `serde` (derive) + `serde_json` only (`serde_json` is also listed as
a dev-dependency for the integration tests). No Win32, no other crates.

## Delivered

| File | Contents |
| --- | --- |
| `src/lib.rs` | module wiring + re-exports (`NoteId`, `DataDir`, `Session`, `TabState`, `WindowRect`, `Node`, `NodeKind`, `Tree`, `TreeError`) |
| `src/id.rs` | `NoteId` = 32 lowercase hex chars, generated from unix nanos + pid + atomic counter hashed by two independently seeded `RandomState` hashers (128 bits); `NoteId::parse` validates length/charset; serde (de)serialisation validates on read |
| `src/tree.rs` | `NodeKind`, `Node`, `Tree` with `new_node`, `rename`, `set_icon`, `move_node` (cycle rejection), `children` (sorted, trash-filtered), `trash`/`restore` (subtree; restore-to-root when the parent is trashed/gone), `purge` (trashed subtrees only, returns removed ids), `duplicate` (node only, `" (copy)"`), `descendants`, `ancestors`, `path_titles`, `is_within`/`is_descendant`, `iter_ordered`; `{ "version": 1, "nodes": [...] }` serde with deterministic node order and load-time repair of dangling parents/cycles |
| `src/storage.rs` | `DataDir` (`resolve` from `NODI_DATA_DIR` → `<exe_dir>\data`, `at`), all layout paths (`tree.json`, `session.json`, `notes/`, `attachments/`, `backups/`, `logs/nodi.log`, `recovery/`), `ensure_layout`, `atomic_write` / `atomic_write_keep_prev` (tmp → `write_all` → `sync_all` → optional `.prev` copy → `rename`), `load_tree`/`save_tree`, `load_note`/`load_note_prev`/`load_note_text`/`save_note` (RTF + `.txt` sidecar, one `.prev` generation), `Session`/`TabState`/`WindowRect` + `load_session`/`save_session`, `recover_leftovers`, `fail_writes` injection switch |
| `src/autosave.rs` | `SaveRequest`, `Job::{Note,TreeSave,SessionSave}`, `Target`, `SaveResult`, `Writer::spawn/submit/flush_blocking` + `Drop` (shutdown + join); one thread, `mpsc`, batch drain with per-note coalescing, newest seq reported |
| `src/backup.rs` | `snapshot`, `list`, `prune`, `restore`, `timestamp_name`; UTC `yyyymmdd-hhmmss`, `-N` collision suffix, never overwrites, retention pruning, safety snapshot before restore |
| `src/search.rs` | `search_titles` (case-insensitive substring, tree order, trashed excluded), `search_text` (counts hits in `.txt` sidecars) |
| `src/links.rs` | `format_note_link` → `nodi://note/<id>`, `parse_note_link` (case-insensitive scheme, trims, tolerates `?query`/`#fragment`, rejects invalid ids, never panics on multibyte input) |

### Safety properties implemented

* Every write is atomic (tmp + `sync_all` + `rename`); a failed write removes
  only the temp file it created and leaves the original untouched.
* `fail_writes = true` returns `io::ErrorKind::Other` / `"injected failure"`
  *before* touching the disk.
* Note content keeps `<id>.rtf.prev` (previous good generation).
* Leftover `*.tmp` files under `notes/` (recursively) are moved to
  `recovery/recovered-<name>-<ts>[.ext]` with collision suffixes. Nothing is
  ever deleted; if a rename fails the file is copied instead.
* Tree JSON is validated before writing (autosave) and repaired on load
  (dangling parents → root, cycles broken) instead of silently losing nodes.
* Two backups in the same second always get two distinct directories
  (`create_dir` is the uniqueness check).
* `restore` snapshots the state it is about to replace and copies each file
  atomically.

## Verification (all run, exit code 0)

```
cargo fmt -p nodi-core --check                                -> exit 0
cargo clippy -p nodi-core --all-targets -- -D warnings        -> exit 0
cargo test -p nodi-core                                       -> exit 0
```

Final `test result:` lines:

```
     Running unittests src\lib.rs (target\debug\deps\nodi_core-ba6e2aa237ca5326.exe)
running 8 tests
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
     Running tests\autosave.rs (target\debug\deps\autosave-2c0e4ecb8c8a5ec8.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
     Running tests\backup.rs (target\debug\deps\backup-dc1fe17cd546facf.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
     Running tests\search_links.rs (target\debug\deps\search_links-1ac36ea69a6e2378.exe)
running 3 tests
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
     Running tests\storage.rs (target\debug\deps\storage-2b7dbbc13d544685.exe)
running 9 tests
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
     Running tests\tree.rs (target\debug\deps\tree-11af5b4804800f4d.exe)
running 11 tests
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
   Doc-tests nodi_core
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

43 tests total (8 unit + 35 integration). Every test uses a unique directory
under `std::env::temp_dir()` and removes it on drop.

Coverage of the required test list:

* tree A > B > C, rename B keeps C under B, full serde round-trip (all fields),
* move A into C → `Err(TreeError::Cycle)`, note-with-children, sibling
  reorder (dense orders kept in both the old and new parent), trash/restore of a
  whole subtree, restore-to-root when the parent is trashed, purge returns the
  removed ids, duplicate copies only the node,
* atomic note save/load, `.prev` holds the previous content on the second save,
* failure injection → `Err(ErrorKind::Other)` and the original file/content is
  unchanged, no scratch file left behind, writes work again afterwards,
* 50 rapid autosave requests → newest RTF/`.txt` on disk and last reported
  seq = 50 Ok (older coalesced results are still Ok; out-of-order seqs keep the
  highest),
* autosave failure injection → `Err` delivered, then Ok after clearing the
  fault; tree/session jobs written; invalid JSON refused; drop flushes,
* two snapshots in the same second are distinct dirs; prune keeps the newest N
  and deletes only snapshots; restore round-trips content and tree and leaves a
  safety snapshot of the replaced state,
* `recover_leftovers` moves planted `.tmp` files (nested + same-name repeat)
  into `recovery/` and preserves their bytes,
* link format/parse round-trip, foreign URLs/uppercase/bad ids rejected,
  multibyte input does not panic.

## API deviations from the task shorthand (deliberate)

* `Tree::duplicate` returns `Option<NoteId>` (`None` when the source is gone)
  instead of panicking to produce a `NoteId`.
* The "keep `.prev`" flag is two methods (`atomic_write` and
  `atomic_write_keep_prev`) rather than a boolean parameter, so call sites
  cannot pass the wrong flag by accident.
* `backup::snapshot/list/prune/restore` take `&DataDir` as their first argument
  (the task shorthand omitted it); `prune(&data, keep)` returns the removed
  directories.
* Mutations that cannot sensibly fail return `bool` (`rename`, `set_icon`,
  `trash`, `restore`, `Writer::submit`); `move_node` returns
  `Result<(), TreeError>`.
* `Tree::children`/`new_node` take `Option<&NoteId>` / `Option<NoteId>`
  respectively.

## Not done / notes for Phase 2+

* `Tree::purge` removes metadata only. Note/attachment files are intentionally
  **not** deleted by `nodi-core` (no user-data deletion path is exposed yet); a
  later explicit storage API (with an undo window) must handle that.
* `backup::restore` copies the snapshot over the live data; files created after
  the snapshot are left in place (they are not deleted).
* Log file creation/rotation at 1 MB (`logs/nodi.log`) is not implemented in
  this task; only `DataDir::log_path()` and the directory layout are provided.
* Title/text search reads the whole sidecar per query; no index/cache yet
  (fine for Phase 1 sizes, revisit if search becomes slow).
* `Writer::spawn` panics if the OS refuses to create the writer thread — a
  deliberate fail-fast because saving without the writer is unsafe.
* `Cargo.lock` appeared at the workspace root because cargo generated it during
  the build; no root or other-crate file was edited by hand.
