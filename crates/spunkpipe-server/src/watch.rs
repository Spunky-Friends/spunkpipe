//! Re-publish automatically after the source folder has been quiet for a while, so a mod that
//! is still being copied in is never published half-finished.

use crate::{publish, PublishOptions, PublishResult};
use spunkpipe_core::Result;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

/// A cheap summary of every file's path, size and modification time.
pub fn fingerprint(source: &Path) -> Result<[u8; 32]> {
    let mut h = blake3::Hasher::new();
    for e in WalkDir::new(source).follow_links(false).sort_by_file_name() {
        let e = e.map_err(|e| spunkpipe_core::Error::Other(e.to_string()))?;
        if !e.file_type().is_file() {
            continue;
        }
        let md = e.metadata().map_err(|e| spunkpipe_core::Error::Other(e.to_string()))?;
        h.update(e.path().to_string_lossy().as_bytes());
        h.update(&md.len().to_le_bytes());
        let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        h.update(&mtime.to_le_bytes());
    }
    Ok(*h.finalize().as_bytes())
}

/// The source folder's fingerprint plus the exclude list's contents, so editing the list is
/// noticed too.
fn state_of(opts: &PublishOptions) -> Option<[u8; 32]> {
    let mut h = blake3::Hasher::new();
    h.update(&fingerprint(&opts.source).ok()?);
    if let Some(f) = &opts.exclude {
        h.update(&std::fs::read(f).unwrap_or_default());
    }
    Some(*h.finalize().as_bytes())
}

/// Publishes once straight away, then again whenever the folder changes and then stays still for
/// `quiet`. Runs until `stop` is set. `on_result` is called after each publish attempt.
pub fn watch(
    opts: &PublishOptions,
    quiet: Duration,
    poll: Duration,
    stop: &AtomicBool,
    on_result: impl Fn(&Result<PublishResult>),
) {
    on_result(&publish(opts));
    let mut seen = state_of(opts);
    let mut changed_at: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(poll);
        let now = state_of(opts);
        if now != seen {
            seen = now;
            changed_at = Some(Instant::now());
        }
        if changed_at.is_some_and(|t| t.elapsed() >= quiet) {
            changed_at = None;
            on_result(&publish(opts));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spunkpipe_core::SecretKey;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    #[test]
    fn publishes_once_after_a_burst_of_changes() {
        let d = tempfile::tempdir().unwrap();
        let (src, out) = (d.path().join("src"), d.path().join("out"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a"), "1").unwrap();
        let opts = PublishOptions {
            source: src.clone(),
            out: out.clone(),
            state: d.path().join("state"),
            game: "7dtd".into(),
            key: SecretKey::generate(),
            force: false,
            keep: 5,
            threads: 2,
            exclude: None,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let published = Arc::new(AtomicUsize::new(0));
        let (s2, p2) = (stop.clone(), published.clone());
        let t = std::thread::spawn(move || {
            watch(&opts, Duration::from_millis(300), Duration::from_millis(30), &s2, |r| {
                if r.as_ref().is_ok_and(|r| r.published) {
                    p2.fetch_add(1, Ordering::Relaxed);
                }
            })
        });
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(published.load(Ordering::Relaxed), 1, "initial publish");
        // A burst of edits, each inside the quiet window, must produce one publish after it ends.
        for i in 0..5 {
            std::fs::write(src.join("a"), format!("v{i}")).unwrap();
            std::thread::sleep(Duration::from_millis(60));
        }
        assert_eq!(published.load(Ordering::Relaxed), 1, "still quiet-waiting");
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(published.load(Ordering::Relaxed), 2);
        stop.store(true, Ordering::Relaxed);
        t.join().unwrap();
        assert_eq!(std::fs::read_to_string(out.join("latest")).unwrap().trim(), "2");
    }
}
