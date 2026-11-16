//! Declarative game profiles: where a game's mods live and how to recognise it running.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub steam_app_id: Option<u32>,
    /// The managed folder, relative to the game's install directory.
    pub managed_dir: String,
    /// The game's folder name inside a Steam library's `steamapps/common`. Lets installs on any
    /// of the user's Steam library drives be found.
    #[serde(default)]
    pub steam_dir: Option<String>,
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

    /// Existing install directories on this machine, most likely first: the usual places, then
    /// every Steam library the user has set up (a game on a second drive is common).
    pub fn detect_installs(&self) -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let mut found: Vec<PathBuf> = Vec::new();
        let mut add = |p: PathBuf| {
            if p.is_dir() && !found.contains(&p) {
                found.push(p);
            }
        };
        for c in self.install_candidates.iter().filter(|c| c.os == std::env::consts::OS) {
            add(match (c.path.strip_prefix("~/"), &home) {
                (Some(rest), Some(h)) => h.join(rest),
                _ => PathBuf::from(&c.path),
            });
        }
        if let Some(dir) = &self.steam_dir {
            for lib in steam_libraries(home.as_deref()) {
                add(lib.join("steamapps").join("common").join(dir));
            }
        }
        found
    }
}

/// Where Steam itself usually lives on this OS.
fn steam_roots(home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(h) = home {
        match std::env::consts::OS {
            "linux" => {
                roots.push(h.join(".local/share/Steam"));
                roots.push(h.join(".steam/steam"));
                roots.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
            }
            "macos" => roots.push(h.join("Library/Application Support/Steam")),
            _ => {}
        }
    }
    if std::env::consts::OS == "windows" {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(var) {
                roots.push(PathBuf::from(p).join("Steam"));
            }
        }
        roots.push(PathBuf::from("C:/Program Files (x86)/Steam"));
    }
    roots
}

/// Every Steam library folder the user has configured, including the main one.
pub fn steam_libraries(home: Option<&Path>) -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in steam_roots(home) {
        if root.is_dir() && !libs.contains(&root) {
            libs.push(root.clone());
        }
        let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) else {
            continue;
        };
        for p in parse_library_paths(&text) {
            if !libs.contains(&p) {
                libs.push(p);
            }
        }
    }
    libs
}

/// Pulls the `"path"` entries out of Steam's `libraryfolders.vdf`.
pub fn parse_library_paths(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('"').skip(1).step_by(2);
        if parts.next().is_some_and(|k| k.eq_ignore_ascii_case("path")) {
            if let Some(v) = parts.next() {
                // Backslashes are doubled in the file (`C:\\Games`).
                out.push(PathBuf::from(v.replace("\\\\", "\\")));
            }
        }
    }
    out
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
    fn reads_library_paths_from_steams_file() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/u/.local/share/Steam\"\n\t\t\"label\"\t\t\"\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"1250\"\t\t\"6494991595\"\n\t\t}\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            parse_library_paths(vdf),
            [PathBuf::from("/home/u/.local/share/Steam"), PathBuf::from("D:\\SteamLibrary")]
        );
    }

    #[test]
    fn finds_an_install_in_a_second_steam_library() {
        let d = tempfile::tempdir().unwrap();
        let (home, lib2) = (d.path().join("home"), d.path().join("second"));
        let steam = home.join(".local/share/Steam");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(lib2.join("steamapps/common/7 Days To Die")).unwrap();
        std::fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", lib2.display()),
        )
        .unwrap();
        let libs = steam_libraries(Some(&home));
        if std::env::consts::OS == "linux" {
            assert!(libs.contains(&lib2));
            assert!(libs.contains(&steam));
        }
    }

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
