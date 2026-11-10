use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use spunkpipe_client::{sync, HttpSource, SyncOptions};
use spunkpipe_core::{par::default_threads, Invite, PublicKey, SecretKey};
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
    /// List built-in game profiles.
    Profiles,
}

fn data_dir() -> Result<PathBuf> {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
    };
    Ok(base.context("cannot work out a data folder; pass --state")?.join("spunkpipe"))
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
        Cmd::Publish { source, out, key, game, keep, force } => {
            let key = SecretKey::load(&key).context("loading signing key")?;
            let r = publish(&PublishOptions { source, out, game, key, force, keep, threads: default_threads() })?;
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
                None => data_dir()?.join(&game),
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
        Cmd::Profiles => {
            for id in spunkpipe_profile::builtin_ids() {
                let p = spunkpipe_profile::builtin(id).unwrap();
                println!("{}\t{}", p.id, p.name);
            }
        }
    }
    Ok(())
}
