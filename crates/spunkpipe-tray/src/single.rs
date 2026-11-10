//! Only one tray at a time per user.

use spunkpipe_daemon::paths;
use std::fs::{File, OpenOptions, TryLockError};
use std::time::{Duration, Instant};

/// Takes the per-user lock, waiting up to `wait` for another tray to finish exiting.
/// Hold the returned file for as long as the tray runs.
pub fn acquire(wait: Duration) -> Option<File> {
    let dir = paths::state_dir("tray")?;
    std::fs::create_dir_all(&dir).ok()?;
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("tray.lock")).ok()?;
    let start = Instant::now();
    loop {
        match f.try_lock() {
            Ok(()) => return Some(f),
            Err(TryLockError::WouldBlock) if start.elapsed() < wait => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return None,
        }
    }
}
