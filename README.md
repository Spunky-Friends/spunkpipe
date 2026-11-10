# SpunkPipe

Keeps your friends' game mods in sync with your game server. Fast, small, open source.

**Status: early development.** First target is 7 Days to Die.

## How it works

- The server owner runs `spunkpipe publish` against the server's mods folder. It writes a signed
  manifest and content-addressed blobs to a directory that any web server can serve.
- Friends install SpunkPipe, paste an invite (`spunkpipe://host#publickey`), and the client keeps
  their mods folder matching the server. Only changed files are downloaded.
- Only files SpunkPipe installed are ever touched. Your own extra mods are left alone.

## Layout

| Crate | Purpose |
|---|---|
| `spunkpipe-core` | Manifest format, hashing, signing, blob store |
| `spunkpipe-profile` | Declarative game profiles (paths, process names) |
| `spunkpipe-server` | The publisher |
| `spunkpipe-client` | Sync engine, lockfile, rollback |
| `spunkpipe` | The command-line binary |

## Licence

Dual licensed under MIT or Apache-2.0, at your option.
