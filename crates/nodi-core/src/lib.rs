//! `nodi-core` — the platform-independent heart of Nodi.
//!
//! This crate knows nothing about Windows. It owns the note tree, the on-disk
//! layout, the atomic write path, the background autosave writer, snapshots and
//! search. The Win32 shell (`nodi-windows`) drives it.
//!
//! Data-loss safety rules implemented here:
//! * every file write goes through `DataDir::atomic_write` (tmp → fsync → rename),
//! * note content keeps one previous generation in `<id>.rtf.prev`,
//! * leftover `.tmp` files are never deleted, they are moved to `data/recovery/`,
//! * backups never overwrite each other.

pub mod autosave;
pub mod backup;
pub mod id;
pub mod image;
pub mod links;
pub mod markdown;
pub mod math;
pub mod rtf;
pub mod search;
pub mod storage;
pub mod tree;

pub use id::NoteId;
pub use storage::{DataDir, Session, TabState, WindowRect};
pub use tree::{Node, NodeKind, Tree, TreeError};
