//! Joining from a clicked `spunkpipe://` link.

use crate::app::notify;
use anyhow::{Context, Result};
use spunkpipe_core::Invite;
use spunkpipe_daemon::{daemon, paths};

pub fn is_link(arg: &str) -> bool {
    arg.starts_with("spunkpipe://") || arg.starts_with("spunkpipe+http://")
}

/// Saves the invite and does the first sync, telling the user what is happening.
pub fn join_from_link(url: &str) -> Result<()> {
    let invite = Invite::parse(url).context("that is not a valid SpunkPipe invite")?;
    let game = invite.game.clone().unwrap_or_else(|| "7dtd".to_string());
    let config = paths::config_file().context("cannot work out a config folder")?;
    notify("SpunkPipe", "Setting up your mods. This can take a minute.");
    let (g, r) = daemon::join_and_sync(url, &game, None, &config)
        .context("could not set up. If the game is not in the usual place, run `spunkpipe join` with --target")?;
    notify(
        "SpunkPipe",
        &format!("Ready: {} files installed in {}. SpunkPipe will keep them up to date.", r.placed + r.unchanged, g.target.display()),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_invite_links() {
        assert!(is_link("spunkpipe://mods.example.com#abc"));
        assert!(is_link("spunkpipe+http://h:1#abc"));
        assert!(!is_link("https://example.com"));
        assert!(!is_link("--help"));
    }
}
