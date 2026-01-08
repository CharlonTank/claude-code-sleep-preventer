# Claude Code Guidelines

## Code Quality

- NEVER use `_variable` patterns to silence unused variable warnings. This indicates bad design or legacy code. If a variable is unused, remove the logic that produces it entirely. We NEVER want legacy code.

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
   Note: whisper-cli must be compiled from https://github.com/ggerganov/whisper.cpp
4. Sign (all Resources binaries first, then app):
   ```bash
   codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/whisper-cli
   codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app/Contents/Resources/globe-listener
   codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app
   ```
5. Create DMG with Applications symlink:
   ```bash
   rm -rf /tmp/dmg-contents
   mkdir -p /tmp/dmg-contents
   cp -R target/release/bundle/ClaudeSleepPreventer.app /tmp/dmg-contents/
   ln -s /Applications /tmp/dmg-contents/Applications
   hdiutil create -volname "Claude Sleep Preventer" -srcfolder /tmp/dmg-contents -ov -format UDZO ClaudeSleepPreventer-X.X.X.dmg
   ```
6. Notarize: `xcrun notarytool submit ClaudeSleepPreventer-X.X.X.dmg --keychain-profile "notary" --wait`
7. Staple: `xcrun stapler staple ClaudeSleepPreventer-X.X.X.dmg`
8. Create GitHub release: `gh release create vX.X.X ClaudeSleepPreventer-X.X.X.dmg --title "vX.X.X" --notes "..."`

**IMPORTANT**: The keychain profile is `"notary"` (NOT "notarytool").

**IMPORTANT**: Update the version number in README.md download links when releasing a new version.
