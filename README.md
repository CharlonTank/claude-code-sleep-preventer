<div align="center">

# ☕ Claude Code Sleep Preventer

### Keep your Mac awake while Claude Code is working
**Close your laptop lid. Walk away. Come back to finished work.**

<br>

[![Download DMG](https://img.shields.io/badge/Download-DMG%20Installer-blue?style=for-the-badge&logo=apple)](https://github.com/CharlonTank/claude-code-sleep-preventer/releases/latest/download/ClaudeSleepPreventer-2.2.6.dmg)

<br>

![macOS](https://img.shields.io/badge/macOS-000000?style=flat&logo=apple&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub release](https://img.shields.io/github/v/release/CharlonTank/claude-code-sleep-preventer)](https://github.com/CharlonTank/claude-code-sleep-preventer/releases)

</div>

---

## The Problem

You ask Claude to refactor your codebase. It's going to take 10 minutes. You close your MacBook lid to grab coffee...

**💀 Mac sleeps. Claude stops. Work lost.**

## The Solution

Install this tool. Now your Mac stays awake while Claude works, even with the lid closed. When Claude finishes, normal sleep resumes.

<div align="center">

| Before | After |
|--------|-------|
| 😴 Lid closed = Mac sleeps | ☕ Lid closed = Claude keeps working |
| 🔄 Come back to interrupted work | ✅ Come back to finished work |

</div>

---

## Installation

### 🍎 Download DMG (Easiest)

1. [Download the latest DMG](https://github.com/CharlonTank/claude-code-sleep-preventer/releases/latest/download/ClaudeSleepPreventer-2.2.6.dmg)
2. Drag `ClaudeSleepPreventer.app` to Applications
3. Launch the app - it will auto-configure on first run
4. Restart Claude Code

### 🍺 Homebrew

```bash
brew tap CharlonTank/tap
brew install claude-sleep-preventer
claude-sleep-preventer install
```

### 🦀 Build from Source

```bash
git clone https://github.com/CharlonTank/claude-code-sleep-preventer.git
cd claude-code-sleep-preventer
cargo build --release
./target/release/claude-sleep-preventer install
```

---

## How It Works

```
You send a prompt
       ↓
   Claude starts working → 🔒 Sleep disabled
       ↓
   Claude finishes → 🔓 Sleep re-enabled
```

That's it. No configuration needed.

---

## Commands

```bash
claude-sleep-preventer status     # Check current state
claude-sleep-preventer cleanup    # Clean up after interrupts
claude-sleep-preventer uninstall  # Remove completely
```

---

## FAQ

**Does it drain my battery?**
No more than usual. Your Mac just stays awake instead of sleeping.

**What if I interrupt Claude with Ctrl+C?**
Run `claude-sleep-preventer cleanup` or the tool auto-detects idle sessions after 10 seconds.

**Does it work with multiple Claude instances?**
Yes! Mac stays awake until ALL instances finish.

---

<div align="center">

Made with ☕ for Claude Code users

[Report Issue](https://github.com/CharlonTank/claude-code-sleep-preventer/issues) · [View Releases](https://github.com/CharlonTank/claude-code-sleep-preventer/releases)

</div>
