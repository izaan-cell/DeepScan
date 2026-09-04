#!/bin/bash
# Builds DeepScan.app and wraps it in a .dmg. Run from the repo root:
#   packaging/macos/build-dmg.sh
# Produces dist/DeepScan-<version>.dmg. Requires cargo + go (java-parser's
# fat jar is bundled if java-parser/target/deepscan-parser.jar already
# exists — run `mvn package` first if you want document/PDF search in the
# packaged app; the app still runs without it).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION="$(cat "$REPO_ROOT/VERSION")"
BUILD_DIR="$REPO_ROOT/dist/macos"
APP="$BUILD_DIR/DeepScan.app"

echo "==> Building rust-engine (release)"
(cd "$REPO_ROOT/rust-engine" && cargo build --release)

echo "==> Building go-daemon"
(cd "$REPO_ROOT/go-daemon" && go build -o "$REPO_ROOT/go-daemon/deepscan-daemon" .)

echo "==> Assembling DeepScan.app v$VERSION"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin" "$APP/Contents/Resources/lib"

cp "$REPO_ROOT/rust-engine/target/release/deepscan-engine" "$APP/Contents/Resources/bin/"
cp "$REPO_ROOT/go-daemon/deepscan-daemon" "$APP/Contents/Resources/bin/"
cp -R "$REPO_ROOT/frontend" "$APP/Contents/Resources/frontend"
cp "$REPO_ROOT/packaging/macos/launcher.sh" "$APP/Contents/MacOS/DeepScan"
chmod +x "$APP/Contents/MacOS/DeepScan"

sed "s/__VERSION__/$VERSION/g" "$REPO_ROOT/packaging/macos/Info.plist.template" > "$APP/Contents/Info.plist"

if [ -f "$REPO_ROOT/java-parser/target/deepscan-parser.jar" ]; then
  echo "==> Bundling java-parser (Tika document extraction)"
  cp "$REPO_ROOT/java-parser/target/deepscan-parser.jar" "$APP/Contents/Resources/lib/"
  if command -v jlink >/dev/null 2>&1; then
    jlink --add-modules java.base,java.naming,java.desktop,java.sql \
      --output "$APP/Contents/Resources/jre" --strip-debug --no-header-files --no-man-pages
  fi
else
  echo "==> Skipping java-parser (no target/deepscan-parser.jar — run 'mvn package' in java-parser/ first for document search)"
fi

echo "==> Creating .dmg"
mkdir -p "$BUILD_DIR/dmg-root"
rm -f "$BUILD_DIR/dmg-root"/*.app 2>/dev/null || true
cp -R "$APP" "$BUILD_DIR/dmg-root/"
ln -sf /Applications "$BUILD_DIR/dmg-root/Applications"

mkdir -p "$REPO_ROOT/dist"
hdiutil create -volname "DeepScan $VERSION" \
  -srcfolder "$BUILD_DIR/dmg-root" \
  -ov -format UDZO \
  "$REPO_ROOT/dist/DeepScan.dmg"

echo "==> Done: dist/DeepScan.dmg"
