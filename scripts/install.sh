#!/bin/sh
# Installs SpunkPipe on Linux or macOS.
#
#   curl -fsSL https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/install.sh | sh -s -- --invite 'spunkpipe://...'
#
# Options:  --invite URI   join a server straight away
# Environment:  SPUNKPIPE_BASE_URL (where releases are fetched from), SPUNKPIPE_INSTALL_DIR
set -eu

base="${SPUNKPIPE_BASE_URL:-https://github.com/Spunky-Friends/spunkpipe/releases/latest/download}"
dir="${SPUNKPIPE_INSTALL_DIR:-$HOME/.local/bin}"
invite=""

while [ $# -gt 0 ]; do
  case "$1" in
    --invite) invite="${2:?--invite needs a value}"; shift 2 ;;
    --invite=*) invite="${1#--invite=}"; shift ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target="x86_64-unknown-linux-musl" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  *) echo "No SpunkPipe build for $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

fetch() { # url dest
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"; else wget -qO "$2" "$1"; fi
}
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch "$base/VERSION" "$tmp/VERSION"
version=$(tr -d '[:space:]' < "$tmp/VERSION")
case "$version" in *[!0-9.]*|"") echo "Bad version from server: $version" >&2; exit 1 ;; esac
archive="spunkpipe-$version-$target.tar.gz"

echo "Downloading SpunkPipe $version for $target..."
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"
fetch "$base/$archive" "$tmp/$archive"

want=$(grep "  $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$want" ] || { echo "No checksum for $archive" >&2; exit 1; }
[ "$(sha256 "$tmp/$archive")" = "$want" ] || { echo "Checksum mismatch for $archive: refusing to install" >&2; exit 1; }

mkdir -p "$dir"
tar -xzf "$tmp/$archive" -C "$dir"
echo "Installed to $dir"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Note: $dir is not on your PATH. Add it, or run $dir/spunkpipe directly." ;;
esac

if [ -n "$invite" ]; then
  "$dir/spunkpipe" join "$invite"
  # Start the tray now rather than waiting for the next login.
  if [ "$(uname -s)" = "Darwin" ] && [ -d "$dir/SpunkPipe.app" ]; then
    open "$dir/SpunkPipe.app" || true
  else
    (nohup "$dir/spunkpipe-tray" >/dev/null 2>&1 &) || true
  fi
else
  echo "Next: spunkpipe join '<the invite you were sent>'"
fi
