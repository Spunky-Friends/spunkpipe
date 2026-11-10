use spunkpipe_client::{DirSource, Source};
use spunkpipe_core::{Result, SecretKey};
use spunkpipe_daemon::{Command, Engine, EngineConfig, GameProbe, Handle, Mode, Snapshot, Status};
use spunkpipe_server::{publish, PublishOptions};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Env {
    _d: tempfile::TempDir,
    src: PathBuf,
    web: PathBuf,
    target: PathBuf,
    state: PathBuf,
    key: SecretKey,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().to_path_buf();
    let e = Env {
        src: r.join("server/Mods"),
        web: r.join("web"),
        target: r.join("client/Mods"),
        state: r.join("state"),
        key: SecretKey::generate(),
        _d: d,
    };
    fs::create_dir_all(&e.src).unwrap();
    fs::create_dir_all(&e.target).unwrap();
    e
}

fn put(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn do_publish(e: &Env) -> u64 {
    publish(&PublishOptions {
        source: e.src.clone(),
        out: e.web.clone(),
        state: e.web.parent().unwrap().join("pubstate"),
        game: "7dtd".into(),
        key: SecretKey::from_base64(&e.key.to_base64()).unwrap(),
        force: false,
        keep: 10,
        threads: 2,
    })
    .unwrap()
    .serial
}

struct Flag(Arc<AtomicBool>);
impl GameProbe for Flag {
    fn running(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A source that hands out data slowly, so a download can be interrupted.
struct Slow {
    inner: DirSource,
    opens: Arc<AtomicUsize>,
}
struct Trickle(Box<dyn Read>);
impl Read for Trickle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_millis(2));
        let n = buf.len().min(4096);
        self.0.read(&mut buf[..n])
    }
}
impl Source for Slow {
    fn open(&self, rel: &str, offset: u64) -> Result<Box<dyn Read>> {
        if rel.starts_with("blobs/") {
            self.opens.fetch_add(1, Ordering::Relaxed);
            return Ok(Box::new(Trickle(self.inner.open(rel, offset)?)));
        }
        self.inner.open(rel, offset)
    }
}

fn start(e: &Env, mode: Mode, running: &Arc<AtomicBool>, source: Arc<dyn Source + Send + Sync>) -> Handle {
    Engine::spawn(
        EngineConfig {
            game: "7dtd".into(),
            target: e.target.clone(),
            state: e.state.clone(),
            mode,
            poll: Duration::from_millis(50),
            tick: Duration::from_millis(20),
        },
        source,
        e.key.public(),
        Arc::new(Flag(running.clone())),
        Arc::new(|_: &Snapshot| {}),
    )
}

fn wait_for(h: &Handle, what: &str, f: impl Fn(&Snapshot) -> bool) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if f(&h.snapshot()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}; last: {:?}", h.snapshot());
}

fn dir_source(e: &Env) -> Arc<dyn Source + Send + Sync> {
    Arc::new(DirSource(e.web.clone()))
}

fn idle(serial: u64) -> impl Fn(&Snapshot) -> bool {
    move |s| s.status == Status::Idle { serial }
}

#[test]
fn auto_downloads_and_applies() {
    let e = env();
    put(&e.src, "M/a.xml", b"one");
    do_publish(&e);
    let h = start(&e, Mode::Auto, &Arc::new(AtomicBool::new(false)), dir_source(&e));
    wait_for(&h, "idle at 1", idle(1));
    assert_eq!(fs::read(e.target.join("M/a.xml")).unwrap(), b"one");

    // A later publish is picked up on a later poll, with no prompting.
    put(&e.src, "M/a.xml", b"two");
    do_publish(&e);
    wait_for(&h, "idle at 2", idle(2));
    assert_eq!(fs::read(e.target.join("M/a.xml")).unwrap(), b"two");
    h.quit();
}

