# Developing SpunkPipe

For people working on SpunkPipe itself. Players should read the [README](../README.md); server owners
should read [server.md](server.md).

## Layout

| Crate | Purpose |
|---|---|
| `spunkpipe-core` | Manifest format, hashing, signing, blob store |
| `spunkpipe-profile` | Declarative game profiles (paths, process names, mods-folder detection) |
| `spunkpipe-server` | The publisher |
| `spunkpipe-client` | Sync engine (check, stage, apply), lockfile, rollback |
| `spunkpipe-daemon` | Background engine: polling, modes, game detection, config, autostart |
| `spunkpipe-tray` | The tray app and its dialogs |
| `spunkpipe` | The command-line binary |

Build and test with `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`.

## How a sync works

The server publishes a signed manifest (ed25519) and content-addressed, zstd-compressed blobs. The client
checks the manifest, stages what is missing, then applies it. A lock file records which files SpunkPipe put
in place, and only those are ever removed. Files whose hash already matches are not downloaded. A file that
differs is replaced after a copy is saved to `<data>/<game>/backup/<timestamp>/`.

## Making a release

1. `spunkpipe keygen release.key`, commit the public key line to `release-key.pub`, and store the secret
   key's contents as the repository secret `RELEASE_SIGNING_KEY`. Keep a backup: lose it and installed
   copies can no longer be updated.
2. Set the version in `Cargo.toml`, merge, then run **Actions, Release** with that version. It must match
   `Cargo.toml`.

The workflows only run when started by hand (Actions, then CI or Release), so nothing runs, or is billed, on
every push. Run CI before merging a change you are unsure about.

A release builds Linux (musl), macOS (Apple Silicon and Intel) and Windows, signs `release.json`, and
publishes the archives, `SHA256SUMS`, the install scripts and ready-to-submit Homebrew, winget and AUR
package files. The install scripts read `releases/latest/download/VERSION`, which can lag a few minutes
behind a new release.
