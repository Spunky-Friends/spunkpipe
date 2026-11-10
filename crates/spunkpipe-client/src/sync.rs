use crate::lock::{Lock, LockEntry};
use crate::source::{read_all, Source};
use spunkpipe_core::fsutil::move_file;
use spunkpipe_core::path::join_safe;
use spunkpipe_core::{
    manifest_path, par::par_map, signature_path, Error, FileEntry, Hash, Manifest, PublicKey,
    Result, LATEST,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_MANIFEST: u64 = 64 << 20;

pub struct SyncOptions {
    /// The managed folder (for 7 Days to Die: `<install>/Mods`).
    pub target: PathBuf,
    /// Where the lock file, staging area and backups live. Must not be inside `target`.
    pub state: PathBuf,
    pub game: String,
    /// Sync to a specific serial. Allows going backwards (rollback).
    pub serial: Option<u64>,
    /// Re-check every managed file even if the server serial has not changed.
    pub verify: bool,
    /// Work out what would change without touching anything.
    pub dry_run: bool,
    pub threads: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub serial: u64,
    /// The server had nothing newer and nothing was checked or changed.
    pub up_to_date: bool,
    /// Distinct blobs downloaded (or, in a dry run, that would be).
    pub fetched_blobs: usize,
    /// Compressed bytes transferred.
    pub fetched_bytes: u64,
    /// Files written into the managed folder.
    pub placed: usize,
    pub removed: usize,
    /// Files that already matched and were left alone.
    pub unchanged: usize,
    /// Pre-existing or locally modified files moved aside before being replaced or removed.
    pub backed_up: usize,
}

fn mtime_ns(md: &fs::Metadata) -> u128 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

pub fn fetch_latest(source: &dyn Source) -> Result<u64> {
    let b = read_all(source, LATEST, 32)?;
    String::from_utf8_lossy(&b)
        .trim()
        .parse()
        .map_err(|_| Error::Other("server's latest pointer is not a number".into()))
}

/// Downloads and verifies the manifest with the given serial. Nothing unsigned is ever parsed.
pub fn fetch_manifest(source: &dyn Source, key: &PublicKey, serial: u64) -> Result<Manifest> {
    let bytes = read_all(source, &manifest_path(serial), MAX_MANIFEST)?;
    let sig = read_all(source, &signature_path(serial), 1024)?;
    key.verify(&bytes, &String::from_utf8_lossy(&sig))?;
    let m = Manifest::from_bytes(&bytes)?;
    if m.serial != serial {
        return Err(Error::BadManifest(format!("manifest says serial {}, expected {serial}", m.serial)));
    }
    Ok(m)
}

enum Disk {
    Missing,
    /// A regular file with this content.
    File(Hash),
    /// A directory, symlink or something else that is not a plain file.
    Other,
}

fn inspect(dest: &Path, locked: Option<&LockEntry>) -> Result<Disk> {
    let md = match fs::symlink_metadata(dest) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Disk::Missing),
        Err(e) => return Err(e.into()),
    };
    if !md.is_file() {
        return Ok(Disk::Other);
    }
    if let Some(l) = locked {
        if l.size == md.len() && l.mtime_ns == mtime_ns(&md) {
            return Ok(Disk::File(l.hash));
        }
    }
    Ok(Disk::File(Hash::from_reader(io::BufReader::new(fs::File::open(dest)?))?))
}

struct Backups<'a> {
    dir: PathBuf,
    target: &'a Path,
    count: usize,
}

impl Backups<'_> {
    /// Moves whatever is at `rel` out of the managed folder so nothing of the user's is ever lost.
    fn save(&mut self, rel: &str) -> Result<()> {
        let from = join_safe(self.target, rel)?;
        let to = join_safe(&self.dir, rel)?;
        fs::create_dir_all(to.parent().unwrap())?;
        move_file(&from, &to)?;
        self.count += 1;
        Ok(())
    }
}

fn prune_empty_parents(target: &Path, file: &Path) {
    let mut p = file.parent();
    while let Some(dir) = p {
        if dir == target || !dir.starts_with(target) || fs::remove_dir(dir).is_err() {
            break;
        }
        p = dir.parent();
    }
}

