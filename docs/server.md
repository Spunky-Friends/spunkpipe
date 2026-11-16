# Running a SpunkPipe server

For whoever runs the game server and publishes the mods. Players should read the
[README](../README.md) instead.

You run `spunkpipe publish` against the server's mods folder. It writes a signed manifest and
content-addressed blobs to a folder that any web server can serve. Players install SpunkPipe and paste an
invite, and their mods folder is kept matching.

## Set up

Get the `spunkpipe` binary onto the server (the install script in the README works there too), then:

```bash
spunkpipe keygen ~/spunkpipe.key
spunkpipe publish --source /path/to/server/Mods --out /var/www/spunkpipe --key ~/spunkpipe.key
spunkpipe invite --key ~/spunkpipe.key --host mods.example.com
```

`keygen` makes the signing key. Keep it private and back it up: players trust mods because they are signed
with it, and a new key means everyone has to join again.

`publish` reads the mods folder and writes only what changed. It does nothing when nothing did.

`invite` prints the invite to send players and a **key fingerprint**. Give players the fingerprint through a
different channel from the link, so they can check it when SpunkPipe shows it to them.

Serve `/var/www/spunkpipe` over HTTPS with any web server.

## Leaving things out

To keep server-only mods (or anything else) away from players, put patterns in `~/spunkpipe.key.exclude`
(the key file's name plus `.exclude`), or point at another file with `--exclude`. It is read on every
publish and stays on the server. One pattern per line, `.gitignore` style, relative to the mods folder:

```
# server-side only
/CurryGiveSkillPoints/
*.log
```

`Name` matches a file or folder of that name anywhere, `/Name` only at the top level, `Name/` only
folders, and a leading `!` brings back something an earlier line left out. Excluded folders are not read at
all. If you add a pattern for something already published, players' copies are removed on their next sync
(backed up first). A list you ask for with `--exclude` that does not exist is an error.

Also leave out anything that differs per player, and anything with secrets in it.

## Web server

Publishing is just files in a folder. Put the folder behind HTTPS and allow only `GET` of `latest`,
`manifests/` and `blobs/`. Blobs never change once written, so they can be cached for a long time. Example
for nginx:

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

Keep rate limits loose: a first sync of a large modpack is thousands of requests from one address. Do not
put a download-size cap, or a CDN that forbids large non-web files, in front of it.

If you have no web server, `spunkpipe serve --root /var/www/spunkpipe --listen 0.0.0.0:8080` serves the
published folder (read-only, with resumable downloads). Add `--watch /path/to/Mods --key ~/spunkpipe.key` to
re-publish automatically once the mods folder has been quiet for 30 seconds. Put a reverse proxy in front of
it if you want HTTPS.

## Publishing on a schedule

Run `spunkpipe publish` from a systemd timer (or cron). Run it as a user that can **read** the mods folder
and **write** the web root, and keep the key and its state folder out of the web root.

Wait until the mods folder has been still for a minute before publishing, so a mod that is still being
copied in is not published half-finished.

## Commands

| Command | What it does |
|---|---|
| `spunkpipe keygen <file>` | Make a signing key |
| `spunkpipe publish` | Publish the mods folder. `--source`, `--out`, `--key`, `--exclude` |
| `spunkpipe invite` | Print the invite and key fingerprint. `--key`, `--host` |
| `spunkpipe serve` | Serve the published folder, optionally re-publishing on change. `--root`, `--listen`, `--watch`, `--key`, `--exclude` |
