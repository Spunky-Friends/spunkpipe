//! Shared tray logic: owns the daemon, turns its state into views, and carries out menu actions.

use crate::menu::{notification_for, Action, GameView};
use anyhow::{bail, Context, Result};
use spunkpipe_daemon::{paths, Command, Config, Daemon, Mode, Snapshot, Status};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Called when anything visible changed. The platform shell installs the real one.
pub type Refresh = Arc<Mutex<Box<dyn Fn() + Send>>>;

/// Why the tray is stopping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Quit,
    /// The configuration changed (a new game was joined); start again to pick it up.
    Restart,
}

pub struct App {
    daemon: Daemon,
    config: Config,
    quit_tx: Sender<Exit>,
    refresh: Refresh,
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
    pub fn start(refresh: Refresh) -> Result<(Arc<App>, Receiver<Exit>)> {
        let path = paths::config_file().context("cannot work out a config folder (set SPUNKPIPE_HOME)")?;
        let config = Config::load(&path)?;
        if config.games.is_empty() {
            bail!("nothing is set up yet. Run `spunkpipe join <invite>` first.");
        }
        let (quit_tx, quit_rx) = channel();
        let prev: Mutex<HashMap<String, Status>> = Mutex::new(HashMap::new());
        let refresh_cb = refresh.clone();
        let daemon = Daemon::start(
            &path,
            Arc::new(move |game: &str, snap: &Snapshot| {
                let view = GameView { game: game.into(), name: game_name(game), snapshot: snap.clone(), percent: 0 };
                let old = prev.lock().unwrap().insert(game.to_string(), snap.status.clone());
                if let Some((t, b)) = notification_for(old.as_ref(), &view) {
                    notify(&t, &b);
                }
                (refresh_cb.lock().unwrap())();
            }),
        )?;
        let app = Arc::new(App { daemon, config, quit_tx, refresh });
        app.watch_config(path);
        Ok((app, quit_rx))
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
            Action::Quit => self.request_exit(Exit::Quit),
        }
    }

    fn set_mode(&self, game: &str, mode: Mode) {
        if let Err(e) = self.daemon.set_mode(game, mode) {
            notify("SpunkPipe", &format!("Could not save the setting: {e}"));
        }
    }

    fn request_exit(&self, why: Exit) {
        let _ = self.quit_tx.send(why);
        // Wake the platform event loop so it notices.
        (self.refresh.lock().unwrap())();
    }

    /// Notices when another process (`spunkpipe join`, or a clicked invite link) changed which
    /// games are configured, and restarts so the new game is picked up. Mode changes made from
    /// this tray do not count.
    fn watch_config(self: &Arc<Self>, path: std::path::PathBuf) {
        fn identity(c: &Config) -> Vec<(String, String, std::path::PathBuf)> {
            c.games.iter().map(|g| (g.game.clone(), g.invite.clone(), g.target.clone())).collect()
        }
        let (app, baseline) = (self.clone(), identity(&self.config));
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if Config::load(&path).is_ok_and(|c| identity(&c) != baseline) {
                app.request_exit(Exit::Restart);
                return;
            }
        });
    }

    /// Stops the engines, and starts a fresh tray if asked to.
    pub fn finish(&self, why: Exit) {
        self.daemon.quit();
        if why == Exit::Restart {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).spawn();
            }
        }
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
