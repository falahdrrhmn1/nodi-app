//! Shared helpers for the integration tests: a unique, self-cleaning temp dir.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nodi_core::DataDir;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A throwaway directory under the system temp dir, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let unique = format!(
            "nodi-core-test-{label}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// A `data/` directory inside this temp dir.
    pub fn data_dir(&self) -> DataDir {
        DataDir::at(self.path.join("data"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