#[test]
fn manual_announces_then_waits_for_the_user() {
    let e = env();
    put(&e.src, "M/a.xml", b"one");
    do_publish(&e);
    let h = start(&e, Mode::Manual, &Arc::new(AtomicBool::new(false)), dir_source(&e));
    wait_for(&h, "update available", |s| matches!(s.status, Status::UpdateAvailable { serial: 1, files: 1, .. }));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!e.target.join("M/a.xml").exists(), "manual mode must not download by itself");
    assert!(matches!(h.snapshot().status, Status::UpdateAvailable { .. }));

    h.send(Command::Update);
    wait_for(&h, "idle at 1", idle(1));
    assert!(e.target.join("M/a.xml").exists());
    h.quit();
}

#[test]
fn paused_does_nothing_until_asked() {
    let e = env();
    put(&e.src, "M/a.xml", b"one");
    do_publish(&e);
    let h = start(&e, Mode::Paused, &Arc::new(AtomicBool::new(false)), dir_source(&e));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(h.snapshot().status, Status::Starting, "no automatic check while paused");

    h.send(Command::CheckNow);
    wait_for(&h, "update available", |s| matches!(s.status, Status::UpdateAvailable { .. }));
    std::thread::sleep(Duration::from_millis(200));
    assert!(!e.target.join("M/a.xml").exists(), "a manual check must not download while paused");

    h.send(Command::SetMode(Mode::Auto));
    wait_for(&h, "idle at 1", idle(1));
    h.quit();
}

#[test]
fn apply_waits_for_the_game_to_close() {
    let e = env();
    put(&e.src, "M/a.xml", b"one");
    do_publish(&e);
    let running = Arc::new(AtomicBool::new(true));
    let h = start(&e, Mode::Auto, &running, dir_source(&e));
    wait_for(&h, "waiting for game", |s| s.status == Status::WaitingForGame { serial: 1 });
    assert!(!e.target.join("M/a.xml").exists(), "files must not change while the game runs");

    running.store(false, Ordering::Relaxed);
    wait_for(&h, "idle at 1", idle(1));
    assert!(e.target.join("M/a.xml").exists());
    h.quit();
}

#[test]
fn pausing_mid_download_keeps_progress_and_resumes() {
    let e = env();
    // Incompressible data so the download takes a while through the throttled source.
    let mut x = 0x9E3779B97F4A7C15u64;
    let big: Vec<u8> = (0..1_500_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    put(&e.src, "M/big.bin", &big);
    do_publish(&e);
    let opens = Arc::new(AtomicUsize::new(0));
    let src = Arc::new(Slow { inner: DirSource(e.web.clone()), opens: opens.clone() });
    let h = start(&e, Mode::Auto, &Arc::new(AtomicBool::new(false)), src);

    wait_for(&h, "downloading", |s| matches!(s.status, Status::Downloading { .. }));
    std::thread::sleep(Duration::from_millis(100));
    h.send(Command::SetMode(Mode::Paused));
    wait_for(&h, "paused", |s| s.mode == Mode::Paused && matches!(s.status, Status::UpdateAvailable { .. }));
    let done = h.progress().bytes_done.load(Ordering::Relaxed);
    assert!(done > 0, "some bytes arrived before the pause");
    assert!(!e.target.join("M/big.bin").exists());
    let part = fs::read_dir(e.state.join("staging")).unwrap().filter_map(|f| f.ok()).any(|f| f.file_name().to_string_lossy().ends_with(".zst.part"));
    assert!(part, "partial download kept");

    h.send(Command::SetMode(Mode::Auto));
    wait_for(&h, "idle at 1", idle(1));
    assert_eq!(fs::read(e.target.join("M/big.bin")).unwrap(), big);
    assert!(opens.load(Ordering::Relaxed) >= 2, "second open resumes from the part file");
    h.quit();
}

#[test]
fn bad_server_reports_an_error_and_recovers() {
    let e = env();
    let h = start(&e, Mode::Auto, &Arc::new(AtomicBool::new(false)), dir_source(&e));
    wait_for(&h, "error", |s| matches!(s.status, Status::Error(_)));
    put(&e.src, "M/a.xml", b"one");
    do_publish(&e);
    wait_for(&h, "idle at 1", idle(1));
    h.quit();
}
