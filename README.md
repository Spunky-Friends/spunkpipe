# SpunkPipe

Keeps your friends' game mods in sync with your game server. Fast, small, open source.

**Status: early development.** First target is 7 Days to Die. Builds for Linux, macOS and Windows; most testing so far is on Linux.

## How it works

- The server owner runs `spunkpipe publish` against the server's mods folder. It writes a signed
  manifest and content-addressed blobs to a directory that any web server can serve.
- Friends install SpunkPipe, paste an invite (`spunkpipe://host#publickey`), and the client keeps
  their mods folder matching the server. Only changed files are downloaded.
- Your own extra mods are left alone. A file at the same path as one of the server's but with
  different contents is replaced with the server's, after your copy is saved to a backup folder (see
  [What happens to your files](#what-happens-to-your-files)).

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
If it says it cannot find the game, or your mods live somewhere else, see
[Where the game's mods folder is](#where-the-games-mods-folder-is).
The tray menu offers Automatic updates, Ask me before downloading, or Paused, plus "Check now".
Downloads are staged first and only swapped in once the game is closed. Use `spunkpipe run`
instead of the tray on machines with no desktop.

### Where the game's mods folder is

SpunkPipe looks for the game in Steam's default folder and in every Steam library you have set up
(including ones on other drives), and manages the game's `Mods` folder inside it. Tell it where
to look instead with either of these, on `join` or `sync`:

```bash
spunkpipe join "<invite>" --game-dir "/path/to/7 Days To Die"   # the game's install folder
spunkpipe join "<invite>" --target   "/path/to/Mods"            # the Mods folder itself
```

To point an already-joined game somewhere else: `spunkpipe folder "/path/to/7 Days To Die"`. Nothing
is removed from the old folder.

**7 Days to Die loads mods from two places**: the `Mods` folder in the install folder, and a `Mods`
folder in the game's user-data folder. If your mods are in the second one, use `--target` with it.
SpunkPipe does not look there on its own. Typical user-data folders:

| System | Folder |
|---|---|
| Linux (native) | `~/.local/share/7DaysToDie/Mods` |
| Linux (Proton) | `<steam library>/steamapps/compatdata/251570/pfx/drive_c/users/steamuser/AppData/Roaming/7DaysToDie/Mods` |
| Windows | `%APPDATA%\7DaysToDie\Mods` |

Pick one place and use it everywhere: the same mod in both folders makes the game load it twice.

### What happens to your files

- **Only what changed is downloaded.** Every file in the folder is compared by hash with the server's
  list. Matching files are skipped, so joining with mods you already have costs almost nothing.
- **Changes are staged first.** The tray and `spunkpipe run` only move them into place once the game
  is closed. `join` and `sync` apply straight away, so close the game first.
- **A file that differs from the server's is replaced**, and your copy is saved first in
  `backup/<timestamp>/` inside SpunkPipe's data folder. The same applies when the server removes a file
  you have changed. Files the server has never had are never touched.
- **Local edits to published files don't stick.** The server's copy wins on the next sync.
  Settings that must differ per player belong in the server's exclude list (see below).

SpunkPipe keeps its files here:

| | Config | Data (lock file, staging, backups) |
|---|---|---|
| Linux | `~/.config/spunkpipe/config.toml` | `~/.local/share/spunkpipe/<game>/` |
| macOS | `~/Library/Application Support/spunkpipe/config.toml` | `~/Library/Application Support/spunkpipe/<game>/` |
| Windows | `%APPDATA%\spunkpipe\config.toml` | `%APPDATA%\spunkpipe\<game>\` |

Set `SPUNKPIPE_HOME` to keep everything in one folder instead.

### Commands

| Command | What it does |
|---|---|
| `spunkpipe join <invite>` | Save an invite, do the first sync, start at login. `--game-dir`, `--target`, `--no-autostart` |
| `spunkpipe status` | Show the configured games and what is installed |
| `spunkpipe sync --invite <invite>` | One sync without saving anything. `--dry-run` shows what would change, `--verify` re-checks every file, `--serial N` goes back to an older version |
| `spunkpipe mode auto\|manual\|paused` | Update automatically, ask first, or do nothing until asked |
| `spunkpipe folder <game folder>` | Change which game folder is managed |
| `spunkpipe run` | The background updater without a tray icon |
| `spunkpipe autostart` / `url-handler` | Start at login / open `spunkpipe://` links, `enable` or `disable` |
| `spunkpipe update` | Update SpunkPipe itself. `--check` only reports |
| `spunkpipe keygen`, `invite`, `publish`, `serve` | For the server owner |

### Troubleshooting

- **"could not find a 7 Days to Die install"**: pass `--game-dir` or `--target` as above.
- **It downloaded far more than expected**: it was probably pointed at a folder that does not hold your
  mods (see the user-data folder note above). Stop it, then join again with the right `--target`.
  Nothing is moved into the game folder until the download is complete.
- **A mod setting keeps reverting**: the server publishes that file. Ask the server owner to exclude it.

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

### Running the server

Publishing is just files in a folder, so any static web server works. Put the folder behind HTTPS and
allow only `GET` of `latest`, `manifests/` and `blobs/`. Blobs never change once written, so they
can be cached for a long time. Example for nginx:

```nginx
limit_conn_zone $binary_remote_addr zone=mods_conn:10m;
limit_req_zone  $binary_remote_addr zone=mods_req:10m rate=300r/s;

server {
    listen 443 ssl;
    server_name mods.example.com;
    root /var/www/spunkpipe;
    autoindex off;
    limit_conn mods_conn 64;                    # a first sync opens up to 16 connections
    limit_req  zone=mods_req burst=1000 nodelay; # and thousands of small requests
    location = /latest        { add_header Cache-Control "no-cache"; default_type text/plain; }
    location ^~ /manifests/   { add_header Cache-Control "public, max-age=3600"; }
    location ^~ /blobs/       { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location /                { return 404; }
}
```

Keep rate limits loose: the first sync of a large modpack is thousands of requests from one address.
Do not put a download-size cap or a CDN that forbids large non-web files in front of it.

To publish on a schedule, run `spunkpipe publish` from a systemd timer (or cron). It reads the mods
folder, writes only what changed, and does nothing when nothing did. Run it as a user that can **read**
the mods folder and write the web root, and keep the key and its state folder out of the web root.
Wait until the mods folder has been still for a minute before publishing, so a mod that is still being
copied in is not published half-finished.

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

The workflows only run when started by hand (Actions, then CI or Release), so nothing runs, or is billed,
on every push. Run CI before merging a change you are unsure about.

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
