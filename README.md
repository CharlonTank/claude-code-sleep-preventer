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

Uses **multiple hooks** to track all Claude activity:

| Hook | When It Fires |
|------|---------------|
| `UserPromptSubmit` | User sends a prompt |
| `PreToolUse` | Before each tool (Read, Write, Bash, etc.) |
| `PreCompact` | Before context compacting |
| `Stop` | Claude finishes responding |

Each hook refreshes a PID file timestamp. SwiftBar monitors these files and only cleans up if:
- Process doesn't exist, OR
- PID file is >10 seconds old AND CPU < 1% (truly idle)

| Event | Action |
|-------|--------|
| Any hook fires | Creates/refreshes PID file, `disablesleep 1` |
| Claude stops normally | Removes PID file, `disablesleep 0` |
| User interrupts Claude | SwiftBar detects idle after 10s, cleans up |
| Multiple instances | Stays awake until ALL stop |
| Mac overheats | Force `disablesleep 0` (via HUD) |

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
     }
   }
   ```

## Menu Bar HUD

The installer can optionally set up a menu bar indicator using [SwiftBar](https://github.com/swiftbar/SwiftBar):

| Icon | Meaning |
|------|---------|
| ☕ 1 | 1 Claude instance working, sleep disabled |
| ☕ 2 | 2 Claude instances working, sleep disabled |
| 💤 3 | 3 Claude instances open but idle, sleep enabled |
| 😴 | No Claude instances, sleep enabled |

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

# See active Claude PIDs
ls -la /tmp/claude_working_pids/

# Check thermal state
pmset -g therm
```

### Reset everything
```bash
sudo pmset -a disablesleep 0
rm -rf /tmp/claude_working_pids
```

## License

MIT
