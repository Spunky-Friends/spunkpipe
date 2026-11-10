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
