# Claude Code Guidelines

## Meta

- **Keep this file updated**: When you add new scripts, processes, or important patterns, update this AGENTS.md file so future sessions have accurate context.

## Code Quality

- NEVER use `_variable` patterns to silence unused variable warnings. This indicates bad design or legacy code. If a variable is unused, remove the logic that produces it entirely. We NEVER want legacy code.

## Testing / Clean Install

Before testing a new build, run the Rust cleanup task to ensure a fresh state:

```bash
cargo xtask clean
```

Optional: keep Whisper models (~500 MB) and whisper-cli:

```bash
cargo xtask clean --keep-model
```

This removes:
- App from /Applications
- App data, logs, caches (optionally keeping models)
- LaunchAgents
- Claude Code hooks
- Sudoers config
- TCC permissions (Input Monitoring, Microphone, Accessibility)
- Whisper CLI + models (Homebrew paths and /tmp build), unless `--keep-model`

## Dev scripts (Rust only)

- `cargo xtask complete-test --skip-notarize` (clean system, build DMG, open it)
- `cargo xtask complete-test --skip-notarize --keep-model` (same but keeps models + whisper-cli)
- `cargo xtask build-dmg --skip-notarize` (local DMG build only)
- `cargo xtask replace-app --open` (rebuild + replace /Applications app)
- `cargo xtask release X.Y.Z --upload` (bump, build DMG, notarize, upload)

## Uninstall

- `claude-sleep-preventer uninstall` removes app data by default; use `-k`/`--keep-model` to preserve Whisper models (~500 MB).

## Release Process

To publish a new version, prefer:

1. `cargo xtask release X.Y.Z` (bumps `Cargo.toml`, `Info.plist`, `README.md`, builds signed DMG, notarizes)
2. Add `--upload` to also push the DMG to GitHub release
3. Commit and push changes

**IMPORTANT**: The keychain profile is `"notary"` (NOT "notarytool").

**IMPORTANT**: Update the version number in README.md download links when releasing a new version.

## macOS Permissions Notes

- **Microphone**: App must call `AVCaptureDevice.requestAccessForMediaType:` to appear in System Preferences list. The system dialog triggers automatically.
- **Accessibility**: Check with `AXIsProcessTrusted()`. Open preferences with `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`
- **Input Monitoring**: Cannot be checked programmatically. Open preferences with `x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent`

## AppleScript Gotchas

- `--` in AppleScript starts a comment. Use short flags like `-y` instead of `--yes` when running commands via AppleScript.
- Use `osascript -e "..."` via `Command::new()` instead of `NSAppleScript` - it's more reliable.
