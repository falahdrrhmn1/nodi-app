# Task 1a — nodi-core (data model + reliable storage)

Read `docs/ARCHITECTURE.md` first. It is binding. Work ONLY inside `crates/nodi-core/`. Do not touch other crates, root Cargo.toml, or files outside `E:\nodi-app\mark-3`.

## Build env (PowerShell, set before every cargo call)
```
$env:CARGO_HOME='E:\Software\Rust\cargo'; $env:RUSTUP_HOME='E:\Software\Rust\rustup'; $env:PATH="E:\Software\Rust\cargo\bin;$env:PATH"
```
Use `cargo test -p nodi-core`, `cargo clippy -p nodi-core --all-targets`, `cargo fmt -p nodi-core`.

## Deliver
Create crate `crates/nodi-core` (lib, `version.workspace = true`, `edition.workspace = true`). Deps: `serde` (derive), `serde_json` only.

1. `id.rs`: `NoteId(String)` — 32 lowercase hex chars, generated from `SystemTime` nanos + process id + atomic counter mixed with `std::hash` (RandomState). `NoteId::parse` validates.
2. `tree.rs`:
   - `NodeKind { Folder, Note }`, `Node { id, kind, title, icon: Option<String>, parent: Option<NoteId>, order: i64, trashed: bool, created: u64, modified: u64 }` (unix seconds).
   - `Tree` holding nodes (HashMap or Vec) with methods: `new_node(kind, title, parent) -> NoteId`, `rename`, `set_icon`, `move_node(id, new_parent, index) -> Result` (rejects moving into self or any descendant; Notes AND Folders may have children), `children(parent) -> Vec<&Node>` sorted by order (trashed excluded), `trash(id)` (trashes subtree), `restore(id)` (restores subtree; if parent is trashed or gone, restore to root), `purge(id)` (permanent delete of a trashed subtree, returns ids removed), `duplicate(id) -> NoteId` (node only, new id, title + " (copy)"), `descendants(id)`, `path_titles(id)`.
   - serde (de)serialize to the `tree.json` format: `{ "version": 1, "nodes": [ ... ] }`.
3. `storage.rs`:
   - `DataDir` with paths per ARCHITECTURE.md; `DataDir::resolve()` (env `NODI_DATA_DIR` else `<exe_dir>\data`), `DataDir::at(path)` for tests; `ensure_layout()` creates subfolders.
   - `atomic_write(path, bytes) -> io::Result<()>`: write `path.tmp`, `sync_all`, if `path` exists copy it to `path.prev` (only for note content; make this a flag), `std::fs::rename(tmp, path)`. On any error remove nothing except the tmp it created; return Err.
   - `load_tree/save_tree`, `load_note(id) -> Option<Vec<u8>>`, `save_note(id, rtf, txt)` (rtf + txt sidecar), `load_session/save_session` (session struct: open tabs ids, active index, per-tab caret pos u32 + scroll u32, sidebar width, window rect).
   - `recover_leftovers()`: any `*.tmp` under notes/ moved to `data/recovery/recovered-<name>-<ts>` and listed in the return value. Never deletes.
   - Failure injection for tests: `DataDir` has field `fail_writes: Arc<AtomicBool>`; when true `atomic_write` returns `Err(io::ErrorKind::Other, "injected failure")` before touching disk.
4. `autosave.rs`:
   - `SaveRequest { note: NoteId, seq: u64, rtf: Vec<u8>, txt: String }`, plus `TreeSave(String json)` and `SessionSave(String json)` variants in an enum `Job`.
   - `SaveResult { target: Target, seq: u64, result: Result<(), String> }`.
   - `Writer::spawn(data: DataDir, notify: Box<dyn Fn(SaveResult) + Send>) -> Writer` — single background thread, mpsc. `Writer::submit(job)`, `Writer::flush_blocking(timeout) -> bool` (returns when all submitted jobs are done), `Drop` joins.
   - Coalescing: if several requests for the same note are queued, only the newest seq needs writing (older ones reported as superseded Ok? NO — report them with `result: Ok(())` only if the newer one succeeded; simplest correct approach: write newest, report result for newest seq; UI ignores stale seqs).
5. `backup.rs`: `snapshot(data) -> PathBuf` copies tree.json + notes/ + attachments/ into `backups/<yyyymmdd-hhmmss>` (format time yourself from unix secs, UTC is fine; add `-N` suffix on collision, NEVER overwrite), `prune(keep: usize)`, `list()`, `restore(snapshot)` (first takes a fresh snapshot of current state, then copies snapshot over data).
6. `search.rs`: `search_titles(tree, query)` case-insensitive substring; `search_text(data, tree, query) -> Vec<(NoteId, usize /*hits*/)>` reading `.txt` sidecars.
7. `links.rs`: `format_note_link(id) -> "nodi://note/<id>"`, `parse_note_link(&str) -> Option<NoteId>`.

## Tests (required, in `crates/nodi-core/tests/` or `#[cfg(test)]`), use a temp dir under `std::env::temp_dir()` with unique name, clean it up:
- tree: create A > B > C, rename B, C still under B, serde round-trip preserves everything.
- tree: move A into C → Err. Note-with-children works. trash/restore subtree.
- storage: atomic save/load round-trip; second save creates `.prev` with old content.
- storage: fail_writes=true → Err and original file unchanged.
- autosave: submit 50 rapid requests for one note, flush_blocking → file has newest content, last reported seq == 50 Ok.
- autosave: failure injection → result Err delivered, then disable failure, resubmit → Ok.
- backup: snapshot twice in same second → two distinct dirs; prune keeps N; restore round-trip.
- recover_leftovers moves a planted `.tmp`.
- links parse/format.

## Done when
`cargo test -p nodi-core` all green, `cargo clippy -p nodi-core --all-targets -- -D warnings` clean, `cargo fmt -p nodi-core --check` clean.
Finish by writing `crates/nodi-core/REPORT.md`: what was done, test output summary (copy the final `test result:` lines), anything not done. Do NOT claim anything you did not run.
