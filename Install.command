#!/bin/bash
set -e

echo "======================================"
echo "  Agents Sleep Preventer Installer"
echo "======================================"
echo ""

# Copy app to Applications
echo "→ Copying app to /Applications..."
cp -R "$(dirname "$0")/AgentsSleepPreventer.app" /Applications/

# Copy CLI to /usr/local/bin
echo "→ Installing CLI tool..."
sudo mkdir -p /usr/local/bin
sudo cp "/Applications/AgentsSleepPreventer.app/Contents/MacOS/asp" /usr/local/bin/asp
sudo cp "/Applications/AgentsSleepPreventer.app/Contents/MacOS/asp" /usr/local/bin/agents-sleep-preventer
sudo chmod +x /usr/local/bin/asp /usr/local/bin/agents-sleep-preventer

# Run install
echo "→ Configuring Claude Code hooks..."
/usr/local/bin/asp install

# Open the app
echo "→ Launching app..."
open /Applications/AgentsSleepPreventer.app

echo ""
echo "✅ Installation complete!"
echo ""
echo "Restart Claude Code to activate sleep prevention."
echo ""
read -p "Press Enter to close..."
