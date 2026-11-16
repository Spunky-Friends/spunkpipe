//! Joining from a clicked `spunkpipe://` link, only after the user agrees.

use crate::app::notify;
use crate::confirm::{ask, pick_folder, prompt_text, Answer, Picked};
use anyhow::{bail, Context, Result};
use spunkpipe_core::Invite;
use spunkpipe_daemon::{daemon, paths, Config, Detected, ModFolder};

pub fn is_link(arg: &str) -> bool {
    arg.starts_with("spunkpipe://") || arg.starts_with("spunkpipe+http://")
}

/// What to do about a link, decided without showing anything.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// This exact invite is already set up; nothing to ask.
    AlreadyJoined,
    /// Ask the user, with this text.
    Confirm(String),
}

pub fn plan(invite: &Invite, game: &str, existing: &Config) -> Result<Plan> {
    // Links come from web pages and chat; insist on HTTPS. Plain HTTP is for `spunkpipe join` typed by hand.
    if !invite.secure {
        bail!("links must use https. For a plain http server, run `spunkpipe join` in a terminal.");
    }
    let current = existing.games.iter().find(|g| g.game == game);
    if current.is_some_and(|g| Invite::parse(&g.invite).is_ok_and(|i| i == *invite)) {
        return Ok(Plan::AlreadyJoined);
    }
    let replacing = current.and_then(|g| Invite::parse(&g.invite).ok()).map(|i| i.host_path);
    prompt_text(&invite.host_path, game, &invite.public_key.fingerprint(), replacing.as_deref())
        .map(Plan::Confirm)
        .context("that link points at a server name SpunkPipe will not show or trust automatically")
}

/// Asks the user where their mods folder is, in a folder chooser that starts at our best guess.
/// `None` means they cancelled.
fn choose_folder(found: &[ModFolder], url: &str) -> Result<Option<std::path::PathBuf>> {
    let title = if found.is_empty() {
        "SpunkPipe could not find the game. Choose the game's Mods folder"
    } else {
        "SpunkPipe found more than one Mods folder. Choose the one to keep in step with the server"
    };
    let start = found.first().map(|f| f.path.as_path());
    match pick_folder(title, start) {
        Picked::Folder(p) => {
            if !p.is_dir() {
                bail!("{} is not a folder", p.display());
            }
            daemon::validate_target(&p)?;
            Ok(Some(p))
        }
        Picked::Cancelled => Ok(None),
        Picked::Unavailable => bail!(
            "SpunkPipe cannot show a folder chooser here. In a terminal, run: spunkpipe join '{url}' --target <your Mods folder>"
        ),
    }
}

/// Saves the invite and does the first sync, telling the user what is happening.
pub fn join_from_link(url: &str) -> Result<()> {
    let invite = Invite::parse(url).context("that is not a valid SpunkPipe invite")?;
    let game = invite.game.clone().unwrap_or_else(|| "7dtd".to_string());
    let config_path = paths::config_file().context("cannot work out a config folder")?;
    let existing = Config::load(&config_path)?;

    if let Plan::Confirm(text) = plan(&invite, &game, &existing)? {
        match ask(&text) {
            Answer::Yes => {}
            Answer::No => return Ok(()),
            Answer::Unavailable => bail!(
                "SpunkPipe cannot show a confirmation window here, so it will not join from a link. In a terminal, run: spunkpipe join '{url}'"
            ),
        }
    }
    let target = match daemon::detect_target(&game)? {
        Detected::Found(p) => p,
        Detected::Ask(found) => match choose_folder(&found, url)? {
            Some(p) => p,
            None => {
                notify("SpunkPipe", "Setup cancelled: no mods folder was chosen. Click the link again to retry.");
                return Ok(());
            }
        },
    };
    notify("SpunkPipe", "Setting up your mods. This can take a minute.");
    let (g, r) = daemon::join_and_sync(url, &game, Some(target), &config_path)
        .context("could not set up. In a terminal, run `spunkpipe join` with --target <your Mods folder>")?;
    notify(
        "SpunkPipe",
        &format!("Ready: {} files installed in {}. SpunkPipe will keep them up to date.", r.placed + r.unchanged, g.target.display()),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spunkpipe_core::SecretKey;
    use spunkpipe_daemon::{GameConfig, Mode};

    fn invite(host: &str, key: &spunkpipe_core::PublicKey) -> Invite {
        Invite::new(true, host, key.clone()).with_game("7dtd")
    }

    fn config_with(i: &Invite) -> Config {
        Config {
            games: vec![GameConfig { game: "7dtd".into(), invite: i.to_string(), target: "/m".into(), mode: Mode::Auto, poll_secs: 300 }],
        }
    }

    #[test]
    fn recognises_invite_links() {
        assert!(is_link("spunkpipe://mods.example.com#abc"));
        assert!(is_link("spunkpipe+http://h:1#abc"));
        assert!(!is_link("https://example.com"));
        assert!(!is_link("--help"));
    }

    #[test]
    fn a_new_server_must_be_confirmed() {
        let k = SecretKey::generate().public();
        let Plan::Confirm(t) = plan(&invite("mods.example.com", &k), "7dtd", &Config::default()).unwrap() else { panic!() };
        assert!(t.contains("mods.example.com") && t.contains(&k.fingerprint()) && t.contains("7dtd"));
    }

    #[test]
    fn the_same_invite_again_needs_no_question() {
        let k = SecretKey::generate().public();
        let i = invite("mods.example.com", &k);
        assert_eq!(plan(&i, "7dtd", &config_with(&i)).unwrap(), Plan::AlreadyJoined);
    }

    #[test]
    fn a_different_key_or_host_is_a_replacement_and_says_so() {
        let (k1, k2) = (SecretKey::generate().public(), SecretKey::generate().public());
        let cfg = config_with(&invite("mods.example.com", &k1));
        // Same host, attacker's key.
        let Plan::Confirm(t) = plan(&invite("mods.example.com", &k2), "7dtd", &cfg).unwrap() else { panic!() };
        assert!(t.starts_with("WARNING") && t.contains(&k2.fingerprint()));
        // Different host.
        let Plan::Confirm(t) = plan(&invite("evil.example", &k2), "7dtd", &cfg).unwrap() else { panic!() };
        assert!(t.starts_with("WARNING") && t.contains("mods.example.com"));
    }

    #[test]
    fn plain_http_links_are_refused() {
        let k = SecretKey::generate().public();
        let http = Invite::new(false, "mods.example.com", k);
        assert!(plan(&http, "7dtd", &Config::default()).is_err());
    }

    #[test]
    fn disguised_hosts_are_refused_outright() {
        let k = SecretKey::generate().public();
        assert!(plan(&invite("mods.exämple.com", &k), "7dtd", &Config::default()).is_err());
    }
}
