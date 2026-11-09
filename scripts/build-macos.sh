#!/usr/bin/env bash
# Build Slicer Router.app for Apple silicon with its Finder thumbnail extension,
# sign it from the inside out, optionally notarize it, and zip it.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/build-macos.sh [--no-sign] [--notarize] [--out DIR]

  --no-sign   Sign ad hoc instead of with APPLE_SIGNING_IDENTITY. For CI.
  --notarize  Notarize with the notarytool keychain profile named by
              NOTARY_PROFILE (default "slicer-router"), then staple.
  --out DIR   Folder for the zip. Defaults to the folder holding the app.

Signing uses APPLE_SIGNING_IDENTITY, for example
"Developer ID Application: Your Name (TEAMID)".
USAGE
}

sign=1
notarize=0
out=
while [ $# -gt 0 ]; do
  case "$1" in
    --no-sign) sign=0 ;;
    --notarize) notarize=1 ;;
    --out) out=${2:?--out needs a folder}; shift ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

if [ "$sign" = 1 ] && [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "error: set APPLE_SIGNING_IDENTITY, or pass --no-sign for an ad hoc build" >&2
  exit 2
fi
if [ "$notarize" = 1 ] && [ "$sign" = 0 ]; then
  echo "error: --notarize needs a Developer ID signature, not --no-sign" >&2
  exit 2
fi

repo=$(cd "$(dirname "$0")/.." && pwd)
target=aarch64-apple-darwin
if [[ "$(uname -m)" != arm64 ]]; then
  echo "error: this script builds $target and must run on an arm64 Mac" >&2
  exit 1
fi
export MACOSX_DEPLOYMENT_TARGET=11.0
extension_src="$repo/app/macos/ThumbnailExtension"
app="$repo/target/$target/release/bundle/macos/Slicer Router.app"
appex="$app/Contents/PlugIns/SlicerRouterThumbnail.appex"

echo "==> Rust static library for the extension"
cargo build --release --locked -p slicer-router-ffi --target "$target"

echo "==> Slicer Router.app"
# This script signs and notarizes the finished bundle, so Tauri must not do it
# first on a bundle that is about to change.
(cd "$repo/app/src-tauri" &&
  env -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE -u APPLE_ID -u APPLE_PASSWORD \
    -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
    cargo tauri build --bundles app --target "$target")
if [[ ! -d "$app" ]]; then
  echo "error: cargo tauri build did not produce $app" >&2
  exit 1
fi
version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")

echo "==> SlicerRouterThumbnail.appex $version"
rm -rf "$appex"
mkdir -p "$appex/Contents/MacOS"
cp "$extension_src/Info.plist" "$appex/Contents/Info.plist"
/usr/libexec/PlistBuddy \
  -c "Add :CFBundleShortVersionString string $version" \
  -c "Add :CFBundleVersion string $version" \
  "$appex/Contents/Info.plist"
swiftc -swift-version 6 -O -target "arm64-apple-macos$MACOSX_DEPLOYMENT_TARGET" \
  -application-extension -parse-as-library -module-name SlicerRouterThumbnail \
  -import-objc-header "$repo/crates/slicer-router-ffi/include/slicer_router.h" \
  -framework QuickLookThumbnailing -framework AppKit -framework ImageIO \
  -L "$repo/target/$target/release" -lslicer_router_ffi -liconv \
  -Xlinker -e -Xlinker _NSExtensionMain -Xlinker -application_extension \
  -Xlinker -dead_strip \
  -o "$appex/Contents/MacOS/SlicerRouterThumbnail" \
  "$extension_src/ThumbnailProvider.swift"
plutil -lint "$appex/Contents/Info.plist" "$app/Contents/Info.plist"

echo "==> Signing"
if [ "$sign" = 1 ]; then
  identity=$APPLE_SIGNING_IDENTITY
  timestamp=--timestamp
else
  identity=-
  timestamp=--timestamp=none
fi
# Inside out: the extension with its sandbox entitlements first, then the app,
# whose signature seals the signed extension. --deep would re-sign the extension
# without its entitlements, and Quick Look only loads sandboxed extensions.
codesign --force --sign "$identity" --options runtime "$timestamp" \
  --entitlements "$extension_src/SlicerRouterThumbnail.entitlements" "$appex"
codesign --force --sign "$identity" --options runtime "$timestamp" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

out=${out:-$(dirname "$app")}
mkdir -p "$out"
zip="$out/SlicerRouter-$version-arm64.zip"

if [ "$notarize" = 1 ]; then
  echo "==> Notarizing"
  rm -f "$zip"
  ditto -c -k --keepParent "$app" "$zip"
  result=$(xcrun notarytool submit "$zip" \
    --keychain-profile "${NOTARY_PROFILE:-slicer-router}" --wait --output-format json)
  echo "$result"
  status=$(plutil -extract status raw -o - - <<<"$result")
  if [ "$status" != Accepted ]; then
    id=$(plutil -extract id raw -o - - <<<"$result")
    xcrun notarytool log "$id" --keychain-profile "${NOTARY_PROFILE:-slicer-router}" >&2 || true
    echo "error: notarization finished as $status" >&2
    exit 1
  fi
  xcrun stapler staple "$app"
  spctl --assess --verbose=2 "$app"
fi

rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
echo "==> $app"
echo "==> $zip"
