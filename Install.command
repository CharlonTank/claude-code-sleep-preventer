#!/bin/bash
set -e

echo "======================================"
echo "  Claude Sleep Preventer Installer"
echo "======================================"
echo ""

# Copy app to Applications
echo "→ Copying app to /Applications..."
cp -R "$(dirname "$0")/ClaudeSleepPreventer.app" /Applications/

# Copy CLI to /usr/local/bin
echo "→ Installing CLI tool..."
sudo mkdir -p /usr/local/bin
sudo cp "/Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer" /usr/local/bin/claude-sleep-preventer
sudo chmod +x /usr/local/bin/claude-sleep-preventer

# Run install
echo "→ Configuring Claude Code hooks..."
/usr/local/bin/claude-sleep-preventer install

# Open the app
echo "→ Launching app..."
open /Applications/ClaudeSleepPreventer.app

echo ""
echo "✅ Installation complete!"
echo ""
echo "Restart Claude Code to activate sleep prevention."
echo ""
read -p "Press Enter to close..."
