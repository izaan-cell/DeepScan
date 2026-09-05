#!/bin/bash
# Builds DeepScan.app and wraps it in a .dmg with the classic drag-to-
# Applications Finder layout (simple black arrow on a white background —
# packaging/macos/assets/dmg-background.png). Run from the repo root:
#   packaging/macos/build-dmg.sh
# Produces dist/DeepScan.dmg. Requires cargo + go (java-parser's fat jar is
# bundled if java-parser/target/deepscan-parser.jar already exists — run
# `mvn package` first if you want document/PDF search in the packaged app;
# the app still runs without it).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION="$(cat "$REPO_ROOT/VERSION")"
BUILD_DIR="$REPO_ROOT/dist/macos"
APP="$BUILD_DIR/DeepScan.app"
VOL_NAME="DeepScan"

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
cp "$REPO_ROOT/packaging/macos/assets/DeepScan.icns" "$APP/Contents/Resources/DeepScan.icns"

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

echo "==> Ad-hoc signing (free — no Developer ID, doesn't remove the Gatekeeper"
echo "    'unidentified developer' warning, but avoids a separate, worse"
echo "    'app is damaged and can't be opened' error some unsigned Apple"
echo "    Silicon builds hit)"
codesign --force --deep --sign - "$APP"

echo "==> Assembling dmg staging root"
STAGING="$BUILD_DIR/dmg-root"
rm -rf "$STAGING"
mkdir -p "$STAGING/.background"
cp -R "$APP" "$STAGING/"
ln -sf /Applications "$STAGING/Applications"
cp "$REPO_ROOT/packaging/macos/assets/dmg-background.png" "$STAGING/.background/background.png"

mkdir -p "$REPO_ROOT/dist"
RW_DMG="$BUILD_DIR/DeepScan-rw.dmg"
rm -f "$RW_DMG"

echo "==> Creating writable dmg to lay out the Finder window"
hdiutil create -volname "$VOL_NAME" -srcfolder "$STAGING" -ov -format UDRW -size 300m "$RW_DMG"

MOUNT_DIR="/Volumes/$VOL_NAME"
hdiutil attach "$RW_DMG" -mountpoint "$MOUNT_DIR" -nobrowse -noautoopen

# Classic drag-to-Applications layout: app icon on the left, Applications
# shortcut on the right, the arrow background pointing between them.
osascript <<EOF
tell application "Finder"
  tell disk "$VOL_NAME"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {200, 120, 800, 520}
    set viewOptions to the icon view options of container window
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to 96
    set background picture of viewOptions to file ".background:background.png"
    set position of item "DeepScan.app" of container window to {150, 190}
    set position of item "Applications" of container window to {450, 190}
    close
    open
    update without registering applications
    delay 1
  end tell
end tell
EOF

hdiutil detach "$MOUNT_DIR"

echo "==> Compressing final .dmg"
rm -f "$REPO_ROOT/dist/DeepScan.dmg"
hdiutil convert "$RW_DMG" -format UDZO -ov -o "$REPO_ROOT/dist/DeepScan.dmg"
rm -f "$RW_DMG"

echo "==> Done: dist/DeepScan.dmg"
