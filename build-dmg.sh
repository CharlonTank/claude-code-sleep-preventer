#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/build"
APP_NAME="Claude Sleep Preventer"
DMG_NAME="ClaudeSleepPreventer"
VERSION="1.0.0"

echo "Building $APP_NAME v$VERSION..."

# Clean build directory
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# Compile AppleScript to app
echo "Compiling AppleScript..."
osacompile -o "$BUILD_DIR/$APP_NAME.app" "$SCRIPT_DIR/installer/install.applescript"

# Create Resources/scripts directory in app bundle
SCRIPTS_DIR="$BUILD_DIR/$APP_NAME.app/Contents/Resources/scripts"
mkdir -p "$SCRIPTS_DIR"

# Copy scripts into app bundle
echo "Copying scripts..."
cp "$SCRIPT_DIR/installer/install-headless.sh" "$SCRIPTS_DIR/"
cp "$SCRIPT_DIR/hooks/prevent-sleep.sh" "$SCRIPTS_DIR/"
cp "$SCRIPT_DIR/hooks/allow-sleep.sh" "$SCRIPTS_DIR/"
cp "$SCRIPT_DIR/hooks/thermal-monitor.sh" "$SCRIPTS_DIR/"
cp "$SCRIPT_DIR/swiftbar/claude-sleep-status.1s.sh" "$SCRIPTS_DIR/"

# Make scripts executable
chmod +x "$SCRIPTS_DIR"/*.sh

# Create DMG staging directory
DMG_STAGING="$BUILD_DIR/dmg"
mkdir -p "$DMG_STAGING"
cp -R "$BUILD_DIR/$APP_NAME.app" "$DMG_STAGING/"

# Add README
cat > "$DMG_STAGING/README.txt" << 'EOF'
Claude Code Sleep Preventer
============================

Double-click "Claude Sleep Preventer.app" to install.

After installation, restart Claude Code.

Menu bar shows:
  - Coffee icon = Claude working, sleep disabled
  - Zzz icon = Claude idle, sleep enabled

For SwiftBar HUD: Install SwiftBar first (brew install --cask swiftbar)

More info: https://github.com/CharlonTank/claude-code-sleep-preventer
EOF

# Create DMG
echo "Creating DMG..."
DMG_PATH="$BUILD_DIR/$DMG_NAME-$VERSION.dmg"
hdiutil create -volname "$APP_NAME" -srcfolder "$DMG_STAGING" -ov -format UDZO "$DMG_PATH"

echo ""
echo "Done! DMG created at:"
echo "  $DMG_PATH"
echo ""
echo "To release:"
echo "  gh release create v$VERSION $DMG_PATH --title 'v$VERSION' --notes 'Release v$VERSION'"
