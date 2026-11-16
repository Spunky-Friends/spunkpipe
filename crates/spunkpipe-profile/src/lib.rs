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
    /// Under Proton, where the game's user-data mods folder is inside the prefix's `steamuser` folder.
    #[serde(default)]
    pub proton_user_mods: Option<String>,
    /// Folders in the user's own data area that the game also loads mods from.
    #[serde(default)]
    pub user_data_candidates: Vec<Candidate>,
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

/// Where a mods folder was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderKind {
    /// The `Mods` folder inside the game's install folder.
    Install,
    /// The `Mods` folder in the user's data folder (`~/.local/share/7DaysToDie`, `%APPDATA%`, ...).
    UserData,
    /// The same, inside a Proton prefix (the Windows game run on Linux).
    Proton,
}

impl FolderKind {
    pub fn label(self) -> &'static str {
        match self {
            FolderKind::Install => "in the game's install folder",
            FolderKind::UserData => "in your user data folder",
            FolderKind::Proton => "in the Proton (Windows) prefix",
        }
    }
}

/// A folder the game loads mods from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModFolder {
    pub path: PathBuf,
    pub kind: FolderKind,
    /// How many mods are already in it (folders with a `ModInfo.xml`, not counting the game's own).
    pub mods: usize,
}

/// What to do about the candidate folders.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    /// One folder is clearly the right one.
    Use(ModFolder),
    /// Several could be (or none were found, when the list is empty): the user has to say.
    Ask(Vec<ModFolder>),
}

/// Counts folders that look like mods. The game's own `0_TFP_*` folders are not mods.
pub fn count_mods(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().to_string_lossy().starts_with("0_TFP"))
        .filter(|e| {
            std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .any(|f| f.file_name().to_string_lossy().eq_ignore_ascii_case("ModInfo.xml"))
        })
        .count()
}

/// Expands `~/` and `%NAME%` in a configured path. `None` if it needs something that is not set.
fn expand_path(raw: &str, home: Option<&Path>) -> Option<PathBuf> {
    let mut s = raw.to_string();
    while let Some(a) = s.find('%') {
        let Some(len) = s[a + 1..].find('%') else { break };
        let value = std::env::var(&s[a + 1..a + 1 + len]).ok()?;
        s = format!("{}{}{}", &s[..a], value, &s[a + len + 2..]);
    }
    match s.strip_prefix("~/") {
        Some(rest) => home.map(|h| h.join(rest)),
        None => Some(PathBuf::from(s)),
    }
}

