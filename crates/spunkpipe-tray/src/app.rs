//! Shared tray logic: owns the daemon, turns its state into views, and carries out menu actions.

use crate::menu::{notification_for, Action, GameView};
use anyhow::{bail, Context, Result};
use spunkpipe_daemon::{paths, Command, Config, Daemon, Mode, Snapshot, Status};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Called when anything visible changed. The platform shell installs the real one.
pub type Refresh = Arc<Mutex<Box<dyn Fn() + Send>>>;

pub struct App {
    daemon: Daemon,
    config: Config,
    quit_tx: Sender<()>,
}

pub fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new().summary(title).body(body).appname("SpunkPipe").show();
}

fn game_name(game: &str) -> String {
    spunkpipe_profile::builtin(game).map_or_else(|| game.to_string(), |p| p.name)
}

impl App {
    /// Starts the daemon for every configured game. Returns the app and a receiver that fires
    /// when the user picks Quit.
    pub fn start(refresh: Refresh) -> Result<(Arc<App>, Receiver<()>)> {
        let path = paths::config_file().context("cannot work out a config folder (set SPUNKPIPE_HOME)")?;
        let config = Config::load(&path)?;
        if config.games.is_empty() {
            bail!("nothing is set up yet. Run `spunkpipe join <invite>` first.");
        }
        let (quit_tx, quit_rx) = channel();
        let prev: Mutex<HashMap<String, Status>> = Mutex::new(HashMap::new());
        let daemon = Daemon::start(
            &path,
            Arc::new(move |game: &str, snap: &Snapshot| {
                let view = GameView { game: game.into(), name: game_name(game), snapshot: snap.clone(), percent: 0 };
                let old = prev.lock().unwrap().insert(game.to_string(), snap.status.clone());
                if let Some((t, b)) = notification_for(old.as_ref(), &view) {
                    notify(&t, &b);
                }
                (refresh.lock().unwrap())();
            }),
        )?;
        Ok((Arc::new(App { daemon, config, quit_tx }), quit_rx))
    }

    pub fn views(&self) -> Vec<GameView> {
        self.config
            .games
            .iter()
            .filter_map(|g| {
                let h = self.daemon.handle(&g.game)?;
                Some(GameView {
                    game: g.game.clone(),
                    name: game_name(&g.game),
                    snapshot: h.snapshot(),
                    percent: h.progress().percent(),
                })
            })
            .collect()
    }

    pub fn downloading(&self) -> bool {
        self.views().iter().any(|v| matches!(v.snapshot.status, Status::Downloading { .. }))
    }

    pub fn handle(&self, a: Action) {
        match a {
            Action::CheckNow(g) => self.daemon.command(&g, Command::CheckNow),
            Action::Update(g) => self.daemon.command(&g, Command::Update),
            Action::SetMode(g, m) => self.set_mode(&g, m),
            Action::OpenFolder(g) => {
                if let Some(c) = self.config.games.iter().find(|c| c.game == g) {
                    open_folder(&c.target);
                }
            }
            Action::Quit => {
                let _ = self.quit_tx.send(());
            }
        }
    }

    fn set_mode(&self, game: &str, mode: Mode) {
        if let Err(e) = self.daemon.set_mode(game, mode) {
            notify("SpunkPipe", &format!("Could not save the setting: {e}"));
        }
    }

    pub fn shutdown(&self) {
        self.daemon.quit();
    }
}

fn open_folder(p: &std::path::Path) {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(p).spawn();
}
