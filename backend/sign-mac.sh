#!/bin/sh
# Signs (and optionally notarizes) target/bundle/Aditus.app. Untested: needs a Developer ID certificate.
#   SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" ./sign-mac.sh
#   NOTARY_PROFILE=<notarytool keychain profile>, or in CI NOTARY_KEY (base64 .p8) + NOTARY_KEY_ID + NOTARY_ISSUER,
#   to also notarize and staple.
set -e
cd "$(dirname "$0")"
APP=target/bundle/Aditus.app
S="codesign --force --options runtime --timestamp --sign $SIGN_IDENTITY"
# Inside out: framework, helpers, then the app. Follows CEF's cefclient layout: only the helper processes (renderer, GPU:
# V8 JIT) get entitlements.plist; the framework and the main app get none. disable-library-validation is dropped because
# everything is signed with one identity; if a signed build fails to launch, add it to the helpers first.
$S "$APP/Contents/Frameworks/Chromium Embedded Framework.framework"
for h in "$APP"/Contents/Frameworks/aditus\ Helper*.app; do $S --entitlements entitlements.plist "$h"; done
$S "$APP"
codesign --verify --deep --strict "$APP"
if [ -n "$NOTARY_PROFILE" ] || [ -n "$NOTARY_KEY" ]; then
  ditto -c -k --keepParent "$APP" target/bundle/notarize.zip
  if [ -n "$NOTARY_KEY" ]; then
    echo "$NOTARY_KEY" | base64 --decode > target/bundle/notary.p8
    xcrun notarytool submit target/bundle/notarize.zip --key target/bundle/notary.p8 --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" --wait
    rm -f target/bundle/notary.p8
  else
    xcrun notarytool submit target/bundle/notarize.zip --keychain-profile "$NOTARY_PROFILE" --wait
  fi
  xcrun stapler staple "$APP"
fi
