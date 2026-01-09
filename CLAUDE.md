# Claude Code Guidelines

## Meta

- **Keep this file updated**: When you add new scripts, processes, or important patterns, update this CLAUDE.md file so future sessions have accurate context.

## Code Quality

- NEVER use `_variable` patterns to silence unused variable warnings. This indicates bad design or legacy code. If a variable is unused, remove the logic that produces it entirely. We NEVER want legacy code.

## Testing / Clean Install

Before testing a new build, run the cleanup script to ensure a fresh state:

```bash
./scripts/clean-ccsp.sh
```

This removes:
- App from /Applications
- App data, logs, caches
- LaunchAgents
- Claude Code hooks
- Sudoers config
- TCC permissions (Input Monitoring, Microphone, Accessibility)

## Release Process

To publish a new version:

1. Bump version in `Cargo.toml` and `Info.plist`
2. `cargo build --release`
3. Create app bundle structure:
   ```bash
   rm -rf target/release/bundle
   mkdir -p target/release/bundle/ClaudeSleepPreventer.app/Contents/{MacOS,Resources}
   cp target/release/claude-sleep-preventer target/release/bundle/ClaudeSleepPreventer.app/Contents/MacOS/
   cp Info.plist target/release/bundle/ClaudeSleepPreventer.app/Contents/
   cp AppIcon.icns target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/
   cp /tmp/whisper.cpp/build/bin/whisper-cli target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/
   swiftc swift/globe-listener.swift -O -o target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/globe-listener
   ```
   Note: whisper-cli must be compiled statically from https://github.com/ggerganov/whisper.cpp:
   ```bash
   cd /tmp && git clone https://github.com/ggerganov/whisper.cpp && cd whisper.cpp
   mkdir build && cd build
   cmake .. -DBUILD_SHARED_LIBS=OFF -DGGML_METAL=ON -DCMAKE_BUILD_TYPE=Release
   make -j8 whisper-cli
   # Binary is at /tmp/whisper.cpp/build/bin/whisper-cli
   ```
4. Sign (all Resources binaries first, then app):
   ```bash
   codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/whisper-cli
   codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/globe-listener
   codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app
   ```
5. Create DMG with Applications symlink:
   ```bash
   rm -rf /tmp/dmg-staging
   mkdir -p /tmp/dmg-staging
   cp -R target/release/bundle/ClaudeSleepPreventer.app /tmp/dmg-staging/
   ln -s /Applications /tmp/dmg-staging/Applications
   hdiutil create -volname "Claude Sleep Preventer" -srcfolder /tmp/dmg-staging -ov -format UDZO ClaudeSleepPreventer-X.X.X.dmg
   ```
6. Notarize: `xcrun notarytool submit ClaudeSleepPreventer-X.X.X.dmg --keychain-profile "notary" --wait`
7. Staple: `xcrun stapler staple ClaudeSleepPreventer-X.X.X.dmg`
8. Update release: `gh release upload vX.X.X ClaudeSleepPreventer-X.X.X.dmg --clobber`
9. Commit and push changes

**IMPORTANT**: The keychain profile is `"notary"` (NOT "notarytool").

**IMPORTANT**: Update the version number in README.md download links when releasing a new version.

## macOS Permissions Notes

- **Microphone**: App must call `AVCaptureDevice.requestAccessForMediaType:` to appear in System Preferences list. The system dialog triggers automatically.
- **Accessibility**: Check with `AXIsProcessTrusted()`. Open preferences with `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`
- **Input Monitoring**: Cannot be checked programmatically. Open preferences with `x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent`

## AppleScript Gotchas

- `--` in AppleScript starts a comment. Use short flags like `-y` instead of `--yes` when running commands via AppleScript.
- Use `osascript -e "..."` via `Command::new()` instead of `NSAppleScript` - it's more reliable.
