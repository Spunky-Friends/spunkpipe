//! Declarative game profiles: where a game's mods live and how to recognise it running.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub steam_app_id: Option<u32>,
    /// The managed folder, relative to the game's install directory.
    pub managed_dir: String,
    /// Process names that mean the game is running (checked case-insensitively).
    pub process_names: Vec<String>,
    /// Places the game is commonly installed. `~` is the home directory.
    pub install_candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// `linux`, `windows` or `macos`.
    pub os: String,
    pub path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid profile: {0}")]
    Parse(#[from] toml::de::Error),
}

impl Profile {
    pub fn from_toml(s: &str) -> Result<Self, Error> {
        Ok(toml::from_str(s)?)
    }

    /// Existing install directories on this machine, most likely first.
    pub fn detect_installs(&self) -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        self.install_candidates
            .iter()
            .filter(|c| c.os == std::env::consts::OS)
            .map(|c| match (c.path.strip_prefix("~/"), &home) {
                (Some(rest), Some(h)) => h.join(rest),
                _ => PathBuf::from(&c.path),
            })
            .filter(|p| p.is_dir())
            .collect()
    }
}

const SDTD: &str = include_str!("../profiles/7dtd.toml");

/// Profiles that ship with SpunkPipe.
pub fn builtin(id: &str) -> Option<Profile> {
    match id {
        "7dtd" => Some(Profile::from_toml(SDTD).expect("built-in profile is valid")),
        _ => None,
    }
}

pub fn builtin_ids() -> &'static [&'static str] {
    &["7dtd"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_parse() {
        for id in builtin_ids() {
            let p = builtin(id).unwrap();
            assert_eq!(&p.id, id);
            assert!(!p.process_names.is_empty());
        }
        assert!(builtin("nope").is_none());
        assert_eq!(builtin("7dtd").unwrap().steam_app_id, Some(251570));
    }
}
