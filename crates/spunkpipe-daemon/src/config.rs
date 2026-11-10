use serde::{Deserialize, Serialize};
use spunkpipe_core::{fsutil::atomic_write, Error, Result};
use std::path::{Path, PathBuf};

/// How the client treats updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Check, download and apply on its own (apply waits until the game is closed).
    #[default]
    Auto,
    /// Check and announce updates, but only download when the user says so.
    Manual,
    /// No automatic network activity at all. "Check now" and "Update" still work on request.
    Paused,
}

fn default_poll() -> u64 {
    300
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameConfig {
    /// Profile id, for example `7dtd`.
    pub game: String,
    /// `spunkpipe://host#publickey`
    pub invite: String,
    /// The managed folder.
    pub target: PathBuf,
    #[serde(default)]
    pub mode: Mode,
    /// Seconds between checks.
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub games: Vec<GameConfig>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| Error::Other(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let s = toml::to_string_pretty(self).map_err(|e| Error::Other(e.to_string()))?;
        atomic_write(path, s.as_bytes())
    }

    /// Adds a game, replacing any existing entry for the same game.
    pub fn upsert(&mut self, g: GameConfig) {
        match self.games.iter_mut().find(|x| x.game == g.game) {
            Some(x) => *x = g,
            None => self.games.push(g),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/config.toml");
        assert_eq!(Config::load(&p).unwrap(), Config::default());
        let mut c = Config::default();
        c.upsert(GameConfig {
            game: "7dtd".into(),
            invite: "spunkpipe://h#k".into(),
            target: "/x/Mods".into(),
            mode: Mode::Manual,
            poll_secs: 60,
        });
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);

        let minimal: Config = toml::from_str("[[games]]\ngame='7dtd'\ninvite='i'\ntarget='/t'\n").unwrap();
        assert_eq!(minimal.games[0].mode, Mode::Auto);
        assert_eq!(minimal.games[0].poll_secs, 300);
    }
}
