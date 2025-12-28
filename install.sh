#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOKS_DIR="$HOME/.claude/hooks"
SETTINGS_FILE="$HOME/.claude/settings.json"

echo "Installing Claude Code Sleep Preventer..."

# Create hooks directory
mkdir -p "$HOOKS_DIR"

# Copy hook scripts
cp "$SCRIPT_DIR/hooks/prevent-sleep.sh" "$HOOKS_DIR/"
cp "$SCRIPT_DIR/hooks/allow-sleep.sh" "$HOOKS_DIR/"
cp "$SCRIPT_DIR/hooks/thermal-monitor.sh" "$HOOKS_DIR/"

# Make executable
chmod +x "$HOOKS_DIR/prevent-sleep.sh"
chmod +x "$HOOKS_DIR/allow-sleep.sh"
chmod +x "$HOOKS_DIR/thermal-monitor.sh"

echo "Hook scripts installed."

# Set up passwordless sudo for pmset
SUDOERS_FILE="/etc/sudoers.d/claude-pmset"
if [ ! -f "$SUDOERS_FILE" ]; then
    echo "Setting up passwordless sudo for pmset..."
    echo "$(whoami) ALL=(ALL) NOPASSWD: /usr/bin/pmset" | sudo tee "$SUDOERS_FILE" > /dev/null
    sudo chmod 440 "$SUDOERS_FILE"
    echo "Passwordless pmset configured."
else
    echo "Passwordless pmset already configured."
fi

# Configure Claude Code hooks in settings.json
echo "Configuring Claude Code hooks..."

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
    # Check if jq is available
    if command -v jq &> /dev/null; then
        # Merge hooks into existing settings
        TMP_FILE=$(mktemp)
        jq --argjson hooks "$HOOKS_CONFIG" '.hooks = $hooks' "$SETTINGS_FILE" > "$TMP_FILE"
        mv "$TMP_FILE" "$SETTINGS_FILE"
        echo "Hooks added to existing settings.json"
    else
        echo ""
        echo "WARNING: jq not installed. Please manually add hooks to $SETTINGS_FILE"
        echo ""
        echo "Add this to your settings.json:"
        echo '  "hooks": '"$HOOKS_CONFIG"
        echo ""
        echo "Install jq with: brew install jq"
    fi
else
    # Create new settings file
    mkdir -p "$(dirname "$SETTINGS_FILE")"
    echo '{"hooks": '"$HOOKS_CONFIG"'}' | jq '.' > "$SETTINGS_FILE" 2>/dev/null || \
    echo '{"hooks": '"$HOOKS_CONFIG"'}' > "$SETTINGS_FILE"
    echo "Created new settings.json with hooks."
fi

# Set default sleep timeout to 5 minutes
echo "Setting default sleep timeout to 5 minutes..."
sudo pmset -a sleep 5
sudo pmset -a disablesleep 0

echo ""
echo "Installation complete!"
echo ""
echo "Restart Claude Code to activate the hooks."
echo ""
echo "How it works:"
echo "  - When Claude starts working: Mac stays awake (even with lid closed)"
echo "  - When Claude stops: Normal sleep behavior resumes"
echo "  - If Mac overheats: Forces sleep for protection"
echo "  - Multiple Claude instances: Stays awake until ALL stop"
