//! Where SpunkPipe keeps its files. `SPUNKPIPE_HOME` overrides everything (portable installs, tests).

use std::path::PathBuf;

fn var(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn base(config: bool) -> Option<PathBuf> {
    if let Some(h) = var("SPUNKPIPE_HOME") {
        return Some(h.join(if config { "config" } else { "data" }));
    }
    let b = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else if config {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
    };
    b.map(|b| b.join("spunkpipe"))
}

pub fn config_file() -> Option<PathBuf> {
    base(true).map(|b| b.join("config.toml"))
}

/// Lock file, staging area and backups for one game.
pub fn state_dir(game: &str) -> Option<PathBuf> {
    base(false).map(|b| b.join(game))
}
