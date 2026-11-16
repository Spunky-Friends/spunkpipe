use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use spunkpipe_client::selfupdate::{self, Outcome, UpdateOptions};
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
        /// Game profile the invite is for (clients assume 7dtd when absent).
        #[arg(long)]
        game: Option<String>,
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
        /// A file listing things to leave out (one `.gitignore`-style pattern per line, relative
        /// to the source folder). Defaults to `<key>.exclude` next to the key file, if it exists.
        #[arg(long)]
        exclude: Option<PathBuf>,
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
        /// The folder the game is installed in (its Mods folder is worked out from this).
        #[arg(long, conflicts_with = "target")]
        game_dir: Option<PathBuf>,
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
        /// Defaults to the game named in the invite, or 7dtd.
        #[arg(long)]
        game: Option<String>,
        /// The managed folder. Defaults to the game's Mods folder if the install is found.
        #[arg(long)]
        target: Option<PathBuf>,
        /// The folder the game is installed in (its Mods folder is worked out from this).
        /// Use this if the game is not in a place SpunkPipe finds on its own.
        #[arg(long, conflicts_with = "target")]
        game_dir: Option<PathBuf>,
        /// Do not register the tray app to start at login or to open spunkpipe:// links.
        #[arg(long)]
        no_autostart: bool,
    },
    /// Point an already-joined game at a different install folder.
    Folder {
        /// The folder the game is installed in, e.g. /data2/SteamLibrary/steamapps/common/7 Days To Die
        game_dir: PathBuf,
        #[arg(long, default_value = "7dtd")]
        game: String,
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
    /// Make `spunkpipe://` links open the tray app (or stop doing so).
    UrlHandler {
        #[arg(value_parser = ["enable", "disable", "status"])]
        action: String,
    },
    /// Update SpunkPipe itself to the newest signed release.
    Update {
        /// Only say whether an update exists.
        #[arg(long)]
        check: bool,
        /// Fetch releases from here instead of GitHub (mirrors, testing).
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Serve a published folder over HTTP, optionally re-publishing when the mods change.
    Serve {
        /// The web root written by `publish`.
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value = "0.0.0.0:8080")]
        listen: String,
        /// Re-publish this folder whenever it changes and then stays quiet.
        #[arg(long, requires = "key")]
        watch: Option<PathBuf>,
        #[arg(long)]
        key: Option<PathBuf>,
        #[arg(long)]
        state: Option<PathBuf>,
        #[arg(long, default_value = "7dtd")]
        game: String,
        #[arg(long, default_value_t = 10)]
        keep: usize,
        /// Seconds the folder must stay unchanged before it is published.
        #[arg(long, default_value_t = 30)]
        quiet: u64,
        /// Exclude list, as for `publish`. Defaults to `<key>.exclude` if it exists.
        #[arg(long)]
        exclude: Option<PathBuf>,
    },
    /// (For release builds) Write the signed release manifest for the archives in a folder.
    ReleaseManifest {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        key: PathBuf,
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
            println!("Key fingerprint: {} (friends can check this when they join)", key.public().fingerprint());
        }
        Cmd::Invite { key, host, http, game } => {
            let pk = SecretKey::load(&key)?.public();
            let inv = Invite::new(!http, &host, pk);
            let inv = if let Some(g) = game { inv.with_game(&g) } else { inv };
            println!("{inv}");
            eprintln!("Key fingerprint: {} (tell friends to check it when they join)", inv.public_key.fingerprint());
        }
        Cmd::Publish { source, out, key, state, game, keep, force, exclude } => {
            let exclude = exclude_list(exclude, &key)?;
            let state = state.unwrap_or_else(|| {
                let mut s = key.clone().into_os_string();
                s.push(".state");
                PathBuf::from(s)
            });
            let key = SecretKey::load(&key).context("loading signing key")?;
            let r = publish(&PublishOptions { source, out, state, game, key, force, keep, threads: default_threads(), exclude })?;
            if r.published {
                println!("Published serial {} ({} files, {} new blobs)", r.serial, r.files, r.new_blobs);
            } else {
                println!("Nothing changed; still at serial {} ({} files)", r.serial, r.files);
            }
        }
        Cmd::Sync { invite, game, target, game_dir, state, serial, verify, dry_run } => {
            let invite = Invite::parse(&invite)?;
            let target = match game_dir {
                Some(d) => Some(daemon::target_for_game_dir(&game, &d)?),
                None => target,
            };
            let profile = spunkpipe_profile::builtin(&game);
            let target = match target {
                Some(t) => t,
                None => {
                    let Some(p) = &profile else { bail!("unknown game {game:?}; pass --target") };
                    let Some(install) = p.detect_installs().into_iter().next() else {
                        bail!("could not find a {} install; pass --game-dir <the game's folder> or --target <its Mods folder>", p.name)
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
        Cmd::Join { invite, game, target, game_dir, no_autostart } => {
            let parsed = Invite::parse(&invite)?;
            let game = game.or(parsed.game.clone()).unwrap_or_else(|| "7dtd".to_string());
            let target = match game_dir {
                Some(d) => Some(daemon::target_for_game_dir(&game, &d)?),
                None => target,
            };
            println!("Joining {} (key fingerprint {}).", parsed.host_path, parsed.public_key.fingerprint());
            if let Some(old) = Config::load(&config_path()?)?.games.iter().find(|g| g.game == game) {
                if old.invite != invite.trim() {
                    println!("Note: this replaces the server you were using before ({}).", old.invite.split('#').next().unwrap_or(""));
                }
            }
            let (g, r) = daemon::join_and_sync(&invite, &game, target, &config_path()?)?;
            println!("Saved. Mods folder: {}", g.target.display());
            println!(
                "Synced to serial {}: {} files placed, {} unchanged, {} backed up.",
                r.serial, r.placed, r.unchanged, r.backed_up
            );
            if !no_autostart {
                match tray_binary() {
                    Some(t) => {
                        autostart::enable(&t)?;
                        if let Err(e) = autostart::register_url_handler(&t) {
                            println!("Could not register spunkpipe:// links: {e}");
                        }
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
        Cmd::Folder { game_dir, game } => {
            let target = daemon::target_for_game_dir(&game, &game_dir)?;
            let path = config_path()?;
            let mut cfg = Config::load(&path)?;
            let Some(g) = cfg.games.iter_mut().find(|g| g.game == game) else { bail!("{game} is not configured; run `spunkpipe join` first") };
            let old = std::mem::replace(&mut g.target, target.clone());
            cfg.save(&path)?;
            println!("{game} now keeps {} in step with the server.", target.display());
            if old != target {
                println!("Nothing was removed from the old folder ({}). Run `spunkpipe sync` or use \"Check now\" in the tray to fill the new one.", old.display());
            }
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
        Cmd::UrlHandler { action } => match action.as_str() {
            "enable" => {
                let t = tray_binary().context("spunkpipe-tray not found next to this program")?;
                autostart::register_url_handler(&t)?;
                println!("spunkpipe:// links will open the tray app.");
            }
            "disable" => {
                autostart::unregister_url_handler()?;
                println!("Disabled.");
            }
            _ => println!("{}", if autostart::url_handler_registered() { "enabled" } else { "disabled" }),
        },
        Cmd::Update { check, base_url } => {
            let key = selfupdate::release_key()?;
            let target = selfupdate::current_target().context("no release is published for this platform")?;
            let install_dir = std::env::current_exe()?.parent().context("cannot find the install folder")?.to_path_buf();
            let source = HttpSource::new(base_url.as_deref().unwrap_or(selfupdate::DEFAULT_BASE_URL));
            let opts = UpdateOptions {
                key,
                current_version: selfupdate::CURRENT_VERSION.to_string(),
                target: target.to_string(),
                install_dir,
                check_only: check,
            };
            match selfupdate::update(&source, &opts)? {
                Outcome::UpToDate { version } => println!("Already up to date (version {version})."),
                Outcome::Available { version } => println!("Version {version} is available. Run `spunkpipe update` to install it."),
                Outcome::Updated { from, to } => println!("Updated from {from} to {to}. Restart the tray app to use it."),
            }
        }
        Cmd::Serve { root, listen, watch, key, state, game, keep, quiet, exclude } => {
            let running = spunkpipe_server::serve::start(root.clone(), &listen, 8)?;
            println!("Serving {} on http://{}", root.display(), running.addr);
            if let (Some(src), Some(key)) = (watch, key) {
                let secret = SecretKey::load(&key).context("loading signing key")?;
                let state = state.unwrap_or_else(|| {
                    let mut s = key.clone().into_os_string();
                    s.push(".state");
                    PathBuf::from(s)
                });
                let exclude = exclude_list(exclude, &key)?;
                let opts = PublishOptions { source: src.clone(), out: root, state, game, key: secret, force: false, keep, threads: default_threads(), exclude };
                println!("Watching {} (publishes after {quiet}s of quiet)", src.display());
                std::thread::spawn(move || {
                    spunkpipe_server::watch::watch(
                        &opts,
                        std::time::Duration::from_secs(quiet),
                        std::time::Duration::from_secs(5),
                        &std::sync::atomic::AtomicBool::new(false),
                        |r| match r {
                            Ok(r) if r.published => println!("Published serial {} ({} files)", r.serial, r.files),
                            Ok(_) => {}
                            Err(e) => eprintln!("publish failed: {e}"),
                        },
                    )
                });
            }
            running.wait();
        }
        Cmd::ReleaseManifest { dir, version, key } => {
            let key = SecretKey::load(&key).context("loading release signing key")?;
            let m = spunkpipe_core::release::ReleaseManifest::from_dir(&dir, &version)?;
            let bytes = m.to_bytes()?;
            std::fs::write(dir.join(spunkpipe_core::release::RELEASE_FILE), &bytes)?;
            std::fs::write(dir.join(spunkpipe_core::release::RELEASE_SIG_FILE), key.sign(&bytes))?;
            println!("Signed release {} with {} archives.", m.version, m.assets.len());
        }
        Cmd::Profiles => {
            for id in spunkpipe_profile::builtin_ids() {
                let p = spunkpipe_profile::builtin(id).unwrap();
                println!("{}\t{}", p.id, p.name);
            }
        }
    }
    Ok(())
}

/// Picks the exclude list: the one given, else `<key>.exclude` if that file exists. A list that was
/// asked for but is missing is an error, so a typo can never quietly publish everything.
fn exclude_list(given: Option<PathBuf>, key: &std::path::Path) -> Result<Option<PathBuf>> {
    let path = match given {
        Some(p) => p,
        None => {
            let mut s = key.to_path_buf().into_os_string();
            s.push(".exclude");
            let p = PathBuf::from(s);
            if !p.is_file() {
                return Ok(None);
            }
            p
        }
    };
    if !path.is_file() {
        anyhow::bail!("exclude list {} does not exist", path.display());
    }
    println!("Leaving out what {} lists", path.display());
    Ok(Some(path))
}
