#!/bin/bash
# Clean all Claude Sleep Preventer data from Mac
# Usage: ./clean-ccsp.sh

set -e

echo "=== Claude Sleep Preventer Cleanup ==="
echo

# Kill running processes
echo "Killing running processes..."
pkill -f "claude-sleep-preventer" 2>/dev/null || true
pkill -f "ClaudeSleepPreventer" 2>/dev/null || true

# Remove app from Applications
echo "Removing app..."
rm -rf /Applications/ClaudeSleepPreventer.app 2>/dev/null || true

# Remove app data
echo "Removing app data..."
rm -rf ~/Library/Application\ Support/ClaudeSleepPreventer 2>/dev/null || true
rm -rf ~/.local/share/ClaudeSleepPreventer 2>/dev/null || true

# Remove logs
echo "Removing logs..."
rm -rf ~/Library/Logs/ClaudeSleepPreventer 2>/dev/null || true

# Remove caches
echo "Removing caches..."
rm -rf ~/Library/Caches/ClaudeSleepPreventer 2>/dev/null || true

# Remove preferences
echo "Removing preferences..."
rm -f ~/Library/Preferences/com.charlontank.claude-sleep-preventer.plist 2>/dev/null || true

# Remove LaunchAgents
echo "Removing LaunchAgents..."
launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.charlontank.claude-sleep-preventer.plist 2>/dev/null || true
rm -f ~/Library/LaunchAgents/com.charlontank.claude-sleep-preventer.plist 2>/dev/null || true

# Remove Claude Code hooks
echo "Removing Claude Code hooks..."
rm -rf ~/.claude/hooks 2>/dev/null || true

# Remove hooks from settings.json
if [ -f ~/.claude/settings.json ]; then
    echo "Cleaning settings.json..."
    # Remove hooks key from settings.json using python (available on all Macs)
    python3 -c "
import json
import sys
try:
    with open('$HOME/.claude/settings.json', 'r') as f:
        data = json.load(f)
    if 'hooks' in data:
        del data['hooks']
        with open('$HOME/.claude/settings.json', 'w') as f:
            json.dump(data, f, indent=2)
        print('  Removed hooks from settings.json')
except Exception as e:
    print(f'  Warning: Could not clean settings.json: {e}')
" 2>/dev/null || true
fi

# Remove sudoers config
echo "Removing sudoers config..."
sudo rm -f /etc/sudoers.d/claude-pmset 2>/dev/null || true

# Reset TCC permissions
echo "Resetting TCC permissions..."
tccutil reset Microphone com.charlontank.claude-sleep-preventer 2>/dev/null || true
tccutil reset Accessibility com.charlontank.claude-sleep-preventer 2>/dev/null || true
tccutil reset ListenEvent com.charlontank.claude-sleep-preventer 2>/dev/null || true

# Re-enable sleep
echo "Re-enabling sleep..."
sudo pmset -a disablesleep 0 2>/dev/null || true

# Unmount any DMG
echo "Unmounting DMG..."
hdiutil detach /Volumes/Claude\ Sleep\ Preventer 2>/dev/null || true

echo
echo "=== Cleanup complete! ==="
echo "You can now install a fresh version."
