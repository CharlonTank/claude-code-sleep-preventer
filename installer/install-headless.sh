#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOKS_DIR="$HOME/.claude/hooks"
SETTINGS_FILE="$HOME/.claude/settings.json"

# Create hooks directory
mkdir -p "$HOOKS_DIR"

# Copy hook scripts
cp "$SCRIPT_DIR/prevent-sleep.sh" "$HOOKS_DIR/"
cp "$SCRIPT_DIR/allow-sleep.sh" "$HOOKS_DIR/"
cp "$SCRIPT_DIR/thermal-monitor.sh" "$HOOKS_DIR/"

# Make executable
chmod +x "$HOOKS_DIR/prevent-sleep.sh"
chmod +x "$HOOKS_DIR/allow-sleep.sh"
chmod +x "$HOOKS_DIR/thermal-monitor.sh"

# Set up passwordless sudo for pmset
SUDOERS_FILE="/etc/sudoers.d/claude-pmset"
if [ ! -f "$SUDOERS_FILE" ]; then
    echo "$(whoami) ALL=(ALL) NOPASSWD: /usr/bin/pmset" > "$SUDOERS_FILE"
    chmod 440 "$SUDOERS_FILE"
fi

# Configure Claude Code hooks in settings.json
HOOKS_CONFIG='{
  "UserPromptSubmit": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.claude/hooks/prevent-sleep.sh"
        }
      ]
    }
  ],
  "PreToolUse": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.claude/hooks/prevent-sleep.sh"
        }
      ]
    }
  ],
  "PreCompact": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.claude/hooks/prevent-sleep.sh"
        }
      ]
    }
  ],
  "Stop": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.claude/hooks/allow-sleep.sh"
        }
      ]
    }
  ]
}'

if [ -f "$SETTINGS_FILE" ]; then
    if command -v jq &> /dev/null; then
        TMP_FILE=$(mktemp)
        jq --argjson hooks "$HOOKS_CONFIG" '.hooks = $hooks' "$SETTINGS_FILE" > "$TMP_FILE"
        mv "$TMP_FILE" "$SETTINGS_FILE"
    fi
else
    mkdir -p "$(dirname "$SETTINGS_FILE")"
    echo '{"hooks": '"$HOOKS_CONFIG"'}' > "$SETTINGS_FILE"
fi

# Set default sleep timeout
pmset -a sleep 5
pmset -a disablesleep 0

# Install SwiftBar plugin if SwiftBar exists
if [ -d "/Applications/SwiftBar.app" ] || command -v swiftbar &> /dev/null; then
    SWIFTBAR_DIR="$HOME/Library/Application Support/SwiftBar/Plugins"
    mkdir -p "$SWIFTBAR_DIR"
    cp "$SCRIPT_DIR/claude-sleep-status.1s.sh" "$SWIFTBAR_DIR/"
    chmod +x "$SWIFTBAR_DIR/claude-sleep-status.1s.sh"
fi

exit 0
