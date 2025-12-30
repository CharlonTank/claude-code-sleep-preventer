#!/bin/bash
set -e

echo "Installing Claude Sleep Preventer..."

# Copy app to Applications
cp -r "$(dirname "$0")/ClaudeSleepPreventer.app" /Applications/

# Copy CLI to /usr/local/bin
sudo cp "/Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer-bin" /usr/local/bin/claude-sleep-preventer

# Run install
/usr/local/bin/claude-sleep-preventer install

# Open the app
open /Applications/ClaudeSleepPreventer.app

echo ""
echo "Installation complete! Restart Claude Code to activate."
