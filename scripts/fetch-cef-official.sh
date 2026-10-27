#!/bin/sh
# EXPERIMENTAL, not used by CI. Puts the OFFICIAL Spotify CEF (no H.264/AAC) at $1 in the layout the `cef` crate expects,
# optionally dropping a libffmpeg.dylib into the framework. See docs/cef-ffmpeg.md: the swap does NOT work with the
# official build (ffmpeg is statically linked into the framework), so a given libffmpeg is refused unless the framework
# actually has a load command for it.
#   fetch-cef-official.sh <dir> [path/to/libffmpeg.dylib]
set -e
DEST=${1:?usage: fetch-cef-official.sh <dir> [libffmpeg.dylib]}
FFMPEG=$2
NAME='cef_binary_156.0.8+g426920c+chromium-156.0.8078.48_macosarm64_minimal.tar.bz2'
SHA256=c51efd8cf7bfd1368ac5ee09094b98645b27f485275fc992173008b3abc07576
SHA1=58510b0c4b4bfd5f02b3d2e027a02266168a7ce3 # from cef-builds.spotifycdn.com/index.json
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/cef.tar.bz2" "https://cef-builds.spotifycdn.com/$(printf %s "$NAME" | sed 's/+/%2B/g')"
echo "$SHA256  $TMP/cef.tar.bz2" | shasum -a 256 -c -
tar -xjf "$TMP/cef.tar.bz2" -C "$TMP"
SRC=$TMP/${NAME%.tar.bz2}
FW="$SRC/Release/Chromium Embedded Framework.framework"
if [ -n "$FFMPEG" ]; then
  if ! otool -L "$FW/Chromium Embedded Framework" | grep -q libffmpeg; then
    echo "refusing: this framework has no libffmpeg load command (ffmpeg is static); a dropped-in dylib would never be loaded" >&2
    exit 1
  fi
  cp "$FFMPEG" "$FW/Libraries/libffmpeg.dylib"
fi
rm -rf "$DEST"; mkdir -p "$DEST"
cp -R "$SRC/include" "$SRC/libcef_dll" "$SRC/cmake" "$SRC/CMakeLists.txt" "$FW" "$DEST/"
printf '{"type":"minimal","name":"%s","sha1":"%s"}\n' "$NAME" "$SHA1" > "$DEST/archive.json"
