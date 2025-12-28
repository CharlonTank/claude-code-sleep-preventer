# Claude Code Sleep Preventer

Keep your Mac awake while Claude Code is working, even with the lid closed.

## Features

- Prevents sleep while Claude Code is running
- Works with lid closed (on AC or battery)
- Supports multiple Claude Code instances
- Automatic thermal protection (forces sleep if Mac overheats)
- Re-enables normal sleep when Claude finishes

## Requirements

- macOS
- [Claude Code](https://claude.ai/claude-code) CLI

## Installation

```bash
git clone https://github.com/YOUR_USERNAME/claude-code-sleep-preventer.git
cd claude-code-sleep-preventer
./install.sh
```

The installer will:
1. Copy hook scripts to `~/.claude/hooks/`
2. Set up passwordless sudo for `pmset` (required for sleep control)
3. Configure Claude Code hooks in `~/.claude/settings.json`
4. Set default sleep timeout to 5 minutes

**Restart Claude Code after installation.**

## Uninstallation

```bash
./uninstall.sh
```

## How It Works

| Event | Action |
|-------|--------|
| Claude starts working | Mac stays awake |
| Claude stops | Normal sleep resumes |
| Mac overheats | Forces sleep for protection |
| Multiple Claude instances | Stays awake until ALL stop |

### Sleep Behavior

| Condition | Lid Open | Lid Closed |
|-----------|----------|------------|
| Claude idle | Sleep after 5 min | Sleep immediately |
| Claude working | Stay awake | Stay awake |
| Thermal warning | Force sleep | Force sleep |

## Manual Installation

If you prefer to install manually:

1. Copy scripts to `~/.claude/hooks/`:
   ```bash
   mkdir -p ~/.claude/hooks
   cp hooks/*.sh ~/.claude/hooks/
   chmod +x ~/.claude/hooks/*.sh
   ```

2. Set up passwordless pmset:
   ```bash
   echo "$(whoami) ALL=(ALL) NOPASSWD: /usr/bin/pmset" | sudo tee /etc/sudoers.d/claude-pmset
   sudo chmod 440 /etc/sudoers.d/claude-pmset
   ```

3. Add hooks to `~/.claude/settings.json`:
   ```json
   {
     "hooks": {
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
     }
   }
   ```

## Troubleshooting

### Check current state
```bash
# See if sleep is disabled
pmset -g | grep SleepDisabled

# See active Claude count
cat /tmp/claude_active_count

# Check thermal state
pmset -g therm
```

### Reset everything
```bash
sudo pmset -a disablesleep 0
rm -f /tmp/claude_active_count /tmp/claude_caffeinate.pid /tmp/thermal_monitor.pid
```

## License

MIT
