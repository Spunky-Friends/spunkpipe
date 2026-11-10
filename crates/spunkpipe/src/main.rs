use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use spunkpipe_client::{sync, HttpSource, SyncOptions};
use spunkpipe_core::{par::default_threads, Invite, PublicKey, SecretKey};
use spunkpipe_daemon::{autostart, daemon, paths, Config, Daemon, Mode, Status};
use spunkpipe_server::{publish, PublishOptions};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "spunkpipe", version, about = "Keep game mods in sync with a game server")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new publisher signing key.
    Keygen {
        /// Where to write the secret key (must not exist).
        keyfile: PathBuf,
    },
    /// Print the invite string to give to friends.
    Invite {
        #[arg(long)]
        key: PathBuf,
        /// Host (and optional port/path) the files are served from, e.g. mods.example.com
        #[arg(long)]
        host: String,
        /// Serve over plain HTTP instead of HTTPS.
        #[arg(long)]
        http: bool,
    },
    /// Publish a mods folder: write a signed manifest and blobs to a web root.
    Publish {
        /// The folder to publish, e.g. the server's Mods folder.
        #[arg(long)]
        source: PathBuf,
        /// The web root to write into.
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        key: PathBuf,
        /// Publisher-private state (hash cache, last serial). Must be outside the web root.
        /// Defaults to `<key>.state` next to the key file.
        #[arg(long)]
        state: Option<PathBuf>,
        #[arg(long, default_value = "7dtd")]
        game: String,
        /// Old manifests to keep for rollback.
        #[arg(long, default_value_t = 10)]
        keep: usize,
        /// Publish even if nothing changed.
        #[arg(long)]
        force: bool,
    },
    /// Make the local mods folder match the server.
    Sync {
        #[arg(long)]
        invite: String,
        #[arg(long, default_value = "7dtd")]
        game: String,
        /// The managed folder. Defaults to the game's Mods folder if the install is found.
        #[arg(long)]
        target: Option<PathBuf>,
        /// Where SpunkPipe keeps its lock file and backups.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Sync to this serial, even if older (rollback).
        #[arg(long)]
        serial: Option<u64>,
        /// Re-check every file even if the server has not changed.
        #[arg(long)]
        verify: bool,
        /// Show what would change without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Set this machine up from an invite: save it, do the first sync and start at login.
    Join {
        /// The invite string you were sent.
        invite: String,
        #[arg(long, default_value = "7dtd")]
        game: String,
        /// The managed folder. Defaults to the game's Mods folder if the install is found.
        #[arg(long)]
        target: Option<PathBuf>,
        /// Do not register the tray app to start at login.
        #[arg(long)]
        no_autostart: bool,
    },
    /// Run the background updater without a tray icon (servers, or desktops with no tray).
    Run,
    /// Show the configured games and what is installed.
    Status,
    /// Change how updates are handled for a game. Restart a running updater to apply it.
    Mode {
        #[arg(value_parser = ["auto", "manual", "paused"])]
        mode: String,
        #[arg(long, default_value = "7dtd")]
        game: String,
    },
    /// Start the tray app at login (or stop doing so).
    Autostart {
        #[arg(value_parser = ["enable", "disable", "status"])]
        action: String,
    },
    /// List built-in game profiles.
    Profiles,
}

fn config_path() -> Result<PathBuf> {
    paths::config_file().context("cannot work out a config folder; set SPUNKPIPE_HOME")
}

