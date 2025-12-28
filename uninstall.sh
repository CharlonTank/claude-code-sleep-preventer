#!/bin/bash

set -e

HOOKS_DIR="$HOME/.claude/hooks"
SETTINGS_FILE="$HOME/.claude/settings.json"
SUDOERS_FILE="/etc/sudoers.d/claude-pmset"

echo "Uninstalling Claude Code Sleep Preventer..."

# Kill any running processes
if [ -f /tmp/claude_caffeinate.pid ]; then
    kill $(cat /tmp/claude_caffeinate.pid) 2>/dev/null || true
    rm -f /tmp/claude_caffeinate.pid
fi

if [ -f /tmp/thermal_monitor.pid ]; then
    kill $(cat /tmp/thermal_monitor.pid) 2>/dev/null || true
    rm -f /tmp/thermal_monitor.pid
fi

# Re-enable sleep
sudo pmset -a disablesleep 0

# Remove hook scripts
rm -f "$HOOKS_DIR/prevent-sleep.sh"
rm -f "$HOOKS_DIR/allow-sleep.sh"
rm -f "$HOOKS_DIR/thermal-monitor.sh"
echo "Hook scripts removed."

# Remove sudoers file
if [ -f "$SUDOERS_FILE" ]; then
    sudo rm -f "$SUDOERS_FILE"
    echo "Passwordless pmset removed."
fi

# Clean up temp files
rm -f /tmp/claude_active_count
rm -f /tmp/claude_sleep.lock
rmdir /tmp/claude_sleep.lock 2>/dev/null || true

# Remove hooks from settings.json
if [ -f "$SETTINGS_FILE" ] && command -v jq &> /dev/null; then
    TMP_FILE=$(mktemp)
    jq 'del(.hooks)' "$SETTINGS_FILE" > "$TMP_FILE"
    mv "$TMP_FILE" "$SETTINGS_FILE"
    echo "Hooks removed from settings.json"
else
    echo "Please manually remove 'hooks' from $SETTINGS_FILE"
fi

echo ""
echo "Uninstall complete!"
echo "Restart Claude Code to apply changes."
