#!/bin/sh
# Fills in the package manager templates for a release.
# Usage: packaging/render.sh <version> <SHA256SUMS> <out-dir>
set -eu
version=$1 sums=$2 out=$3
here=$(cd "$(dirname "$0")" && pwd)

sha_of() { # target extension
  f="spunkpipe-$version-$1.$2"
  h=$(grep "  $f\$" "$sums" | cut -d' ' -f1)
  [ -n "$h" ] || { echo "no checksum for $f in $sums" >&2; exit 1; }
  echo "$h"
}

linux=$(sha_of x86_64-unknown-linux-musl tar.gz)
darm=$(sha_of aarch64-apple-darwin tar.gz)
dintel=$(sha_of x86_64-apple-darwin tar.gz)
win=$(sha_of x86_64-pc-windows-msvc zip)

fill() { # template dest
  sed -e "s/@VERSION@/$version/g" \
      -e "s/@SHA_X86_64_LINUX@/$linux/g" \
      -e "s/@SHA_AARCH64_DARWIN@/$darm/g" \
      -e "s/@SHA_X86_64_DARWIN@/$dintel/g" \
      -e "s/@SHA_X86_64_WINDOWS@/$win/g" "$1" > "$2"
  if grep -q '@[A-Z_0-9]*@' "$2"; then echo "unfilled placeholder in $2" >&2; exit 1; fi
}

mkdir -p "$out"
fill "$here/homebrew/spunkpipe.rb.in" "$out/homebrew-spunkpipe.rb"
fill "$here/aur/PKGBUILD.in" "$out/PKGBUILD"
for f in "$here"/winget/*.in; do
  base=$(basename "$f" .in)
  fill "$f" "$out/winget-$base"
done
echo "rendered package files into $out"
