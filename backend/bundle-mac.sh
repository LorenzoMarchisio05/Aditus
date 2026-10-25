#!/bin/sh
# Builds Aditus.app (debug by default; `./bundle-mac.sh release` for the optimised one).
# Needs: CEF_PATH (prebuilt CEF), ninja, and `bundle-cef-app` from github.com/tauri-apps/cef-rs
# (cargo install cef --features build-util --bin bundle-cef-app) on PATH or in $BUNDLER.
set -e
cd "$(dirname "$0")"
PROFILE=${1:-debug}
export CEF_PATH=${CEF_PATH:-$HOME/.local/share/cef}
export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:$PATH
FLAGS=; [ "$PROFILE" = release ] && FLAGS=--release
(cd ../ui && npm run build)
cargo build $FLAGS
OUT=target/bundle
rm -rf "$OUT/Aditus.app"; mkdir -p "$OUT"
${BUNDLER:-bundle-cef-app} aditus -o "$OUT" -i dev.aditus.browser -d Aditus -v 0.1.1
APP=$OUT/aditus.app
[ -d "$APP" ] && mv "$APP" "$OUT/Aditus.app"
APP=$OUT/Aditus.app
if [ "$PROFILE" = release ]; then
  # bundle-cef-app always picks the debug binaries.
  cp target/release/aditus "$APP/Contents/MacOS/aditus"
  for h in "$APP"/Contents/Frameworks/aditus\ Helper*.app; do cp target/release/aditus_helper "$h/Contents/MacOS/$(basename "$h" .app)"; done
fi
PL=$APP/Contents/Info.plist
B=/usr/libexec/PlistBuddy
put() { $B -c "Delete :$1" "$PL" 2>/dev/null || true; $B -c "Add :$1 $2" "$PL"; }
put CFBundleURLTypes array
$B -c 'Add :CFBundleURLTypes:0 dict' -c 'Add :CFBundleURLTypes:0:CFBundleURLName string "Web site URL"' \
   -c 'Add :CFBundleURLTypes:0:CFBundleURLSchemes array' -c 'Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string http' -c 'Add :CFBundleURLTypes:0:CFBundleURLSchemes:1 string https' "$PL"
# Default-browser pickers also want an HTML document type, like Chrome and Safari have.
put CFBundleDocumentTypes array
$B -c 'Add :CFBundleDocumentTypes:0 dict' -c 'Add :CFBundleDocumentTypes:0:CFBundleTypeName string "HTML document"' -c 'Add :CFBundleDocumentTypes:0:CFBundleTypeRole string Viewer' \
   -c 'Add :CFBundleDocumentTypes:0:LSItemContentTypes array' -c 'Add :CFBundleDocumentTypes:0:LSItemContentTypes:0 string public.html' -c 'Add :CFBundleDocumentTypes:0:LSItemContentTypes:1 string public.xhtml' "$PL"
put NSCameraUsageDescription 'string "A website you opened in Aditus wants to use your camera."'
put NSMicrophoneUsageDescription 'string "A website you opened in Aditus wants to use your microphone."'
put NSLocationWhenInUseUsageDescription 'string "A website you opened in Aditus wants to know your location."'
put CFBundleIconFile 'string icon'
cp resources/icons/icon.icns "$APP/Contents/Resources/icon.icns"
rm -rf "$APP/Contents/Resources/dist"; cp -R ../ui/dist "$APP/Contents/Resources/dist"
# Trim: English only, no software GL.
FW="$APP/Contents/Frameworks/Chromium Embedded Framework.framework"
find "$FW/Resources" -maxdepth 1 -name '*.lproj' ! -name 'en.lproj' ! -name 'en_US.lproj' -exec rm -rf {} + 2>/dev/null || true
rm -f "$FW/Libraries/libvk_swiftshader.dylib" "$FW/Libraries/vk_swiftshader_icd.json" "$FW/Libraries/libEGL.dylib.disabled" 2>/dev/null || true
codesign --force --deep -s - "$APP" 2>&1 | tail -1 || true
echo "$APP"
