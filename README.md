# Claude Code Sleep Preventer

Keep your Mac awake while Claude Code is working, even with the lid closed.

<p align="center">
  <img src="https://img.shields.io/badge/☕_2-Claude_Active-green" alt="Active">
  <img src="https://img.shields.io/badge/😴-Sleep_Enabled-gray" alt="Sleeping">
</p>

## Features

- Prevents sleep while Claude Code is running
- Works with lid closed (on AC or battery)
- Supports multiple Claude Code instances
- Automatic thermal protection (forces sleep if Mac overheats)
- Re-enables normal sleep when Claude finishes
- **Menu bar HUD** showing active Claude count

## Requirements

- macOS
- [Claude Code](https://claude.ai/claude-code) CLI

## Installation

```bash
git clone https://github.com/CharlonTank/claude-code-sleep-preventer.git
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

Uses `pmset -a disablesleep` to control Mac sleep state via Claude Code hooks:

- **UserPromptSubmit hook**: Runs `prevent-sleep.sh` → disables sleep
- **Stop hook**: Runs `allow-sleep.sh` → re-enables sleep

A thermal monitor runs in the background and forces sleep if the Mac overheats.

| Event | Action |
|-------|--------|
| Claude starts working | `disablesleep 1` |
| Claude stops | `disablesleep 0` |
| Mac overheats | Force `disablesleep 0` |
| Multiple instances | Stays awake until ALL stop |

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

## Menu Bar HUD

The installer can optionally set up a menu bar indicator using [SwiftBar](https://github.com/swiftbar/SwiftBar):

| Icon | Meaning |
|------|---------|
| ☕ 1 | 1 Claude instance active, sleep disabled |
| ☕ 2 | 2 Claude instances active, sleep disabled |
| 😴 | No Claude active, sleep enabled |

Click the icon for more details and a manual override option.

### Manual HUD Installation

If you skipped HUD during install:

```bash
brew install --cask swiftbar
mkdir -p ~/Library/Application\ Support/SwiftBar/Plugins
cp swiftbar/claude-sleep-status.1s.sh ~/Library/Application\ Support/SwiftBar/Plugins/
```

Launch SwiftBar and point it to the plugins folder.

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
rm -f /tmp/claude_active_count /tmp/thermal_monitor.pid
```

## License

MIT
