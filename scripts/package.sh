#!/bin/sh
# Builds the release archive for one target from already-built binaries.
# Usage: scripts/package.sh <version> <target> <bin-dir> <out-dir>
set -eu
version=$1 target=$2 bin=$3 out=$4
stage=$(mktemp -d)
exe=""
case "$target" in *windows*) exe=".exe" ;; esac

cp "$bin/spunkpipe$exe" "$bin/spunkpipe-tray$exe" "$stage/"
cp LICENSE-MIT LICENSE-APACHE README.md "$stage/"

case "$target" in
  *apple-darwin)
    # The app bundle lets macOS open spunkpipe:// links with the tray program next to it.
    mkdir -p "$stage/SpunkPipe.app/Contents/MacOS"
    sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$stage/SpunkPipe.app/Contents/Info.plist"
    ln -s ../../../spunkpipe-tray "$stage/SpunkPipe.app/Contents/MacOS/spunkpipe-tray"
    ;;
esac

mkdir -p "$out"
name="spunkpipe-$version-$target"
case "$target" in
  *windows*)
    (cd "$stage" && if command -v zip >/dev/null 2>&1; then zip -qr "$out/$name.zip" .; else 7z a -tzip "$out/$name.zip" . >/dev/null; fi)
    ;;
  *)
    tar -C "$stage" -czf "$out/$name.tar.gz" .
    ;;
esac
rm -rf "$stage"
echo "built $name"
