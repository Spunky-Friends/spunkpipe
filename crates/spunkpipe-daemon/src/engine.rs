//! One engine per game: a thread that owns the update state machine.
//!
//! ```text
//! check -> UpdateAvailable -> (Auto, or the user says Update) -> Downloading -> staged
//!       -> game running? WaitingForGame : Applying -> Idle
//! ```
//! Downloading happens on a worker thread so a pause or quit takes effect immediately; partial
//! downloads are kept and resumed.

use crate::config::Mode;
use crate::game::GameProbe;
use spunkpipe_client::{apply, check, stage, Check, Pending, Progress, Source, SyncOptions};
use spunkpipe_core::{par::default_threads, Error, PublicKey};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Just started, nothing checked yet.
    Starting,
    Checking,
    /// Everything matches the server.
    Idle { serial: u64 },
    /// An update exists and has not been downloaded.
    UpdateAvailable { serial: u64, files: usize, bytes: u64 },
    Downloading { serial: u64 },
    /// Downloaded and verified; waiting for the game to close before swapping files in.
    WaitingForGame { serial: u64 },
    Applying { serial: u64 },
    /// The last check or download failed. It is retried on the next check.
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: Mode,
    pub status: Status,
}

#[derive(Clone, Debug)]
pub enum Command {
    /// Check the server now, whatever the mode.
    CheckNow,
    SetMode(Mode),
    /// Download (if needed) and apply the pending update now, whatever the mode.
    Update,
    Quit,
}

enum Msg {
    Cmd(Command),
    StageDone { generation: u64, result: Result<u64, Error> },
}

pub struct EngineConfig {
    pub game: String,
    pub target: PathBuf,
    pub state: PathBuf,
    pub mode: Mode,
    /// Time between automatic checks.
    pub poll: Duration,
    /// How often the engine wakes to look at the clock and the game. 5 seconds is plenty.
    pub tick: Duration,
}

/// Control and observe a running engine.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Msg>,
    snapshot: Arc<Mutex<Snapshot>>,
    progress: Arc<Progress>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Handle {
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(Msg::Cmd(c));
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }

    /// Download progress, meaningful while the status is `Downloading`.
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Stops the engine and waits for it.
    pub fn quit(&self) {
        self.send(Command::Quit);
        if let Some(j) = self.join.lock().unwrap().take() {
            let _ = j.join();
        }
    }
}

pub struct Engine;

type OnChange = Arc<dyn Fn(&Snapshot) + Send + Sync>;

struct Worker {
    cancel: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

struct State {
    cfg: EngineConfig,
    opts: SyncOptions,
    source: Arc<dyn Source + Send + Sync>,
    key: PublicKey,
    probe: Arc<dyn GameProbe>,
    tx: Sender<Msg>,
    snapshot: Arc<Mutex<Snapshot>>,
    progress: Arc<Progress>,
    on_change: OnChange,
    mode: Mode,
    pending: Option<Arc<Pending>>,
    staged: bool,
    worker: Option<Worker>,
    generation: u64,
    last_check: Option<Instant>,
}

impl Engine {
    pub fn spawn(
        cfg: EngineConfig,
        source: Arc<dyn Source + Send + Sync>,
        key: PublicKey,
        probe: Arc<dyn GameProbe>,
        on_change: OnChange,
    ) -> Handle {
        let (tx, rx) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(Snapshot { mode: cfg.mode, status: Status::Starting }));
        let progress = Arc::new(Progress::default());
        let opts = SyncOptions {
            target: cfg.target.clone(),
            state: cfg.state.clone(),
            game: cfg.game.clone(),
            serial: None,
            verify: false,
            dry_run: false,
            threads: default_threads(),
        };
        let mut st = State {
            mode: cfg.mode,
            cfg,
            opts,
            source,
            key,
            probe,
            tx: tx.clone(),
            snapshot: snapshot.clone(),
            progress: progress.clone(),
            on_change,
            pending: None,
            staged: false,
            worker: None,
            generation: 0,
            last_check: None,
        };
        let join = std::thread::Builder::new()
            .name(format!("spunkpipe-{}", st.cfg.game))
            .spawn(move || st.run(rx))
            .expect("spawn engine thread");
        Handle { tx, snapshot, progress, join: Arc::new(Mutex::new(Some(join))) }
    }
}

impl State {
    fn set(&self, status: Status) {
        let snap = Snapshot { mode: self.mode, status };
        *self.snapshot.lock().unwrap() = snap.clone();
        (self.on_change)(&snap);
    }

    fn status(&self) -> Status {
        self.snapshot.lock().unwrap().status.clone()
    }