/// Downloads one blob (resuming a partial download) and verifies it into `staging/<hash>`.
fn fetch_blob(source: &dyn Source, staging: &Path, hash: &Hash, size: u64) -> Result<u64> {
    let part = staging.join(format!("{hash}.zst.part"));
    let staged = staging.join(hash.to_hex());
    let mut transferred = 0;
    for attempt in 0..2 {
        let offset = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let mut reader = source.open(&hash.blob_path(), offset)?;
        let mut f = OpenOptions::new().create(true).append(true).open(&part)?;
        transferred += io::copy(&mut reader, &mut f)?;
        drop(f);
        match spunkpipe_core::store::decode_verified(&part, &staged, hash, size) {
            Ok(()) => {
                let _ = fs::remove_file(&part);
                return Ok(transferred);
            }
            // A bad partial file: throw it away and try once from the start.
            Err(Error::CorruptBlob(..) | Error::Io(_)) if attempt == 0 => {
                let _ = fs::remove_file(&part);
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

pub fn sync(source: &dyn Source, key: &PublicKey, opts: &SyncOptions) -> Result<Report> {
    if opts.state.starts_with(&opts.target) {
        return Err(Error::Other("the state folder must not be inside the managed folder".into()));
    }
    let mut lock = Lock::load(&opts.state)?;
    if !lock.game.is_empty() && lock.game != opts.game {
        return Err(Error::Other(format!(
            "state folder belongs to game {:?}, not {:?}",
            lock.game, opts.game
        )));
    }

    let latest = fetch_latest(source)?;
    let want = opts.serial.unwrap_or(latest);
    if opts.serial.is_none() && want < lock.serial {
        return Err(Error::Other(format!(
            "refusing to go backwards: server is at serial {want}, this machine is at {}",
            lock.serial
        )));
    }
    if opts.serial.is_none() && want == lock.serial && !opts.verify {
        return Ok(Report { serial: want, up_to_date: true, ..Default::default() });
    }

    let manifest = fetch_manifest(source, key, want)?;
    if manifest.game != opts.game {
        return Err(Error::BadManifest(format!("manifest is for game {:?}, not {:?}", manifest.game, opts.game)));
    }

    // Decide what each file needs. Hashing runs in parallel.
    let target = &opts.target;
    let inspected = par_map(&manifest.files, opts.threads, |f| -> Result<Disk> {
        inspect(&join_safe(target, &f.path)?, lock.files.get(&f.path))
    });
    let mut report = Report { serial: manifest.serial, ..Default::default() };
    // (entry, something-is-in-the-way)
    let mut to_place: Vec<(&FileEntry, bool)> = Vec::new();
    let mut satisfied: Vec<&FileEntry> = Vec::new();
    for (f, d) in manifest.files.iter().zip(inspected) {
        match d? {
            Disk::File(h) if h == f.hash => satisfied.push(f),
            Disk::Missing => to_place.push((f, false)),
            // A file we installed and the user has not touched is ours to replace.
            Disk::File(h) if lock.files.get(&f.path).is_some_and(|l| l.hash == h) => to_place.push((f, false)),
            // Anything else in the way belongs to the user: keep a copy.
            _ => to_place.push((f, true)),
        }
    }
    report.unchanged = satisfied.len();

    let wanted: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let stale: Vec<&String> = lock.files.keys().filter(|p| !wanted.contains(p.as_str())).collect();

    // Blobs to download, one per distinct content.
    let mut groups: BTreeMap<Hash, (u64, Vec<&FileEntry>)> = BTreeMap::new();
    for (f, _) in &to_place {
        groups.entry(f.hash).or_insert((f.size, Vec::new())).1.push(f);
    }

    if opts.dry_run {
        report.fetched_blobs = groups.len();
        report.placed = to_place.len();
        report.removed = stale.len();
        return Ok(report);
    }

    // Stage: download and verify everything before touching the managed folder.
    let staging = opts.state.join("staging");
    fs::create_dir_all(&staging)?;
    let jobs: Vec<(Hash, u64)> = groups.iter().map(|(h, (s, _))| (*h, *s)).collect();
    let fetched = par_map(&jobs, opts.threads, |(h, s)| fetch_blob(source, &staging, h, *s));
    for r in fetched {
        report.fetched_bytes += r?;
    }
    report.fetched_blobs = jobs.len();

    // Apply. Each file lands with one rename; the lock is written last, so an interrupted
    // run simply re-plans next time and finds the finished files already satisfied.
    let stamp = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut backups = Backups { dir: opts.state.join("backup").join(stamp.to_string()), target, count: 0 };
    let blocked: HashMap<&str, bool> = to_place.iter().map(|(f, b)| (f.path.as_str(), *b)).collect();
    for (hash, (_, entries)) in &groups {
        let staged = staging.join(hash.to_hex());
        for (i, f) in entries.iter().enumerate() {
            let dest = join_safe(target, &f.path)?;
            if blocked[f.path.as_str()] {
                backups.save(&f.path)?;
            }
            fs::create_dir_all(dest.parent().unwrap())?;
            if i + 1 == entries.len() {
                move_file(&staged, &dest)?;
            } else {
                fs::copy(&staged, &dest)?;
            }
            report.placed += 1;
        }
    }
    for rel in &stale {
        let dest = join_safe(target, rel)?;
        let locked = lock.files.get(rel.as_str());
        match inspect(&dest, locked)? {
            Disk::Missing => {}
            Disk::File(h) if locked.is_some_and(|l| l.hash == h) => {
                fs::remove_file(&dest)?;
                report.removed += 1;
            }
            // Modified or replaced since we installed it: keep a copy.
            _ => {
                backups.save(rel)?;
                report.removed += 1;
            }
        }
        prune_empty_parents(target, &dest);
    }
    report.backed_up = backups.count;

    let mut files = BTreeMap::new();
    for f in &manifest.files {
        let md = fs::metadata(join_safe(target, &f.path)?)?;
        files.insert(f.path.clone(), LockEntry { hash: f.hash, size: f.size, mtime_ns: mtime_ns(&md) });
    }
    lock.game = manifest.game.clone();
    lock.serial = manifest.serial;
    lock.files = files;
    lock.save(&opts.state)?;
    let _ = fs::remove_dir_all(&staging);
    Ok(report)
}
