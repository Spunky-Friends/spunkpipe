use crate::config::{Config, GameConfig, Mode};
use crate::engine::{Command, Engine, EngineConfig, Handle, Snapshot};
use crate::game::{GameProbe, NeverRunning, ProcessProbe};
use crate::paths;
use spunkpipe_client::HttpSource;
use spunkpipe_core::{Error, Invite, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Called whenever any game's status changes, with the game id.
pub type OnChange = Arc<dyn Fn(&str, &Snapshot) + Send + Sync>;

/// Runs one engine per configured game and keeps the config file in step with mode changes.
pub struct Daemon {
    config_path: PathBuf,
    config: Mutex<Config>,
    handles: Vec<(String, Handle)>,
}

impl Daemon {
    pub fn start(config_path: &Path, on_change: OnChange) -> Result<Daemon> {
        let config = Config::load(config_path)?;
        let mut handles = Vec::new();
        for g in &config.games {
            let invite = Invite::parse(&g.invite)?;
            let profile = spunkpipe_profile::builtin(&g.game);
            let probe: Arc<dyn GameProbe> = match &profile {
                Some(p) => Arc::new(ProcessProbe::new(&p.process_names)),
                None => Arc::new(NeverRunning),
            };
            let state = paths::state_dir(&g.game)
                .ok_or_else(|| Error::Other("cannot work out a data folder (set SPUNKPIPE_HOME)".into()))?;
            let game = g.game.clone();
            let cb = on_change.clone();
            let handle = Engine::spawn(
                EngineConfig {
                    game: g.game.clone(),
                    target: g.target.clone(),
                    state,
                    mode: g.mode,
                    poll: Duration::from_secs(g.poll_secs.max(30)),
                    tick: Duration::from_secs(5),
                },
                Arc::new(HttpSource::new(&invite.base_url())),
                invite.public_key.clone(),
                probe,
                Arc::new(move |s| cb(&game, s)),
            );
            handles.push((g.game.clone(), handle));
        }
        Ok(Daemon { config_path: config_path.to_path_buf(), config: Mutex::new(config), handles })
    }

    pub fn games(&self) -> Vec<String> {
        self.handles.iter().map(|(g, _)| g.clone()).collect()
    }

    pub fn handle(&self, game: &str) -> Option<&Handle> {
        self.handles.iter().find(|(g, _)| g == game).map(|(_, h)| h)
    }

    pub fn snapshot(&self, game: &str) -> Option<Snapshot> {
        self.handle(game).map(|h| h.snapshot())
    }

    pub fn command(&self, game: &str, c: Command) {
        if let Some(h) = self.handle(game) {
            h.send(c);
        }
    }

    /// Changes a game's mode and remembers it across restarts.
    pub fn set_mode(&self, game: &str, mode: Mode) -> Result<()> {
        self.command(game, Command::SetMode(mode));
        let mut cfg = self.config.lock().unwrap();
        if let Some(g) = cfg.games.iter_mut().find(|g| g.game == game) {
            g.mode = mode;
        }
        cfg.save(&self.config_path)
    }

    pub fn quit(&self) {
        for (_, h) in &self.handles {
            h.quit();
        }
    }
}

/// Adds (or replaces) a game in the config from an invite. Does not download anything.
pub fn join(invite: &str, game: &str, target: Option<PathBuf>, config_path: &Path) -> Result<GameConfig> {
    Invite::parse(invite)?;
    let target = match target {
        Some(t) => t,
        None => {
            let p = spunkpipe_profile::builtin(game)
                .ok_or_else(|| Error::Other(format!("unknown game {game:?}; pass the mods folder explicitly")))?;
            let install = p
                .detect_installs()
                .into_iter()
                .next()
                .ok_or_else(|| Error::Other(format!("could not find a {} install; pass the mods folder explicitly", p.name)))?;
            install.join(&p.managed_dir)
        }
    };
    let g = GameConfig { game: game.to_string(), invite: invite.trim().to_string(), target, mode: Mode::Auto, poll_secs: 300 };
    let mut cfg = Config::load(config_path)?;
    cfg.upsert(g.clone());
    cfg.save(config_path)?;
    Ok(g)
}