    fn run(&mut self, rx: mpsc::Receiver<Msg>) {
        self.set(Status::Starting);
        // The first automatic check happens straight away.
        if self.mode != Mode::Paused {
            self.do_check();
        }
        loop {
            match rx.recv_timeout(self.cfg.tick) {
                Ok(Msg::Cmd(Command::Quit)) => break,
                Ok(Msg::Cmd(Command::CheckNow)) => self.do_check(),
                Ok(Msg::Cmd(Command::SetMode(m))) => self.set_mode(m),
                Ok(Msg::Cmd(Command::Update)) => self.begin_update(),
                Ok(Msg::StageDone { generation, result }) => self.stage_done(generation, result),
                Err(RecvTimeoutError::Timeout) => self.tick(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.cancel_worker();
    }

    fn cancel_worker(&mut self) {
        if let Some(w) = self.worker.take() {
            w.cancel.store(true, Ordering::Relaxed);
            let _ = w.join.join();
        }
        // A cancelled worker's StageDone message is stale; bump the generation so it is ignored.
        self.generation += 1;
    }

    /// Where the status returns to once downloading stops without finishing.
    fn resting_status(&self) -> Status {
        match &self.pending {
            Some(p) => Status::UpdateAvailable { serial: p.serial(), files: p.files_to_place(), bytes: p.download_bytes() },
            None => Status::Starting,
        }
    }

    fn set_mode(&mut self, m: Mode) {
        if m == self.mode {
            return;
        }
        self.mode = m;
        match m {
            Mode::Paused => {
                if self.worker.is_some() {
                    self.cancel_worker();
                    self.set(self.resting_status());
                    return;
                }
                let s = self.status();
                self.set(s);
            }
            Mode::Manual => {
                let s = self.status();
                self.set(s);
            }
            Mode::Auto => {
                let s = self.status();
                self.set(s);
                if self.pending.is_some() {
                    self.begin_update();
                } else {
                    self.do_check();
                }
            }
        }
    }

    fn tick(&mut self) {
        // A downloaded update is applied as soon as the game is closed.
        if self.pending.is_some() && self.staged {
            if self.mode != Mode::Paused || matches!(self.status(), Status::WaitingForGame { .. }) {
                self.try_apply();
            }
            return;
        }
        if self.worker.is_some() || self.mode == Mode::Paused {
            return;
        }
        let due = self.last_check.is_none_or(|t| t.elapsed() >= self.cfg.poll);
        if due {
            self.do_check();
        } else if self.pending.is_some() && self.mode == Mode::Auto {
            // A failed download is retried.
            self.begin_update();
        }
    }

    fn do_check(&mut self) {
        if self.worker.is_some() {
            return; // a download is under way; the next check follows it
        }
        self.set(Status::Checking);
        self.last_check = Some(Instant::now());
        match check(&*self.source, &self.key, &self.opts) {
            Ok(Check::UpToDate(serial)) => {
                self.pending = None;
                self.staged = false;
                self.set(Status::Idle { serial });
            }
            Ok(Check::Pending(p)) => {
                if self.pending.as_ref().is_none_or(|old| old.serial() != p.serial()) {
                    self.staged = false;
                }
                self.pending = Some(Arc::new(p));
                self.set(self.resting_status());
                if self.mode == Mode::Auto {
                    self.begin_update();
                }
            }
            Err(e) => {
                let s = if self.pending.is_some() { self.resting_status() } else { Status::Error(e.to_string()) };
                if matches!(s, Status::Error(_)) {
                    eprintln!("spunkpipe[{}]: check failed: {e}", self.cfg.game);
                }
                self.set(s);
            }
        }
    }

    fn begin_update(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let Some(p) = self.pending.clone() else {
            self.do_check();
            return;
        };
        if self.staged {
            self.try_apply();
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (source, opts, progress, tx, generation) =
            (self.source.clone(), self.clone_opts(), self.progress.clone(), self.tx.clone(), self.generation);
        let c = cancel.clone();
        let join = std::thread::spawn(move || {
            let result = stage(&*source, &opts, &p, &c, &progress);
            let _ = tx.send(Msg::StageDone { generation, result });
        });
        self.worker = Some(Worker { cancel, join });
        let serial = self.pending.as_ref().map_or(0, |p| p.serial());
        self.set(Status::Downloading { serial });
    }

    fn clone_opts(&self) -> SyncOptions {
        SyncOptions {
            target: self.opts.target.clone(),
            state: self.opts.state.clone(),
            game: self.opts.game.clone(),
            serial: None,
            verify: false,
            dry_run: false,
            threads: self.opts.threads,
        }
    }

    fn stage_done(&mut self, generation: u64, result: Result<u64, Error>) {
        if generation != self.generation {
            return; // from a download we already cancelled
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join.join();
        }
        match result {
            Ok(_) => {
                self.staged = true;
                self.try_apply();
            }
            Err(Error::Cancelled) => self.set(self.resting_status()),
            Err(e) => {
                eprintln!("spunkpipe[{}]: download failed: {e}", self.cfg.game);
                self.set(Status::Error(e.to_string()));
            }
        }
    }

    fn try_apply(&mut self) {
        let Some(p) = self.pending.clone() else { return };
        let serial = p.serial();
        if self.probe.running() {
            if self.status() != (Status::WaitingForGame { serial }) {
                self.set(Status::WaitingForGame { serial });
            }
            return;
        }
        self.set(Status::Applying { serial });
        match apply(&self.opts, &p) {
            Ok(_) => {
                self.pending = None;
                self.staged = false;
                self.set(Status::Idle { serial });
            }
            Err(e) => {
                eprintln!("spunkpipe[{}]: applying failed: {e}", self.cfg.game);
                // The staged files are kept; the next tick retries.
                self.set(Status::Error(e.to_string()));
            }
        }
    }
}
