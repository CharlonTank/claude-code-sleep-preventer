# Claude Code Guidelines

## Code Quality

- NEVER use `_variable` patterns to silence unused variable warnings. This indicates bad design or legacy code. If a variable is unused, remove the logic that produces it entirely. We NEVER want legacy code.

## Release Process

To publish a new version:
1. Bump version in `Cargo.toml`
2. `cargo build --release`
3. Create app bundle in `target/release/bundle/ClaudeSleepPreventer.app`
4. Sign: `codesign --force --options runtime --sign "Developer ID Application" target/release/bundle/ClaudeSleepPreventer.app`
5. Create DMG: `hdiutil create -volname "Claude Sleep Preventer" -srcfolder target/release/bundle/ClaudeSleepPreventer.app -ov -format UDZO ClaudeSleepPreventer-X.X.X.dmg`
6. Notarize: `xcrun notarytool submit ClaudeSleepPreventer-X.X.X.dmg --keychain-profile "notary" --wait`
7. Staple: `xcrun stapler staple ClaudeSleepPreventer-X.X.X.dmg`
8. Create GitHub release: `gh release create vX.X.X ClaudeSleepPreventer-X.X.X.dmg --title "vX.X.X" --notes "..."`

**IMPORTANT**: The keychain profile is `"notary"` (NOT "notarytool").
