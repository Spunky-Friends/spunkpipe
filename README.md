# SpunkPipe

Keeps your game mods the same as your friend's server. You install it once, and it downloads
only what changed, from then on.

**Status: early development.** Works with 7 Days to Die on Linux, Windows and macOS. Most testing so far
is on Linux.

Running a server? See [docs/server.md](docs/server.md). Working on SpunkPipe? See
[docs/development.md](docs/development.md).

## How it works

The server owner publishes their mods folder as a signed list of files. SpunkPipe on your computer
compares that list with your `Mods` folder and downloads only the files that are missing or different.
Everything is checked against the server's signature before it is used, and new files are only put in place
while the game is closed. Your own extra mods are left alone, and anything of yours it replaces is backed up
first. You need an invite from the server owner to get started.

## Install

### Windows

Open PowerShell and paste this, with your invite in place of `spunkpipe://...`:

```powershell
& ([scriptblock]::Create((irm https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/install.ps1))) -Invite 'spunkpipe://...'
```

It downloads SpunkPipe, checks it, installs it to `%LOCALAPPDATA%\Programs\SpunkPipe`, downloads your
mods and sets itself to start when you log in.

**Installing by hand instead.** Download `spunkpipe-<version>-x86_64-pc-windows-msvc.zip` from the
[releases page](https://github.com/Spunky-Friends/spunkpipe/releases/latest) and then:

1. Unzip it to a folder you will keep, such as `%LOCALAPPDATA%\Programs\SpunkPipe`. Do not run it from
   Downloads: if you delete the folder later, SpunkPipe stops working.
2. Open PowerShell in that folder and run `.\spunkpipe.exe join "spunkpipe://..."` with your invite.
3. Start `spunkpipe-tray.exe` once. From then on it starts by itself when you log in.

Windows may warn that the program is from an unknown publisher. Choose **More info**, then **Run anyway**.

### Linux and macOS

```bash
curl -fsSL https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/install.sh | sh -s -- --invite 'spunkpipe://...'
```

### Without an invite

Leave the invite part out to only install, then run this later:

```bash
spunkpipe join 'spunkpipe://...'
```

On Windows, open a new terminal first (the installer adds SpunkPipe to your PATH).

## Your first sync

1. SpunkPipe finds the game and its mods folder. If it cannot tell which folder to use, it asks you to
   choose one (see [Which folder](#which-folder)). Nothing is downloaded until you have chosen.
2. It downloads only the files you do not already have. If you already have the server's mods, this takes
   seconds.
3. It adds a small tray icon, and starts it at login.

When it says **Synced**, start the game.

### If you click an invite link instead

If SpunkPipe is already installed, clicking a `spunkpipe://` link opens a window showing the server and
its key fingerprint. **Check the fingerprint matches what the server owner told you.** Mods can run
programs on your computer, so a link never sets anything up without you saying yes.

## Day to day

The tray icon has:

- **Automatic updates**: updates download in the background and are put in place when the game is closed.
  This is the default.
- **Ask me before downloading**: it tells you when there is an update and waits.
- **Paused**: does nothing until you change it back.
- **Check now**: looks for an update straight away.

Updates are never swapped in while the game is running. On a machine with no desktop (or no tray), run
`spunkpipe run` instead.

## Which folder

7 Days to Die loads mods from two places: the `Mods` folder in the game's install folder, and a `Mods`
folder in its user-data folder. SpunkPipe looks in both, including every Steam library you have set up and
the Proton prefix on Linux:

| System | User-data folder |
|---|---|
| Linux (native) | `~/.local/share/7DaysToDie/Mods` |
| Linux (Proton) | `<steam library>/steamapps/compatdata/251570/pfx/drive_c/users/steamuser/AppData/Roaming/7DaysToDie/Mods` |
| Windows | `%APPDATA%\7DaysToDie\Mods` |
| macOS | `~/Library/Application Support/7DaysToDie/Mods` |

It keeps the folder that already holds your mods. On a fresh install with no mods anywhere, it uses the
install folder's `Mods`. **If it cannot tell** (mods in more than one place, two installs, or no game
found), it asks you on your first sync: the tray shows a folder chooser, and `spunkpipe join` in a terminal
lists the folders it found.

To say where it is yourself:

```bash
spunkpipe join "<invite>" --game-dir "/path/to/7 Days To Die"   # the game's install folder
spunkpipe join "<invite>" --target   "/path/to/Mods"            # the Mods folder itself
```

To point an already-joined game somewhere else: `spunkpipe folder "/path/to/7 Days To Die"`. Nothing is
removed from the old folder.

Use one place only. The same mod in both folders makes the game load it twice.

## What happens to your files

- **Only what changed is downloaded.** Every file in the folder is compared with the server's list.
  Matching files are skipped.
- **Your own extra mods are left alone.** SpunkPipe never touches a file the server has never had.
- **A file that differs from the server's is replaced**, and your copy is saved first in
  `backup/<timestamp>/` inside SpunkPipe's data folder (below). The same goes for a file the server removes
  that you have changed.
- **Local edits to server mods do not stick.** The server's copy wins on the next sync. If a setting must be
  different for you, ask the server owner to leave that file out.
- **Downloads are staged first.** The tray and `spunkpipe run` only move them into place once the game is
  closed. `join` and `sync` apply straight away, so close the game first.

SpunkPipe keeps its own files here:

| | Settings | Data (lock file, staging, backups) |
|---|---|---|
| Linux | `~/.config/spunkpipe/config.toml` | `~/.local/share/spunkpipe/<game>/` |
| macOS | `~/Library/Application Support/spunkpipe/config.toml` | `~/Library/Application Support/spunkpipe/<game>/` |
| Windows | `%APPDATA%\spunkpipe\config.toml` | `%APPDATA%\spunkpipe\<game>\` |

## Troubleshooting

- **"could not find a 7 Days to Die install"**: tell it where the game is with `--game-dir` or `--target`
  (see [Which folder](#which-folder)).
- **It is downloading a huge amount**: it is probably pointed at a folder that does not hold your mods.
  Stop it (close the window or press Ctrl+C), then join again with the right `--target`. Nothing is moved
  into the game folder until the download is complete.
- **A mod setting keeps going back**: the server publishes that file. Ask the server owner to leave it out.
- **The game loads a mod twice**: the mod is in both `Mods` folders. Delete one copy.
- **Clicking the link opens a web search, or nothing happens**: SpunkPipe is not installed yet, or has not
  registered itself for links. Install it, then run `spunkpipe url-handler enable`.
- **I want to undo an update**: `spunkpipe sync --invite "<invite>" --serial N` goes back to an older
  version. Your replaced files are also in the `backup` folder.

## Commands

You will rarely need these. The tray does the day-to-day work.

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

`spunkpipe update` only accepts releases signed with SpunkPipe's release key, and never installs an older
version.

## Licence

Dual licensed under MIT or Apache-2.0, at your option.
