//! The publisher: scans a mods folder and writes a signed manifest plus blobs to a web root.

use serde::{Deserialize, Serialize};
use spunkpipe_core::{
    fsutil::atomic_write, manifest_path, signature_path, store, Error, FileEntry, Hash, Manifest,
    Result, SecretKey, LATEST,
};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct PublishOptions {
    /// The folder to publish (for 7 Days to Die: the server's `Mods` folder).
    pub source: PathBuf,
    /// The web root to write into.
    pub out: PathBuf,
    pub game: String,
    pub key: SecretKey,
    /// Publish even if nothing changed.
    pub force: bool,
    /// Number of old manifests to keep for rollback.
    pub keep: usize,
    pub threads: usize,
}

#[derive(Debug)]
pub struct PublishResult {
    pub serial: u64,
    pub files: usize,
    pub new_blobs: usize,
    /// False when nothing changed and no new manifest was written.
    pub published: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    // path -> (size, mtime_ns, hash)
    entries: BTreeMap<String, (u64, u128, Hash)>,
}

fn mtime_ns(md: &fs::Metadata) -> u128 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn read_serial(out: &Path) -> Result<Option<u64>> {
    match fs::read_to_string(out.join(LATEST)) {
        Ok(s) => s
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| Error::Other(format!("{LATEST} does not hold a serial number"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

struct Scanned {
    rel: String,
    abs: PathBuf,
    size: u64,
    mtime: u128,
}

fn scan(source: &Path) -> Result<Vec<Scanned>> {
    let mut out = Vec::new();
    for entry in WalkDir::new(source).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|e| Error::Other(e.to_string()))?;
        let ft = entry.file_type();
        if ft.is_dir() {
            continue;
        }
        if !ft.is_file() {
            // Symlinks and special files are never published.
            continue;
        }
        let rel = entry.path().strip_prefix(source).unwrap();
        let rel = rel
            .components()
            .map(|c| {
                c.as_os_str()
                    .to_str()
                    .map(str::to_string)
                    .ok_or_else(|| Error::Other(format!("non-UTF-8 file name: {}", entry.path().display())))
            })
            .collect::<Result<Vec<_>>>()?
            .join("/");
        spunkpipe_core::path::validate_rel_path(&rel)?;
        let md = entry.metadata().map_err(|e| Error::Other(e.to_string()))?;
        out.push(Scanned { rel, abs: entry.path().to_path_buf(), size: md.len(), mtime: mtime_ns(&md) });
    }
    Ok(out)
}

pub fn publish(opts: &PublishOptions) -> Result<PublishResult> {
    if !opts.source.is_dir() {
        return Err(Error::Other(format!("{} is not a directory", opts.source.display())));
    }
    fs::create_dir_all(&opts.out)?;
    let cache_path = opts.out.join(".publish-cache.json");
    let cache: Cache = fs::read(&cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let scanned = scan(&opts.source)?;
    let results = spunkpipe_core::par::par_map(&scanned, opts.threads, |s| -> Result<(FileEntry, bool)> {
        if let Some(&(size, mtime, hash)) = cache.entries.get(&s.rel) {
            if size == s.size && mtime == s.mtime && opts.out.join(hash.blob_path()).exists() {
                return Ok((FileEntry { path: s.rel.clone(), hash, size }, false));
            }
        }
        let (hash, size, new) = store::add_file(&opts.out, &s.abs)?;
        Ok((FileEntry { path: s.rel.clone(), hash, size }, new))
    });

    let mut files = Vec::with_capacity(results.len());
    let mut new_blobs = 0;
    for r in results {
        let (entry, new) = r?;
        new_blobs += new as usize;
        files.push(entry);
    }

    let new_cache = Cache {
        entries: scanned
            .iter()
            .zip(&files)
            .map(|(s, f)| (s.rel.clone(), (f.size, s.mtime, f.hash)))
            .collect(),
    };
    atomic_write(&cache_path, &serde_json::to_vec(&new_cache)?)?;

    let prev = read_serial(&opts.out)?;
    if let Some(prev_serial) = prev {
        let prev_bytes = fs::read(opts.out.join(manifest_path(prev_serial)))?;
        let prev_manifest = Manifest::from_bytes(&prev_bytes)?;
        let mut sorted = files.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        if !opts.force && prev_manifest.game == opts.game && prev_manifest.files == sorted {
            return Ok(PublishResult { serial: prev_serial, files: files.len(), new_blobs, published: false });
        }
    }

    let serial = prev.unwrap_or(0) + 1;
    let created = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let manifest = Manifest::new(&opts.game, serial, created, files);
    manifest.validate()?;
    let bytes = manifest.to_bytes()?;
    let sig = opts.key.sign(&bytes);

    // Immutable files first; `latest` last, so a client never sees a serial whose files are missing.
    atomic_write(&opts.out.join(manifest_path(serial)), &bytes)?;
    atomic_write(&opts.out.join(signature_path(serial)), sig.as_bytes())?;
    atomic_write(&opts.out.join(LATEST), format!("{serial}\n").as_bytes())?;

    prune(&opts.out, serial, opts.keep.max(1))?;
    Ok(PublishResult { serial, files: manifest.files.len(), new_blobs, published: true })
}

/// Removes manifests older than the newest `keep`, and blobs no remaining manifest references.
fn prune(out: &Path, latest: u64, keep: usize) -> Result<()> {
    let mut kept = Vec::new();
    let dir = out.join("manifests");
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".json") else { continue };
        let Ok(n) = stem.parse::<u64>() else { continue };
        if latest.saturating_sub(n) >= keep as u64 {
            let _ = fs::remove_file(dir.join(&name));
            let _ = fs::remove_file(dir.join(format!("{n}.sig")));
        } else {
            kept.push(n);
        }
    }
    let mut referenced: HashSet<String> = HashSet::new();
    for n in kept {
        let m = Manifest::from_bytes(&fs::read(out.join(manifest_path(n)))?)?;
        referenced.extend(m.files.iter().map(|f| f.hash.to_hex()));
    }
    let blobs = out.join("blobs");
    for shard in fs::read_dir(&blobs)? {
        let shard = shard?;
        if !shard.file_type()?.is_dir() || shard.file_name() == ".tmp" {
            continue;
        }
        for blob in fs::read_dir(shard.path())? {
            let blob = blob?;
            let name = blob.file_name().to_string_lossy().into_owned();
            if let Some(hex) = name.strip_suffix(".zst") {
                if !referenced.contains(hex) {
                    let _ = fs::remove_file(blob.path());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(src: &Path, out: &Path, key: SecretKey) -> PublishOptions {
        PublishOptions { source: src.into(), out: out.into(), game: "7dtd".into(), key, force: false, keep: 2, threads: 4 }
    }

    #[test]
    fn publishes_and_skips_when_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let (src, out) = (d.path().join("src"), d.path().join("out"));
        fs::create_dir_all(src.join("A")).unwrap();
        fs::write(src.join("A/ModInfo.xml"), "<x/>").unwrap();
        let key = SecretKey::from_base64(&SecretKey::generate().to_base64()).unwrap();

        let r1 = publish(&opts(&src, &out, SecretKey::from_base64(&key.to_base64()).unwrap())).unwrap();
        assert!(r1.published);
        assert_eq!((r1.serial, r1.new_blobs), (1, 1));
        let r2 = publish(&opts(&src, &out, SecretKey::from_base64(&key.to_base64()).unwrap())).unwrap();
        assert!(!r2.published);
        assert_eq!(r2.serial, 1);

        fs::write(src.join("A/ModInfo.xml"), "<y/>").unwrap();
        let r3 = publish(&opts(&src, &out, key)).unwrap();
        assert!(r3.published);
        assert_eq!((r3.serial, r3.new_blobs), (2, 1));
    }

    #[test]
    fn prunes_old_manifests_and_blobs() {
        let d = tempfile::tempdir().unwrap();
        let (src, out) = (d.path().join("src"), d.path().join("out"));
        fs::create_dir_all(&src).unwrap();
        let key = SecretKey::generate().to_base64();
        let mut first_blob = None;
        for i in 1..=4u32 {
            fs::write(src.join("f"), format!("version {i}")).unwrap();
            let r = publish(&opts(&src, &out, SecretKey::from_base64(&key).unwrap())).unwrap();
            assert_eq!(r.serial, i as u64);
            if i == 1 {
                first_blob = Some(Hash::of(b"version 1").blob_path());
            }
        }
        assert!(!out.join(manifest_path(1)).exists());
        assert!(!out.join(manifest_path(2)).exists());
        assert!(out.join(manifest_path(3)).exists() && out.join(manifest_path(4)).exists());
        assert!(!out.join(first_blob.unwrap()).exists());
        assert!(out.join(Hash::of(b"version 3").blob_path()).exists());
    }
}
