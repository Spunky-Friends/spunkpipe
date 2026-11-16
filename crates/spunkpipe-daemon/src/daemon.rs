use crate::config::{Config, GameConfig, Mode};
use crate::engine::{Command, Engine, EngineConfig, Handle, Snapshot};
use crate::game::{GameProbe, NeverRunning, ProcessProbe};
use crate::paths;
use spunkpipe_client::HttpSource;
use spunkpipe_core::{Error, Invite, Result};
use spunkpipe_profile::{Choice, ModFolder};
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

/// Works out the managed folder (for 7 Days to Die: `Mods`) from the folder the game is
/// installed in. For users whose game is somewhere SpunkPipe does not look.
pub fn target_for_game_dir(game: &str, game_dir: &Path) -> Result<PathBuf> {
    let p = spunkpipe_profile::builtin(game).ok_or_else(|| Error::Other(format!("unknown game {game:?}")))?;
    if !game_dir.is_dir() {
        return Err(Error::Other(format!("{} is not a folder", game_dir.display())));
    }
    Ok(game_dir.join(&p.managed_dir))
}

/// The outcome of looking for a game's mods folder on this machine.
#[derive(Debug, PartialEq, Eq)]
pub enum Detected {
    /// One folder is clearly the right one.
    Found(PathBuf),
    /// The user has to choose: several folders could be right, or none were found (empty list).
    Ask(Vec<ModFolder>),
}

/// Looks for the folder to manage: one that already holds mods wins, then the install folder.
pub fn detect_target(game: &str) -> Result<Detected> {
    let p = spunkpipe_profile::builtin(game)
        .ok_or_else(|| Error::Other(format!("unknown game {game:?}; pass --target <the mods folder>")))?;
    Ok(match p.choose_mods_folder() {
        Choice::Use(f) => Detected::Found(f.path),
        Choice::Ask(list) => Detected::Ask(list),
    })
}

/// Refuses a managed folder that could only be a mistake: a relative path, a drive or filesystem
/// root, or the home folder itself. SpunkPipe writes whole mod folders into it.
pub fn validate_target(target: &Path) -> Result<()> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if !target.is_absolute() || target.parent().is_none() || home.as_deref() == Some(target) {
        return Err(Error::Other(format!(
            "{} is not a mods folder. Choose the game's Mods folder, not a drive or your home folder",
            target.display()
        )));
    }
    Ok(())
}

/// Adds (or replaces) a game in the config from an invite. Does not download anything.
pub fn join(invite: &str, game: &str, target: Option<PathBuf>, config_path: &Path) -> Result<GameConfig> {
    Invite::parse(invite)?;
    let target = match target {
        Some(t) => t,
        None => match detect_target(game)? {
            Detected::Found(p) => p,
            Detected::Ask(list) if list.is_empty() => {
                return Err(Error::Other(
                    "could not find the game. Say where it is with --game-dir <the game's folder> (or --target <its Mods folder>)".into(),
                ))
            }
            Detected::Ask(list) => {
                let places: Vec<String> = list.iter().map(|f| f.path.display().to_string()).collect();
                return Err(Error::Other(format!(
                    "found more than one place for mods ({}). Choose one with --target <folder>",
                    places.join(", ")
                )));
            }
        },
    };
    validate_target(&target)?;
    let g = GameConfig { game: game.to_string(), invite: invite.trim().to_string(), target, mode: Mode::Auto, poll_secs: 300 };
    let mut cfg = Config::load(config_path)?;
    cfg.upsert(g.clone());
    cfg.save(config_path)?;
    Ok(g)
}

/// Joins and does the first sync, so the mods are in place before anything else happens.
pub fn join_and_sync(
    invite: &str,
    game: &str,
    target: Option<PathBuf>,
    config_path: &Path,
) -> Result<(GameConfig, spunkpipe_client::Report)> {
    let g = join(invite, game, target, config_path)?;
    let parsed = Invite::parse(&g.invite)?;
    let state = paths::state_dir(game).ok_or_else(|| Error::Other("cannot work out a data folder (set SPUNKPIPE_HOME)".into()))?;
    let report = spunkpipe_client::sync(
        &HttpSource::new(&parsed.base_url()),
        &parsed.public_key,
        &spunkpipe_client::SyncOptions {
            target: g.target.clone(),
            state,
            game: game.to_string(),
            serial: None,
            verify: false,
            dry_run: false,
            threads: spunkpipe_core::par::default_threads(),
        },
    )?;
    Ok((g, report))
}

#[cfg(test)]
mod game_dir_tests {
    use super::*;

    #[test]
    fn a_root_a_relative_path_or_the_home_folder_is_not_a_mods_folder() {
        let d = tempfile::tempdir().unwrap();
        assert!(validate_target(&d.path().join("Mods")).is_ok());
        assert!(validate_target(Path::new("/")).is_err());
        assert!(validate_target(Path::new("Mods")).is_err());
        if let Some(h) = std::env::var_os("HOME") {
            assert!(validate_target(Path::new(&h)).is_err());
        }
    }

    #[test]
    fn mods_folder_comes_from_the_game_folder() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(target_for_game_dir("7dtd", d.path()).unwrap(), d.path().join("Mods"));
        assert!(target_for_game_dir("7dtd", &d.path().join("missing")).is_err());
        assert!(target_for_game_dir("nope", d.path()).is_err());
    }
}
