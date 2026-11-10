use serde::{Deserialize, Serialize};
use spunkpipe_core::{fsutil::atomic_write, Hash, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What SpunkPipe installed. Only files listed here are ever modified or removed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Lock {
    pub game: String,
    pub serial: u64,
    pub files: BTreeMap<String, LockEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockEntry {
    pub hash: Hash,
    pub size: u64,
    /// Used to skip re-hashing files that have not been touched since we wrote them.
    pub mtime_ns: u128,
}

impl Lock {
    pub fn path(state: &Path) -> PathBuf {
        state.join("lock.json")
    }

    pub fn load(state: &Path) -> Result<Lock> {
        match std::fs::read(Self::path(state)) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, state: &Path) -> Result<()> {
        atomic_write(&Self::path(state), &serde_json::to_vec_pretty(self)?)
    }
}
