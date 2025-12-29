# Claude Code Sleep Preventer

Keep your Mac awake while Claude Code is working, even with the lid closed.

<p align="center">
  <img src="https://img.shields.io/badge/Rust-🦀-orange" alt="Rust">
  <img src="https://img.shields.io/badge/☕_2-Claude_Active-green" alt="Active">
  <img src="https://img.shields.io/badge/😴-Sleep_Enabled-gray" alt="Sleeping">
</p>

## Features

- Single Rust binary - no dependencies
- Prevents sleep while Claude Code is working
- Works with lid closed (on AC or battery)
- Supports multiple Claude Code instances
- Automatic cleanup of interrupted sessions
- Re-enables normal sleep when Claude finishes

## Installation

### Option 1: Homebrew (recommended)

```bash
brew tap CharlonTank/tap
brew install claude-sleep-preventer
claude-sleep-preventer install
```

### Option 2: Download Binary

```bash
curl -L https://github.com/CharlonTank/claude-code-sleep-preventer/releases/latest/download/claude-sleep-preventer -o /usr/local/bin/claude-sleep-preventer
chmod +x /usr/local/bin/claude-sleep-preventer
claude-sleep-preventer install
```

### Option 3: Build from Source

```bash
git clone https://github.com/CharlonTank/claude-code-sleep-preventer.git
cd claude-code-sleep-preventer
cargo build --release
./target/release/claude-sleep-preventer install
```

**Restart Claude Code after installation.**

## Usage

```bash
# Check status
claude-sleep-preventer status

# Clean up stale PIDs (interrupted sessions)
claude-sleep-preventer cleanup

# Run cleanup daemon (optional, runs every second)
claude-sleep-preventer daemon

# Uninstall
claude-sleep-preventer uninstall
```

## How It Works

Uses Claude Code hooks to track activity:

| Hook | When It Fires |
|------|---------------|
| `UserPromptSubmit` | User sends a prompt |
| `PreToolUse` | Before each tool (Read, Write, Bash, etc.) |
| `PreCompact` | Before context compacting |
| `Stop` | Claude finishes responding |

Each hook calls `claude-sleep-preventer start` which:
1. Creates a PID file in `/tmp/claude_working_pids/`
2. Disables sleep via `pmset -a disablesleep 1`

When Claude stops, `claude-sleep-preventer stop`:
1. Removes the PID file
2. Re-enables sleep if no other instances are working

### Interrupt Detection

If you interrupt Claude (Ctrl+C), the Stop hook doesn't fire. Run `cleanup` or `daemon` to detect idle processes (CPU < 1% for >10 seconds) and clean up.

## Commands

| Command | Description |
|---------|-------------|
| `start` | Register process, disable sleep |
| `stop` | Unregister process, enable sleep |
| `status` | Show current state |
| `cleanup` | Clean up stale PIDs |
| `daemon` | Run cleanup every second |
| `install` | Install hooks and configure |
| `uninstall` | Remove hooks and restore defaults |

## Troubleshooting

```bash
# Check status
claude-sleep-preventer status

# Manual cleanup
claude-sleep-preventer cleanup

# Reset everything
claude-sleep-preventer uninstall
sudo pmset -a disablesleep 0
```

## License

MIT
