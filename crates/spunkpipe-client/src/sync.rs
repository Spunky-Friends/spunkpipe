//! The sync engine, in three steps so a daemon can pause between them:
//! [`check`] (cheap, decides what is needed), [`stage`] (downloads and verifies, can be
//! cancelled and resumed) and [`apply`] (moves files into the managed folder).

use crate::lock::{Lock, LockEntry};
use crate::source::{read_all, Source};
use spunkpipe_core::fsutil::move_file;
use spunkpipe_core::path::join_safe;
use spunkpipe_core::{
    manifest_path, par::par_map, signature_path, Error, FileEntry, Hash, Manifest, PublicKey,
    Result, LATEST,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

const MAX_MANIFEST: u64 = 64 << 20;
const CHUNK: usize = 64 * 1024;

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
    /// Work out what would change without touching anything (used by [`sync`]).
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

/// Live download progress, safe to read from another thread.
#[derive(Default)]
pub struct Progress {
    pub blobs_total: AtomicUsize,
    pub blobs_done: AtomicUsize,
    /// Compressed bytes, as stored on the server.
    pub bytes_total: AtomicU64,
    pub bytes_done: AtomicU64,
}

impl Progress {
    pub fn reset(&self, blobs: usize, bytes: u64) {
        self.blobs_total.store(blobs, Ordering::Relaxed);
        self.blobs_done.store(0, Ordering::Relaxed);
        self.bytes_total.store(bytes, Ordering::Relaxed);
        self.bytes_done.store(0, Ordering::Relaxed);
    }

    /// 0..=100
    pub fn percent(&self) -> u8 {
        let total = self.bytes_total.load(Ordering::Relaxed).max(1);
        (self.bytes_done.load(Ordering::Relaxed).min(total) * 100 / total) as u8
    }
}

/// What a check found. Holds everything needed to stage and apply without asking again.
#[derive(Debug)]
pub struct Pending {
    manifest: Manifest,
    place: Vec<FileEntry>,
    stale: Vec<String>,
    unchanged: usize,
    /// One download per distinct content: hash -> (size, stored size, indexes into `place`).
    groups: BTreeMap<Hash, (u64, u64, Vec<usize>)>,
}

impl Pending {
    pub fn serial(&self) -> u64 {
        self.manifest.serial
    }
    pub fn files_to_place(&self) -> usize {
        self.place.len()
    }
    pub fn files_to_remove(&self) -> usize {
        self.stale.len()
    }
    pub fn unchanged(&self) -> usize {
        self.unchanged
    }
    pub fn blobs(&self) -> usize {
        self.groups.len()
    }
    /// Compressed bytes that would be downloaded (before counting any partial downloads).
    pub fn download_bytes(&self) -> u64 {
        self.groups.values().map(|(_, stored, _)| stored).sum()
    }
}

#[derive(Debug)]
pub enum Check {
    UpToDate(u64),
    Pending(Pending),
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
        let from = checked_dest(self.target, rel)?;
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

/// Joins `rel` onto `target` and refuses if any folder on the way is a symlink, so a link can
/// never redirect a write or delete outside the managed folder. (Windows junctions are not
/// detected by `is_symlink`; the managed folder is expected to be a normal game folder.)
fn checked_dest(target: &Path, rel: &str) -> Result<PathBuf> {
    let dest = join_safe(target, rel)?;
    let mut cur = target.to_path_buf();
    let comps: Vec<&str> = rel.split('/').collect();
    for comp in &comps[..comps.len() - 1] {
        cur.push(comp);
        match fs::symlink_metadata(&cur) {
            Ok(md) if md.file_type().is_symlink() => {
                return Err(Error::Other(format!("refusing to write through symlink {}", cur.display())));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(dest)
}

/// Copies `reader` to `out` in chunks, checking `cancel` between chunks. At most `limit` bytes
/// are copied; the return value says how many, so a caller can tell an oversized stream.
fn copy_cancellable(
    reader: &mut dyn Read,
    out: &mut dyn Write,
    limit: u64,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<u64> {
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    while total < limit {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let want = buf.len().min((limit - total) as usize);
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        total += n as u64;
        progress.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(total)
}

/// Downloads one blob (resuming a partial download) and verifies it into `staging/<hash>`.
fn fetch_blob(
    source: &dyn Source,
    staging: &Path,
    hash: &Hash,
    size: u64,
    stored: u64,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<u64> {
    let part = staging.join(format!("{hash}.zst.part"));
    let staged = staging.join(hash.to_hex());
    let tmp = staging.join(format!("{hash}.tmp"));
    if staged.exists() {
        // Left by an earlier run that was stopped before applying. Never trust it blindly:
        // re-check size and hash, and throw it away if it does not match.
        let ok = fs::metadata(&staged).is_ok_and(|m| m.len() == size)
            && fs::File::open(&staged)
                .and_then(|f| Hash::from_reader(io::BufReader::new(f)))
                .is_ok_and(|h| &h == hash);
        if ok {
            progress.bytes_done.fetch_add(stored, Ordering::Relaxed);
            progress.blobs_done.fetch_add(1, Ordering::Relaxed);
            return Ok(0);
        }
        let _ = fs::remove_file(&staged);
    }
    let mut transferred = 0;
    // A valid blob cannot be larger than zstd's worst case for `size` bytes (plus a little framing).
    let max_total = zstd::zstd_safe::compress_bound(size as usize) as u64 + 1024;
    for attempt in 0..2 {
        let offset = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if offset > max_total {
            let _ = fs::remove_file(&part);
            continue;
        }
        let mut reader = source.open(&hash.blob_path(), offset)?;
        let mut f = OpenOptions::new().create(true).append(true).open(&part)?;
        progress.bytes_done.fetch_add(offset, Ordering::Relaxed);
        // Read one byte past the limit so an oversized stream is detected, not silently truncated.
        let n = copy_cancellable(&mut reader, &mut f, max_total - offset + 1, cancel, progress)?;
        drop(f);
        transferred += n;
        if offset + n > max_total {
            let _ = fs::remove_file(&part);
            return Err(Error::CorruptBlob(hash.to_hex(), "blob larger than its declared size allows".into()));
        }
        // Decode to a temporary name and rename only once verified, so a crash mid-decode can
        // never leave a partial file under the final staged name.
        let verified = spunkpipe_core::store::decode_verified(&part, &tmp, hash, size).and_then(|()| {
            fs::rename(&tmp, &staged).map_err(Error::from)
        });
        match verified {
            Ok(()) => {
                let _ = fs::remove_file(&part);
                progress.blobs_done.fetch_add(1, Ordering::Relaxed);
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

/// Step 1: asks the server what is current and works out what this machine needs.
/// Only a small amount of data is fetched unless the serial changed (or `verify` is set).
pub fn check(source: &dyn Source, key: &PublicKey, opts: &SyncOptions) -> Result<Check> {
    if opts.state.starts_with(&opts.target) {
        return Err(Error::Other("the state folder must not be inside the managed folder".into()));
    }
    let lock = Lock::load(&opts.state)?;
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
        return Ok(Check::UpToDate(want));
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
    let mut place = Vec::new();
    let mut unchanged = 0;
    for (f, d) in manifest.files.iter().zip(inspected) {
        match d? {
            Disk::File(h) if h == f.hash => unchanged += 1,
            _ => place.push(f.clone()),
        }
    }
    let wanted: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let stale: Vec<String> = lock.files.keys().filter(|p| !wanted.contains(p.as_str())).cloned().collect();

    let mut groups: BTreeMap<Hash, (u64, u64, Vec<usize>)> = BTreeMap::new();
    for (i, f) in place.iter().enumerate() {
        groups.entry(f.hash).or_insert((f.size, f.stored, Vec::new())).2.push(i);
    }
    Ok(Check::Pending(Pending { manifest, place, stale, unchanged, groups }))
}

/// Step 2: downloads and verifies everything needed into the state folder's staging area.
/// Nothing in the managed folder is touched. Set `cancel` to stop; progress is kept, so a
/// later call resumes where this one left off.
pub fn stage(
    source: &dyn Source,
    opts: &SyncOptions,
    pending: &Pending,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<u64> {
    let staging = opts.state.join("staging");
    fs::create_dir_all(&staging)?;
    progress.reset(pending.groups.len(), pending.download_bytes());
    let jobs: Vec<(Hash, u64, u64)> = pending.groups.iter().map(|(h, (s, st, _))| (*h, *s, *st)).collect();
    let results = par_map(&jobs, opts.threads, |(h, s, st)| fetch_blob(source, &staging, h, *s, *st, cancel, progress));
    let mut transferred = 0;
    let mut first_err = None;
    for r in results {
        match r {
            Ok(n) => transferred += n,
            // A cancel beats any other error: the user asked to stop.
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(transferred),
    }
}

/// Step 3: moves staged files into the managed folder and records them in the lock.
///
/// Each file lands with one rename and the lock is written last, so an interrupted run simply
/// re-plans next time and finds the finished files already satisfied. The caller must make sure
/// the game is not running: on Windows its files are locked while it runs.
pub fn apply(opts: &SyncOptions, pending: &Pending) -> Result<Report> {
    let target = opts.target.as_path();
    let staging = opts.state.join("staging");
    let mut lock = Lock::load(&opts.state)?;
    let mut report = Report {
        serial: pending.manifest.serial,
        unchanged: pending.unchanged,
        fetched_blobs: pending.groups.len(),
        ..Default::default()
    };
    let stamp = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut backups = Backups { dir: opts.state.join("backup").join(stamp.to_string()), target, count: 0 };

    for (hash, (_, _, idxs)) in &pending.groups {
        let staged = staging.join(hash.to_hex());
        if !staged.exists() {
            return Err(Error::NotFound(format!("staged file for {hash}; stage again")));
        }
        for (i, &idx) in idxs.iter().enumerate() {
            let f = &pending.place[idx];
            let dest = checked_dest(target, &f.path)?;
            // Re-inspect now: the disk may have changed since the check. A file we installed and
            // the user has not touched is ours to replace; anything else in the way is theirs.
            let ours = match inspect(&dest, lock.files.get(&f.path))? {
                Disk::Missing => true,
                Disk::File(h) => lock.files.get(&f.path).is_some_and(|l| l.hash == h) || h == f.hash,
                Disk::Other => false,
            };
            if !ours {
                backups.save(&f.path)?;
            }
            fs::create_dir_all(dest.parent().unwrap())?;
            if i + 1 == idxs.len() {
                move_file(&staged, &dest)?;
            } else {
                fs::copy(&staged, &dest)?;
            }
            report.placed += 1;
        }
    }
    for rel in &pending.stale {
        let dest = checked_dest(target, rel)?;
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
    for f in &pending.manifest.files {
        let md = fs::metadata(join_safe(target, &f.path)?)?;
        files.insert(f.path.clone(), LockEntry { hash: f.hash, size: f.size, mtime_ns: mtime_ns(&md) });
    }
    lock.game = pending.manifest.game.clone();
    lock.serial = pending.manifest.serial;
    lock.files = files;
    lock.save(&opts.state)?;
    let _ = fs::remove_dir_all(&staging);
    Ok(report)
}

/// Check, stage and apply in one go (what the `sync` command does).
pub fn sync(source: &dyn Source, key: &PublicKey, opts: &SyncOptions) -> Result<Report> {
    let pending = match check(source, key, opts)? {
        Check::UpToDate(serial) => return Ok(Report { serial, up_to_date: true, ..Default::default() }),
        Check::Pending(p) => p,
    };
    if opts.dry_run {
        return Ok(Report {
            serial: pending.serial(),
            fetched_blobs: pending.blobs(),
            fetched_bytes: pending.download_bytes(),
            placed: pending.files_to_place(),
            removed: pending.files_to_remove(),
            unchanged: pending.unchanged(),
            ..Default::default()
        });
    }
    let fetched = stage(source, opts, &pending, &AtomicBool::new(false), &Progress::default())?;
    let mut report = apply(opts, &pending)?;
    report.fetched_bytes = fetched;
    Ok(report)
}
