# SpunkPipe

Keeps your friends' game mods in sync with your game server. Fast, small, open source.

**Status: early development.** First target is 7 Days to Die.

## How it works

- The server owner runs `spunkpipe publish` against the server's mods folder. It writes a signed
  manifest and content-addressed blobs to a directory that any web server can serve.
- Friends install SpunkPipe, paste an invite (`spunkpipe://host#publickey`), and the client keeps
  their mods folder matching the server. Only changed files are downloaded.
- Only files SpunkPipe installed are ever touched. Your own extra mods are left alone.

## Using it

Server owner:

```bash
spunkpipe keygen ~/spunkpipe.key
spunkpipe publish --source /path/to/server/Mods --out /var/www/spunkpipe --key ~/spunkpipe.key
spunkpipe invite --key ~/spunkpipe.key --host mods.example.com
```

Serve `/var/www/spunkpipe` over HTTPS with any web server, and send friends the invite.

Friends:

```bash
spunkpipe join "spunkpipe://mods.example.com#<publickey>"
```

That saves the invite, does the first sync and registers `spunkpipe-tray` to start at login.
The tray menu offers Automatic updates, Ask me before downloading, or Paused, plus "Check now".
Downloads are staged first and only swapped in once the game is closed. Use `spunkpipe run`
instead of the tray on machines with no desktop.

### Leaving things out

To keep server-only mods (or anything else) away from friends, put patterns in `~/spunkpipe.key.exclude`
(the key file's name plus `.exclude`), or point at another file with `--exclude`. It is read on every
publish and stays on the server. One pattern per line, `.gitignore` style, relative to the mods folder:

```
# server-side only
/CurryGiveSkillPoints/
*.log
```

`Name` matches a file or folder of that name anywhere, `/Name` only at the top level, `Name/` only
folders, and a leading `!` brings back something an earlier line left out. Excluded folders are not
read at all. If you add a pattern for something already published, friends' copies are removed on
their next sync (backed up first). A list you ask for with `--exclude` that does not exist is an error.

### Serving it yourself

If you have no web server, `spunkpipe serve --root /var/www/spunkpipe --listen 0.0.0.0:8080` serves the
published folder (read-only, with resumable downloads). Add `--watch /path/to/Mods --key ~/spunkpipe.key`
to re-publish automatically once the mods folder has been quiet for 30 seconds. Put a reverse proxy in
front of it if you want HTTPS.

## Install

Linux and macOS:

```bash
curl -fsSL https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/install.sh | sh -s -- --invite 'spunkpipe://...'
```

Windows (PowerShell):

```powershell
& ([scriptblock]::Create((irm https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/install.ps1))) -Invite 'spunkpipe://...'
```

Leave the invite out to just install, then run `spunkpipe join '<invite>'` later. Update with
`spunkpipe update`; it only accepts releases signed with SpunkPipe's release key, and never downgrades.
Each release also publishes ready-to-submit Homebrew, winget and AUR package files.

### Invite links

`spunkpipe invite` prints the invite and a key fingerprint. Clicking a `spunkpipe://` link asks you to
confirm the server and the fingerprint first (mods are code, so a link must never join a server on its
own), and warns if it would replace the server you already use. Check the fingerprint with whoever sent
you the link.

## Making a release (maintainers)

1. `spunkpipe keygen release.key`, commit the public key line to `release-key.pub`, and store the secret
   key's contents as the repository secret `RELEASE_SIGNING_KEY`. Keep a backup: lose it and installed
   copies can no longer be updated.
2. Set the version in `Cargo.toml`, merge, then run **Actions, Release** with that version.

It builds Linux (musl), macOS (Apple Silicon and Intel) and Windows, signs `release.json`, and publishes
the archives, `SHA256SUMS`, the install scripts and the package files.

## Layout

| Crate | Purpose |
|---|---|
| `spunkpipe-core` | Manifest format, hashing, signing, blob store |
| `spunkpipe-profile` | Declarative game profiles (paths, process names) |
| `spunkpipe-server` | The publisher |
| `spunkpipe-client` | Sync engine (check, stage, apply), lockfile, rollback |
| `spunkpipe-daemon` | Background engine: polling, modes, game detection, config, autostart |
| `spunkpipe-tray` | The tray app |
| `spunkpipe` | The command-line binary |

## Licence

Dual licensed under MIT or Apache-2.0, at your option.
