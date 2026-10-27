#!/bin/sh
# Puts a CEF with H.264/AAC (proprietary_codecs=true ffmpeg_branding=Chrome) at $1, in the layout the `cef` crate expects.
# Spotify's prebuilt CEF has no H.264, which most anime/streaming sites need. This is a third-party build
# (github.com/obs-rust/cef, built on a contributor's Mac, pinned by hash): to move to a newer Chromium,
# update VERSION/NAME/SHA256 here from a newer obs-rust release, or build CEF yourself.
# Distributing H.264/AAC decoders is subject to patent licensing in your jurisdiction.
set -e
DEST=${1:?usage: fetch-cef.sh <dir>}
TAG=8078
NAME='cef_binary_156.0.8+g426920c+chromium-156.0.8078.48_macosarm64_minimal.tar.bz2' # what the cef crate checks in archive.json
SHA256=2ddd78dd796af956728984747aaba4d859658354f00c6035be975b9f322f95cb
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/cef.tar.xz" "https://github.com/obs-rust/cef/releases/download/$TAG/cef_binary_${TAG}_macos_arm64.tar.xz"
echo "$SHA256  $TMP/cef.tar.xz" | shasum -a 256 -c -
tar -xf "$TMP/cef.tar.xz" -C "$TMP"
SRC=$TMP/cef_binary_${TAG}_macos_arm64
rm -rf "$DEST"; mkdir -p "$DEST"
cp -R "$SRC/include" "$SRC/libcef_dll" "$SRC/cmake" "$SRC/CMakeLists.txt" "$SRC/Release/Chromium Embedded Framework.framework" "$DEST/"
printf '{"type":"minimal","name":"%s","sha1":"%s"}\n' "$NAME" "$(shasum "$TMP/cef.tar.xz" | cut -d' ' -f1)" > "$DEST/archive.json"
