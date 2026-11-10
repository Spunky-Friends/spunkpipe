//! Signed release manifest: what `spunkpipe update` trusts, instead of trusting the web host.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub const RELEASE_FORMAT: u32 = 1;
pub const RELEASE_FILE: &str = "release.json";
pub const RELEASE_SIG_FILE: &str = "release.json.sig";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub format: u32,
    /// `major.minor.patch`, without a leading `v`.
    pub version: String,
    pub assets: Vec<ReleaseAsset>,
}

pub fn sha256_hex(mut r: impl Read) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Parses `1.2.3` (a leading `v` is allowed) into comparable numbers.
pub fn parse_version(v: &str) -> Result<(u64, u64, u64)> {
    let bad = || Error::Other(format!("not a version number: {v:?}"));
    let mut it = v.trim().trim_start_matches('v').split('.');
    let mut next = || it.next().ok_or_else(bad)?.parse::<u64>().map_err(|_| bad());
    let out = (next()?, next()?, next()?);
    if it.next().is_some() {
        return Err(bad());
    }
    Ok(out)
}

impl ReleaseManifest {
    pub fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|a| a.name == name)
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let m: ReleaseManifest = serde_json::from_slice(b)?;
        if m.format != RELEASE_FORMAT {
            return Err(Error::Other(format!("unsupported release format {}", m.format)));
        }
        parse_version(&m.version)?;
        for a in &m.assets {
            // Asset names become file names to download; keep them plain.
            if a.name.is_empty() || a.name.contains(['/', '\\', ':']) || a.name.starts_with('.') {
                return Err(Error::Other(format!("bad asset name {:?}", a.name)));
            }
        }
        Ok(m)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// Hashes every `spunkpipe-*` archive in `dir`.
    pub fn from_dir(dir: &Path, version: &str) -> Result<Self> {
        parse_version(version)?;
        let mut assets = Vec::new();
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("spunkpipe-") || !(name.ends_with(".tar.gz") || name.ends_with(".zip")) {
                continue;
            }
            let f = std::fs::File::open(e.path())?;
            let size = f.metadata()?.len();
            assets.push(ReleaseAsset { name, sha256: sha256_hex(std::io::BufReader::new(f))?, size });
        }
        assets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ReleaseManifest { format: RELEASE_FORMAT, version: version.trim_start_matches('v').to_string(), assets })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(parse_version("0.10.0").unwrap() > parse_version("0.9.9").unwrap());
        assert_eq!(parse_version("v1.2.3").unwrap(), (1, 2, 3));
        for bad in ["1.2", "1.2.3.4", "a.b.c", "", "1.2.x"] {
            assert!(parse_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn builds_from_a_directory_and_round_trips() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("spunkpipe-0.2.0-x86_64-unknown-linux-musl.tar.gz"), b"abc").unwrap();
        std::fs::write(d.path().join("notes.txt"), b"ignored").unwrap();
        let m = ReleaseManifest::from_dir(d.path(), "v0.2.0").unwrap();
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.assets.len(), 1);
        // SHA-256 of "abc"
        assert_eq!(m.assets[0].sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(ReleaseManifest::from_bytes(&m.to_bytes().unwrap()).unwrap(), m);
    }

    #[test]
    fn rejects_path_like_asset_names() {
        let json = r#"{"format":1,"version":"1.0.0","assets":[{"name":"../x.tar.gz","sha256":"00","size":1}]}"#;
        assert!(ReleaseManifest::from_bytes(json.as_bytes()).is_err());
    }
}