/// Decides between candidate folders. A folder that already holds mods wins; when more than one
/// does, or none can be picked sensibly, the user is asked.
pub fn decide(folders: Vec<ModFolder>) -> Choice {
    let with_mods: Vec<ModFolder> = folders.iter().filter(|f| f.mods > 0).cloned().collect();
    match with_mods.len() {
        1 => return Choice::Use(with_mods[0].clone()),
        0 => {}
        _ => return Choice::Ask(with_mods),
    }
    // Nothing has mods yet (a fresh setup): the install folder's Mods is the normal place.
    let installs: Vec<ModFolder> = folders.iter().filter(|f| f.kind == FolderKind::Install).cloned().collect();
    match installs.len() {
        1 => Choice::Use(installs[0].clone()),
        0 if folders.len() == 1 => Choice::Use(folders[0].clone()),
        0 => Choice::Ask(folders),
        _ => Choice::Ask(installs),
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

impl Profile {
    /// Every folder on this machine the game loads mods from, with how many mods each holds.
    pub fn mod_folders(&self) -> Vec<ModFolder> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let mut out: Vec<ModFolder> = Vec::new();
        let mut push = |path: PathBuf, kind: FolderKind| {
            if !out.iter().any(|f| f.path == path) {
                let mods = count_mods(&path);
                out.push(ModFolder { path, kind, mods });
            }
        };
        for install in self.detect_installs() {
            push(install.join(&self.managed_dir), FolderKind::Install);
        }
        for c in self.user_data_candidates.iter().filter(|c| c.os == std::env::consts::OS) {
            if let Some(p) = expand_path(&c.path, home.as_deref()).filter(|p| p.is_dir()) {
                push(p, FolderKind::UserData);
            }
        }
        if let (Some(rel), Some(id)) = (&self.proton_user_mods, self.steam_app_id) {
            for lib in steam_libraries(home.as_deref()) {
                let p = lib
                    .join("steamapps/compatdata")
                    .join(id.to_string())
                    .join("pfx/drive_c/users/steamuser")
                    .join(rel);
                if p.is_dir() {
                    push(p, FolderKind::Proton);
                }
            }
        }
        out
    }

    /// The folder to manage, or the candidates to ask the user about.
    pub fn choose_mods_folder(&self) -> Choice {
        decide(self.mod_folders())
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

    fn folder(path: &str, kind: FolderKind, mods: usize) -> ModFolder {
        ModFolder { path: path.into(), kind, mods }
    }

    #[test]
    fn a_folder_that_already_has_mods_wins() {
        let install = folder("/g/Mods", FolderKind::Install, 0);
        let user = folder("/u/Mods", FolderKind::UserData, 85);
        assert_eq!(decide(vec![install, user.clone()]), Choice::Use(user));
    }

    #[test]
    fn several_folders_with_mods_means_asking() {
        let a = folder("/u/Mods", FolderKind::UserData, 85);
        let b = folder("/p/Mods", FolderKind::Proton, 85);
        let install = folder("/g/Mods", FolderKind::Install, 0);
        assert_eq!(decide(vec![install, a.clone(), b.clone()]), Choice::Ask(vec![a, b]));
    }

    #[test]
    fn a_fresh_setup_uses_the_install_folder_even_if_an_empty_user_folder_exists() {
        let install = folder("/g/Mods", FolderKind::Install, 0);
        let user = folder("/u/Mods", FolderKind::UserData, 0);
        assert_eq!(decide(vec![user, install.clone()]), Choice::Use(install));
    }

    #[test]
    fn two_fresh_installs_means_asking_and_nothing_found_means_asking_with_no_options() {
        let a = folder("/native/Mods", FolderKind::Install, 0);
        let b = folder("/proton/Mods", FolderKind::Install, 0);
        assert_eq!(decide(vec![a.clone(), b.clone()]), Choice::Ask(vec![a, b]));
        assert_eq!(decide(vec![]), Choice::Ask(vec![]));
    }

    #[test]
    fn counts_only_folders_that_are_mods() {
        let d = tempfile::tempdir().unwrap();
        for (dir, has_info) in [("0_TFP_Harmony", true), ("Good", true), ("AlsoGood", true), ("Loose", false)] {
            std::fs::create_dir_all(d.path().join(dir)).unwrap();
            if has_info {
                std::fs::write(d.path().join(dir).join("modinfo.xml"), "<x/>").unwrap();
            }
        }
        std::fs::write(d.path().join("readme.txt"), "x").unwrap();
        assert_eq!(count_mods(d.path()), 2);
        assert_eq!(count_mods(&d.path().join("missing")), 0);
    }

    #[test]
    fn expands_home_and_environment_names() {
        assert_eq!(expand_path("~/x/y", Some(Path::new("/h"))), Some(PathBuf::from("/h/x/y")));
        assert_eq!(expand_path("~/x", None), None);
        std::env::set_var("SPUNKPIPE_TEST_DIR", "/e");
        assert_eq!(expand_path("%SPUNKPIPE_TEST_DIR%/m", None), Some(PathBuf::from("/e/m")));
        assert_eq!(expand_path("%SPUNKPIPE_NOT_SET_ANYWHERE%/m", None), None);
        assert_eq!(expand_path("/plain", None), Some(PathBuf::from("/plain")));
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
