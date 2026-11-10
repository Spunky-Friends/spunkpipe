use crate::path::validate_rel_path;
use crate::{Error, Hash, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const FORMAT: u32 = 1;
const MAX_FILES: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path using `/` separators.
    pub path: String,
    /// BLAKE3 hash of the uncompressed content.
    pub hash: Hash,
    /// Uncompressed size in bytes.
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// Game profile id, for example `7dtd`.
    pub game: String,
    /// Increases by one on every publish. Clients refuse to go backwards.
    pub serial: u64,
    pub created_unix: u64,
    /// Sorted by path.
    pub files: Vec<FileEntry>,
}

/// Two paths with the same key may refer to the same file on a case-insensitive or
/// normalising filesystem.
fn collision_key(path: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    // Lowercase, decompose, then lowercase again: some characters only fold after decomposition.
    path.to_lowercase().nfd().collect::<String>().to_lowercase()
}

impl Manifest {
    pub fn new(game: &str, serial: u64, created_unix: u64, mut files: Vec<FileEntry>) -> Self {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Manifest { format: FORMAT, game: game.to_string(), serial, created_unix, files }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// Parses and validates a manifest.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let m: Manifest = serde_json::from_slice(bytes)?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |s: String| Err(Error::BadManifest(s));
        if self.format != FORMAT {
            return bad(format!("unsupported format {}", self.format));
        }
        if self.game.is_empty() || self.game.len() > 64 {
            return bad("bad game id".into());
        }
        if self.files.len() > MAX_FILES {
            return bad("too many files".into());
        }
        let mut seen = HashSet::new();
        let mut prev: Option<&str> = None;
        for f in &self.files {
            validate_rel_path(&f.path)?;
            if let Some(p) = prev {
                if p >= f.path.as_str() {
                    return bad(format!("files not strictly sorted at {:?}", f.path));
                }
            }
            prev = Some(&f.path);
            // Windows and macOS are case-insensitive and macOS normalises Unicode, so paths that
            // differ only in case or in composed vs decomposed characters land on the same file.
            if !seen.insert(collision_key(&f.path)) {
                return bad(format!("case/normalisation duplicate {:?}", f.path));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(p: &str) -> FileEntry {
        FileEntry { path: p.into(), hash: Hash::of(p.as_bytes()), size: 1 }
    }

    #[test]
    fn round_trip() {
        let m = Manifest::new("7dtd", 3, 100, vec![entry("b"), entry("a/x")]);
        let back = Manifest::from_bytes(&m.to_bytes().unwrap()).unwrap();
        assert_eq!(m, back);
        assert_eq!(back.files[0].path, "a/x");
    }

    #[test]
    fn rejects_bad_manifests() {
        let mut m = Manifest::new("7dtd", 1, 0, vec![entry("a"), entry("b")]);
        m.files[1].path = "../b".into();
        assert!(m.validate().is_err());

        let m = Manifest::new("7dtd", 1, 0, vec![entry("Mod/A.dll"), entry("mod/a.dll")]);
        assert!(m.validate().is_err());

        // "é" as one character vs "e" plus a combining accent (macOS treats these as the same name).
        let m = Manifest::new("7dtd", 1, 0, vec![entry("Mod/caf\u{e9}.xml"), entry("Mod/cafe\u{301}.xml")]);
        assert!(m.validate().is_err());
        let m = Manifest::new("7dtd", 1, 0, vec![entry("Mod/STRASSE.xml"), entry("Mod/strasse.xml")]);
        assert!(m.validate().is_err());

        let mut m = Manifest::new("7dtd", 1, 0, vec![entry("a")]);
        m.format = 99;
        assert!(m.validate().is_err());

        let mut m = Manifest::new("7dtd", 1, 0, vec![entry("a"), entry("b")]);
        m.files.swap(0, 1);
        assert!(m.validate().is_err());
    }
}