fn tray_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "spunkpipe-tray.exe" } else { "spunkpipe-tray" };
    let p = exe.parent()?.join(name);
    p.is_file().then_some(p)
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Keygen { keyfile } => {
            let key = SecretKey::generate();
            key.save_new(&keyfile).with_context(|| format!("writing {}", keyfile.display()))?;
            println!("Wrote secret key to {} (keep it private and back it up).", keyfile.display());
            println!("Public key: {}", key.public().to_base64());
        }
        Cmd::Invite { key, host, http } => {
            let pk = SecretKey::load(&key)?.public();
            println!("{}", Invite::new(!http, &host, pk));
        }
        Cmd::Publish { source, out, key, state, game, keep, force } => {
            let state = state.unwrap_or_else(|| {
                let mut s = key.clone().into_os_string();
                s.push(".state");
                PathBuf::from(s)
            });
            let key = SecretKey::load(&key).context("loading signing key")?;
            let r = publish(&PublishOptions { source, out, state, game, key, force, keep, threads: default_threads() })?;
            if r.published {
                println!("Published serial {} ({} files, {} new blobs)", r.serial, r.files, r.new_blobs);
            } else {
                println!("Nothing changed; still at serial {} ({} files)", r.serial, r.files);
            }
        }
        Cmd::Sync { invite, game, target, state, serial, verify, dry_run } => {
            let invite = Invite::parse(&invite)?;
            let profile = spunkpipe_profile::builtin(&game);
            let target = match target {
                Some(t) => t,
                None => {
                    let Some(p) = &profile else { bail!("unknown game {game:?}; pass --target") };
                    let Some(install) = p.detect_installs().into_iter().next() else {
                        bail!("could not find a {} install; pass --target", p.name)
                    };
                    install.join(&p.managed_dir)
                }
            };
            let state = match state {
                Some(s) => s,
                None => paths::state_dir(&game).context("cannot work out a data folder; pass --state")?,
            };
            let source = HttpSource::new(&invite.base_url());
            let r = sync(
                &source,
                &PublicKey::from_base64(&invite.public_key.to_base64())?,
                &SyncOptions { target, state, game, serial, verify, dry_run, threads: default_threads() },
            )?;
            if r.up_to_date {
                println!("Up to date (serial {}).", r.serial);
            } else {
                let verb = if dry_run { "Would change" } else { "Synced" };
                println!(
                    "{verb} to serial {}: {} files placed, {} removed, {} unchanged, {} blobs ({} bytes) fetched, {} backed up.",
                    r.serial, r.placed, r.removed, r.unchanged, r.fetched_blobs, r.fetched_bytes, r.backed_up
                );
            }
        }
        Cmd::Join { invite, game, target, no_autostart } => {
            let g = daemon::join(&invite, &game, target, &config_path()?)?;
            println!("Saved. Mods folder: {}", g.target.display());
            let parsed = Invite::parse(&g.invite)?;
            let state = paths::state_dir(&game).context("cannot work out a data folder")?;
            let r = sync(
                &HttpSource::new(&parsed.base_url()),
                &parsed.public_key,
                &SyncOptions { target: g.target.clone(), state, game: game.clone(), serial: None, verify: false, dry_run: false, threads: default_threads() },
            )?;
            println!(
                "Synced to serial {}: {} files placed, {} unchanged, {} backed up.",
                r.serial, r.placed, r.unchanged, r.backed_up
            );
            if !no_autostart {
                match tray_binary() {
                    Some(t) => {
                        autostart::enable(&t)?;
                        println!("The tray app will start at login. Start it now with: {}", t.display());
                    }
                    None => println!("Tray app not found next to this program; run `spunkpipe run` for background updates."),
                }
            }
        }
        Cmd::Run => {
            let d = Daemon::start(
                &config_path()?,
                std::sync::Arc::new(|game, s| {
                    let line = match &s.status {
                        Status::Starting => "starting".to_string(),
                        Status::Checking => "checking".to_string(),
                        Status::Idle { serial } => format!("up to date (serial {serial})"),
                        Status::UpdateAvailable { serial, files, bytes } => {
                            format!("update available: serial {serial}, {files} files, {bytes} bytes")
                        }
                        Status::Downloading { serial } => format!("downloading serial {serial}"),
                        Status::WaitingForGame { serial } => format!("serial {serial} ready; waiting for the game to close"),
                        Status::Applying { serial } => format!("applying serial {serial}"),
                        Status::Error(e) => format!("error: {e}"),
                    };
                    println!("[{game}] ({:?}) {line}", s.mode);
                }),
            )?;
            if d.games().is_empty() {
                bail!("nothing configured; run `spunkpipe join <invite>` first");
            }
            loop {
                std::thread::park();
            }
        }
        Cmd::Status => {
            let cfg = Config::load(&config_path()?)?;
            if cfg.games.is_empty() {
                println!("Nothing configured. Run `spunkpipe join <invite>`.");
            }
            for g in &cfg.games {
                let lock = paths::state_dir(&g.game)
                    .map(|s| spunkpipe_client::Lock::load(&s))
                    .transpose()?
                    .unwrap_or_default();
                println!("{}: mode {:?}, serial {}, {} files, folder {}", g.game, g.mode, lock.serial, lock.files.len(), g.target.display());
            }
            println!("Start at login: {}", if autostart::is_enabled() { "yes" } else { "no" });
        }
        Cmd::Mode { mode, game } => {
            let m = match mode.as_str() {
                "auto" => Mode::Auto,
                "manual" => Mode::Manual,
                _ => Mode::Paused,
            };
            let path = config_path()?;
            let mut cfg = Config::load(&path)?;
            let Some(g) = cfg.games.iter_mut().find(|g| g.game == game) else { bail!("{game} is not configured") };
            g.mode = m;
            cfg.save(&path)?;
            println!("{game} is now {mode}. The tray menu changes it live; a running `spunkpipe run` needs a restart.");
        }
        Cmd::Autostart { action } => match action.as_str() {
            "enable" => {
                let t = tray_binary().context("spunkpipe-tray not found next to this program")?;
                autostart::enable(&t)?;
                println!("Enabled.");
            }
            "disable" => {
                autostart::disable()?;
                println!("Disabled.");
            }
            _ => println!("{}", if autostart::is_enabled() { "enabled" } else { "disabled" }),
        },
        Cmd::Profiles => {
            for id in spunkpipe_profile::builtin_ids() {
                let p = spunkpipe_profile::builtin(id).unwrap();
                println!("{}\t{}", p.id, p.name);
            }
        }
    }
    Ok(())
}
