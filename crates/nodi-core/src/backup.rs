//! Snapshot backups: copy, list, prune (retention) and restore.
//!
//! A snapshot lives in `data/backups/<yyyymmdd-hhmmss>/` (UTC) and contains
//! `tree.json`, `notes/` and `attachments/`. Snapshots never overwrite each
//! other: a collision inside the same second gets a `-1`, `-2`, ... suffix.
//!
//! `restore` always takes a fresh safety snapshot of the current state first,
//! so a restore can itself be undone.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::id::now_secs;
use crate::storage::DataDir;

/// Snapshot retention used by the app (keep the newest 20).
pub const DEFAULT_KEEP: usize = 20;

/// Takes a snapshot of `tree.json`, `notes/` and `attachments/` and returns the
/// new snapshot directory.
pub fn snapshot(data: &DataDir) -> io::Result<PathBuf> {
    data.ensure_layout()?;
    let dir = create_unique_snapshot_dir(&data.backups_dir(), now_secs())?;

    let outcome = (|| -> io::Result<()> {
        copy_file_if_exists(&data.tree_path(), &dir.join("tree.json"))?;
        copy_tree(&data.notes_dir(), &dir.join("notes"))?;
        copy_tree(&data.attachments_dir(), &dir.join("attachments"))?;
        Ok(())
    })();

    // On failure the partial snapshot is left in place: it is harmless and
    // deleting it would risk removing data the user might still want. The
    // error is reported and `prune` will age it out.
    outcome?;
    Ok(dir)
}

/// Snapshot directories, oldest first.
pub fn list(data: &DataDir) -> io::Result<Vec<PathBuf>> {
    let base = data.backups_dir();
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut snapshots: Vec<(SnapshotKey, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(key) = snapshot_key(name) {
            snapshots.push((key, entry.path()));
        }
    }
    snapshots.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(snapshots.into_iter().map(|(_, path)| path).collect())
}

/// Keeps the newest `keep` snapshots and deletes older ones. Returns the
/// removed directories (oldest first). Deleting snapshots is the whole point
/// of retention, so this is the one place where removal is intended; only
/// directories that match the snapshot naming scheme are ever touched.
pub fn prune(data: &DataDir, keep: usize) -> io::Result<Vec<PathBuf>> {
    let mut snapshots = list(data)?;
    if snapshots.len() <= keep {
        return Ok(Vec::new());
    }
    let remove_count = snapshots.len() - keep;
    let removed: Vec<PathBuf> = snapshots.drain(..remove_count).collect();
    for dir in &removed {
        fs::remove_dir_all(dir)?;
    }
    Ok(removed)
}

/// Restores `snapshot` over the live data directory. Returns the safety
/// snapshot that was taken of the state *before* the restore.
pub fn restore(data: &DataDir, snapshot: &Path) -> io::Result<PathBuf> {
    if !snapshot.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("snapshot not found: {}", snapshot.display()),
        ));
    }

    let safety = self::snapshot(data)?;
    data.ensure_layout()?;

    let tree_snapshot = snapshot.join("tree.json");
    if tree_snapshot.is_file() {
        copy_file_atomic(&tree_snapshot, &data.tree_path())?;
    }
    copy_tree_over(&snapshot.join("notes"), &data.notes_dir())?;
    copy_tree_over(&snapshot.join("attachments"), &data.attachments_dir())?;
    Ok(safety)
}

/// `yyyymmdd-hhmmss` for a unix timestamp, UTC.
pub fn timestamp_name(unix_secs: u64) -> String {
    let (year, month, day, hour, minute, second) = civil_utc(unix_secs);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Converts unix seconds to a UTC calendar date/time.
fn civil_utc(unix_secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let seconds_of_day = (unix_secs % 86_400) as u32;
    let days = (unix_secs / 86_400) as i64;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };

    (year, month as u32, day as u32, hour, minute, second)
}

fn create_unique_snapshot_dir(base: &Path, unix_secs: u64) -> io::Result<PathBuf> {
    fs::create_dir_all(base)?;
    let stamp = timestamp_name(unix_secs);
    let mut attempt = 0u32;
    loop {
        let name = if attempt == 0 {
            stamp.clone()
        } else {
            format!("{stamp}-{attempt}")
        };
        let dir = base.join(name);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && attempt < 10_000 => {
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SnapshotKey {
    stamp: String,
    suffix: u64,
}

fn snapshot_key(name: &str) -> Option<SnapshotKey> {
    let bytes = name.as_bytes();
    if bytes.len() < 15 {
        return None;
    }
    if bytes[8] != b'-' {
        return None;
    }
    if !bytes[..8].iter().all(u8::is_ascii_digit) || !bytes[9..15].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let stamp = name[..15].to_owned();
    let rest = &name[15..];
    let suffix = if rest.is_empty() {
        0
    } else {
        let digits = rest.strip_prefix('-')?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()?
    };
    Some(SnapshotKey { stamp, suffix })
}

fn copy_file_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::copy(from, to) {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Copies a directory recursively into `to`. `*.tmp` scratch files are not
/// part of a snapshot, so they are skipped. Missing sources are ignored.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&path, &target)?;
        } else if !is_tmp(&path) {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Like [`copy_tree`] but overwrites the destination (restore path).
fn copy_tree_over(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree_over(&path, &target)?;
        } else if !is_tmp(&path) {
            copy_file_atomic(&path, &target)?;
        }
    }
    Ok(())
}

/// Streaming atomic copy: `<dest>.tmp` → rename, so a crash mid-restore can
/// never leave a half-written note.
fn copy_file_atomic(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp_name = to.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = to.with_file_name(tmp_name);
    let outcome = (|| -> io::Result<()> {
        fs::copy(from, &tmp)?;
        fs::rename(&tmp, to)?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    outcome
}

fn is_tmp(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
}
