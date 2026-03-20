import Cocoa
import Carbon

struct ActiveInstance {
    let pid: Int
    let ageSecs: Int
    let cpu: Double
    let location: String
}

struct InstanceList {
    let active: [ActiveInstance]
    let inactive: [Int]
    let hooksInstalled: Bool
    let sleepDisabled: Bool

    static let empty = InstanceList(active: [], inactive: [], hooksInstalled: false, sleepDisabled: false)

    var inactiveCount: Int {
        inactive.count
    }
}

struct UpdateRelease {
    let version: String
    let releaseURL: URL
    let downloadURL: URL
}

private enum UpdateCheckMode {
    case automatic
    case manual
}

private struct UpdateCheckError: Error {
    let message: String
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var statusItem: NSStatusItem?
    private var agentProcess: Process?
    private let menu = NSMenu()
    private var isRefreshing = false
    private var isRefreshingStatus = false
    private var isCheckingForUpdates = false
    private var isInstalling = false
    private var isUninstalling = false
    private var availableUpdate: UpdateRelease?
    private var statusRefreshTimer: Timer?
    private var updateCheckTimer: Timer?
    private var hotKeyHandler: EventHandlerRef?
    private var hotKeyActive: EventHotKeyRef?
    private var hotKeyInactive: EventHotKeyRef?
    private let hotKeySignature: OSType = 0x63637370 // 'ccsp'
    private let hotKeyQueue = DispatchQueue(label: "ccsp.hotkeys")
    private var activeIndex = 0
    private var inactiveIndex = 0
    private let statusRefreshInterval: TimeInterval = 2.0
    private let automaticUpdateCheckInterval: TimeInterval = 12 * 60 * 60
    private let automaticUpdateStartupDelay: TimeInterval = 10.0
    private let releasesAPIURL = URL(string: "https://api.github.com/repos/CharlonTank/claude-code-sleep-preventer/releases/latest")!
    private let fallbackReleaseURL = URL(string: "https://github.com/CharlonTank/claude-code-sleep-preventer/releases/latest")!
    private let lastUpdateCheckDefaultsKey = "ccsp.lastUpdateCheckDate"
    private let skippedUpdateVersionDefaultsKey = "ccsp.skippedUpdateVersion"

    func applicationDidFinishLaunching(_ notification: Notification) {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.title = "Zz"
        }

        menu.delegate = self
        updateMenu(with: .empty)
        item.menu = menu
        statusItem = item

        startAgent()
        registerHotKeys()
        refreshMenu()
        startStatusRefreshTimer()
        promptInstallHooksIfNeeded()
        startUpdateChecks()
    }

    func applicationWillTerminate(_ notification: Notification) {
        statusRefreshTimer?.invalidate()
        updateCheckTimer?.invalidate()
        unregisterHotKeys()
        stopAgent()
    }

    func menuWillOpen(_ menu: NSMenu) {
        refreshMenu()
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }

    @objc private func openLogs() {
        let logDir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/ClaudeSleepPreventer")
        NSWorkspace.shared.open(logDir)
    }

    @objc private func openSettings() {
        let cliPath = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/claude-sleep-preventer")

        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            process.executableURL = cliPath
            process.arguments = ["settings"]
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            do {
                try process.run()
                process.waitUntilExit()
            } catch {
                NSLog("Failed to open settings: \(error)")
            }
            DispatchQueue.main.async {
                self.refreshMenu()
            }
        }
    }

    @objc private func focusInstance(_ sender: NSMenuItem) {
        focusPid(sender.tag)
    }

    private func startAgent() {
        guard agentProcess == nil else { return }
        let agentURL = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/claude-sleep-preventer")

        let process = Process()
        process.executableURL = agentURL
        process.arguments = ["agent"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice

        do {
            try process.run()
            agentProcess = process
        } catch {
            NSLog("Failed to start ccsp-agent: \(error)")
        }
    }

    private func stopAgent() {
        guard let process = agentProcess else { return }
        if process.isRunning {
            process.terminate()
            process.waitUntilExit()
        }
        agentProcess = nil
    }

    @objc private func installHooksAction() {
        showInstallDialog()
    }

    @objc private func uninstallAction() {
        showUninstallDialog()
    }

    @objc private func checkForUpdatesAction() {
        checkForUpdates(mode: .manual)
    }

    @objc private func downloadAvailableUpdateAction() {
        guard let availableUpdate else {
            checkForUpdates(mode: .manual)
            return
        }
        openUpdate(availableUpdate)
    }

    private func refreshMenu() {
        if isRefreshing {
            return
        }
        isRefreshing = true
        DispatchQueue.global(qos: .background).async {
            let list = self.fetchInstanceList()
            DispatchQueue.main.async {
                self.updateMenu(with: list)
                self.isRefreshing = false
            }
        }
    }

    private func startStatusRefreshTimer() {
        statusRefreshTimer?.invalidate()
        let timer = Timer(
            timeInterval: statusRefreshInterval,
            repeats: true
        ) { [weak self] _ in
            self?.refreshStatusTitle()
        }
        statusRefreshTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    private func startUpdateChecks() {
        updateCheckTimer?.invalidate()
        let timer = Timer(
            timeInterval: automaticUpdateCheckInterval,
            repeats: true
        ) { [weak self] _ in
            self?.checkForUpdates(mode: .automatic)
        }
        updateCheckTimer = timer
        RunLoop.main.add(timer, forMode: .common)

        DispatchQueue.main.asyncAfter(deadline: .now() + automaticUpdateStartupDelay) { [weak self] in
            self?.checkForUpdatesIfDue()
        }
    }

    private func refreshStatusTitle() {
        if isRefreshingStatus {
            return
        }
        isRefreshingStatus = true
        DispatchQueue.global(qos: .utility).async {
            let list = self.fetchInstanceList()
            DispatchQueue.main.async {
                self.updateStatusTitle(with: list)
                self.isRefreshingStatus = false
            }
        }
    }

    private func checkForUpdatesIfDue() {
        let defaults = UserDefaults.standard
        if let lastCheckDate = defaults.object(forKey: lastUpdateCheckDefaultsKey) as? Date,
           Date().timeIntervalSince(lastCheckDate) < automaticUpdateCheckInterval {
            return
        }
        checkForUpdates(mode: .automatic)
    }

    private func checkForUpdates(mode: UpdateCheckMode) {
        if isCheckingForUpdates {
            return
        }

        isCheckingForUpdates = true

        var request = URLRequest(url: releasesAPIURL)
        request.httpMethod = "GET"
        request.timeoutInterval = 15
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.setValue("ClaudeSleepPreventer/\(currentAppVersion())", forHTTPHeaderField: "User-Agent")

        URLSession.shared.dataTask(with: request) { data, response, error in
            let result = self.parseLatestRelease(data: data, response: response, error: error)
            DispatchQueue.main.async {
                self.isCheckingForUpdates = false
                UserDefaults.standard.set(Date(), forKey: self.lastUpdateCheckDefaultsKey)
                self.handleUpdateCheckResult(result, mode: mode)
            }
        }.resume()
    }

    private func parseLatestRelease(
        data: Data?,
        response: URLResponse?,
        error: Error?
    ) -> Result<UpdateRelease, UpdateCheckError> {
        if let error {
            return .failure(UpdateCheckError(message: "Unable to contact GitHub: \(error.localizedDescription)"))
        }

        guard let httpResponse = response as? HTTPURLResponse else {
            return .failure(UpdateCheckError(message: "GitHub update check returned an invalid response."))
        }

        guard (200...299).contains(httpResponse.statusCode) else {
            return .failure(UpdateCheckError(message: "GitHub update check failed with HTTP \(httpResponse.statusCode)."))
        }

        guard let data else {
            return .failure(UpdateCheckError(message: "GitHub update check returned no data."))
        }

        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let tagName = json["tag_name"] as? String
        else {
            return .failure(UpdateCheckError(message: "Could not parse the latest GitHub release."))
        }

        let version = normalizedVersion(tagName)
        if version.isEmpty {
            return .failure(UpdateCheckError(message: "The latest GitHub release does not have a valid version tag."))
        }

        let releaseURL = URL(string: json["html_url"] as? String ?? "") ?? fallbackReleaseURL
        let downloadURL = latestDMGURL(from: json) ?? releaseURL

        return .success(
            UpdateRelease(
                version: version,
                releaseURL: releaseURL,
                downloadURL: downloadURL
            )
        )
    }

    private func latestDMGURL(from json: [String: Any]) -> URL? {
        guard let assets = json["assets"] as? [[String: Any]] else {
            return nil
        }

        for asset in assets {
            guard
                let name = asset["name"] as? String,
                name.hasSuffix(".dmg"),
                let download = asset["browser_download_url"] as? String,
                let url = URL(string: download)
            else {
                continue
            }
            return url
        }

        return nil
    }

    private func handleUpdateCheckResult(
        _ result: Result<UpdateRelease, UpdateCheckError>,
        mode: UpdateCheckMode
    ) {
        switch result {
        case .success(let release):
            let currentVersion = currentAppVersion()
            if isVersion(release.version, newerThan: currentVersion) {
                availableUpdate = release
                let skippedVersion = UserDefaults.standard.string(forKey: skippedUpdateVersionDefaultsKey)
                if mode == .manual || skippedVersion != release.version {
                    showUpdateAvailableAlert(release, mode: mode, currentVersion: currentVersion)
                }
            } else {
                availableUpdate = nil
                UserDefaults.standard.removeObject(forKey: skippedUpdateVersionDefaultsKey)
                if mode == .manual {
                    showUpToDateAlert(currentVersion: currentVersion)
                }
            }
        case .failure(let error):
            if mode == .manual {
                showUpdateError(error.message)
            }
        }
    }

    private func currentAppVersion() -> String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        return normalizedVersion(version ?? "0.0.0")
    }

    private func normalizedVersion(_ version: String) -> String {
        var normalized = version.trimmingCharacters(in: .whitespacesAndNewlines)
        if normalized.hasPrefix("v") {
            normalized.removeFirst()
        }
        return normalized
    }

    private func isVersion(_ candidate: String, newerThan current: String) -> Bool {
        normalizedVersion(candidate).compare(
            normalizedVersion(current),
            options: [.numeric]
        ) == .orderedDescending
    }

    private func showUpdateAvailableAlert(
        _ release: UpdateRelease,
        mode: UpdateCheckMode,
        currentVersion: String
    ) {
        let alert = NSAlert()
        alert.messageText = "Update available"

        let checkSource = mode == .automatic ? "A background update check found" : "A newer version is available"
        alert.informativeText = """
            \(checkSource): Claude Sleep Preventer \(release.version).
            You are currently running \(currentVersion).

            Download the latest DMG to update the app.
            """

        alert.addButton(withTitle: "Download")
        alert.addButton(withTitle: "Later")
        alert.addButton(withTitle: "Skip This Version")

        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            openUpdate(release)
            return
        }
        if response == .alertThirdButtonReturn {
            UserDefaults.standard.set(release.version, forKey: skippedUpdateVersionDefaultsKey)
        }
    }

    private func showUpToDateAlert(currentVersion: String) {
        let alert = NSAlert()
        alert.messageText = "You're up to date"
        alert.informativeText = "Claude Sleep Preventer \(currentVersion) is the latest available version."
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    private func showUpdateError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Update check failed"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    private func openUpdate(_ release: UpdateRelease) {
        if !NSWorkspace.shared.open(release.downloadURL) {
            NSWorkspace.shared.open(release.releaseURL)
        }
    }

    private func fetchInstanceList() -> InstanceList {
        let agentURL = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/claude-sleep-preventer")

        let process = Process()
        process.executableURL = agentURL
        process.arguments = ["list"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice

        do {
            try process.run()
        } catch {
            return .empty
        }

        process.waitUntilExit()
        let hooksInstalled = isHooksInstalled()

        guard process.terminationStatus == 0 else {
            return InstanceList(active: [], inactive: [], hooksInstalled: hooksInstalled, sleepDisabled: false)
        }

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return InstanceList(active: [], inactive: [], hooksInstalled: hooksInstalled, sleepDisabled: false)
        }

        let activeArray = json["active"] as? [[String: Any]] ?? []
        var active: [ActiveInstance] = []
        for item in activeArray {
            guard
                let pid = (item["pid"] as? NSNumber)?.intValue,
                let ageSecs = (item["age_secs"] as? NSNumber)?.intValue,
                let cpu = (item["cpu"] as? NSNumber)?.doubleValue,
                let location = item["location"] as? String
            else {
                continue
            }
            active.append(ActiveInstance(pid: pid, ageSecs: ageSecs, cpu: cpu, location: location))
        }

        let inactiveArray = json["inactive"] as? [Any] ?? []
        let inactive = inactiveArray.compactMap { ($0 as? NSNumber)?.intValue }
        let sleepDisabled = (json["sleep_disabled"] as? NSNumber)?.boolValue ?? false

        return InstanceList(active: active, inactive: inactive, hooksInstalled: hooksInstalled, sleepDisabled: sleepDisabled)
    }

    private func updateMenu(with list: InstanceList) {
        updateStatusTitle(with: list)
        menu.removeAllItems()

        menu.addItem(disabledItem("Active Instances"))
        if list.active.isEmpty {
            menu.addItem(disabledItem("  No Active Instances"))
        } else {
            let maxItems = 6
            for instance in list.active.prefix(maxItems) {
                let cpuText = String(format: "%.1f%%", instance.cpu)
                let title = "\(instance.location) [\(instance.pid)] - \(instance.ageSecs)s - \(cpuText)"
                let item = NSMenuItem(title: title, action: #selector(focusInstance), keyEquivalent: "")
                item.target = self
                item.tag = instance.pid
                item.indentationLevel = 1
                menu.addItem(item)
            }
            if list.active.count > maxItems {
                let moreItem = disabledItem("...")
                moreItem.indentationLevel = 1
                menu.addItem(moreItem)
            }
        }

        if list.inactiveCount > 0 {
            menu.addItem(NSMenuItem.separator())
            menu.addItem(disabledItem("Inactive Instances"))
            let maxInactiveItems = 6
            for pid in list.inactive.prefix(maxInactiveItems) {
                let title = "PID \(pid)"
                let item = NSMenuItem(title: title, action: #selector(focusInstance), keyEquivalent: "")
                item.target = self
                item.tag = pid
                item.indentationLevel = 1
                menu.addItem(item)
            }
            if list.inactiveCount > maxInactiveItems {
                let moreItem = disabledItem("...")
                moreItem.indentationLevel = 1
                menu.addItem(moreItem)
            }
        }

        menu.addItem(NSMenuItem.separator())

        let sleepStatus = list.sleepDisabled ? "disablesleep = 1 (sleep blocked)" : "disablesleep = 0 (sleep allowed)"
        menu.addItem(disabledItem(sleepStatus))

        menu.addItem(NSMenuItem.separator())

        menu.addItem(disabledItem("Claude Hooks"))
        let hooksStatus = list.hooksInstalled ? "  Installed" : "  Not installed"
        menu.addItem(disabledItem(hooksStatus))
        let hooksTitle = list.hooksInstalled ? "Reinstall Claude Hooks..." : "Install Claude Hooks..."
        let hooksItem = NSMenuItem(title: hooksTitle, action: #selector(installHooksAction), keyEquivalent: "i")
        hooksItem.target = self
        menu.addItem(hooksItem)

        menu.addItem(NSMenuItem.separator())

        if let availableUpdate {
            let downloadUpdateItem = NSMenuItem(
                title: "Download Update \(availableUpdate.version)...",
                action: #selector(downloadAvailableUpdateAction),
                keyEquivalent: ""
            )
            downloadUpdateItem.target = self
            menu.addItem(downloadUpdateItem)
        }

        let checkForUpdatesItem = NSMenuItem(
            title: isCheckingForUpdates ? "Checking for Updates..." : "Check for Updates...",
            action: #selector(checkForUpdatesAction),
            keyEquivalent: ""
        )
        checkForUpdatesItem.target = self
        checkForUpdatesItem.isEnabled = !isCheckingForUpdates
        menu.addItem(checkForUpdatesItem)

        menu.addItem(NSMenuItem.separator())

        let settingsItem = NSMenuItem(title: "Settings...", action: #selector(openSettings), keyEquivalent: ",")
        settingsItem.target = self
        let logsItem = NSMenuItem(title: "Open Logs", action: #selector(openLogs), keyEquivalent: "l")
        logsItem.target = self
        let uninstallItem = NSMenuItem(title: "Uninstall...", action: #selector(uninstallAction), keyEquivalent: "")
        uninstallItem.target = self
        let quitItem = NSMenuItem(title: "Quit", action: #selector(quit), keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(settingsItem)
        menu.addItem(logsItem)
        menu.addItem(uninstallItem)
        menu.addItem(quitItem)
    }

    private func updateStatusTitle(with list: InstanceList) {
        guard let button = statusItem?.button else {
            return
        }
        if list.active.isEmpty {
            button.title = "Zz"
        } else {
            button.title = "ON \(list.active.count)"
        }
    }

    private func registerHotKeys() {
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )

        let installStatus = InstallEventHandler(
            GetApplicationEventTarget(),
            { _, eventRef, userData in
                guard let userData = userData else {
                    return OSStatus(noErr)
                }
                let app = Unmanaged<AppDelegate>.fromOpaque(userData).takeUnretainedValue()
                var hotKeyId = EventHotKeyID()
                let status = GetEventParameter(
                    eventRef,
                    EventParamName(kEventParamDirectObject),
                    EventParamType(typeEventHotKeyID),
                    nil,
                    MemoryLayout<EventHotKeyID>.size,
                    nil,
                    &hotKeyId
                )
                if status == noErr && hotKeyId.signature == app.hotKeySignature {
                    app.handleHotKey(id: hotKeyId.id)
                }
                return OSStatus(noErr)
            },
            1,
            &eventType,
            UnsafeMutableRawPointer(Unmanaged.passUnretained(self).toOpaque()),
            &hotKeyHandler
        )

        if installStatus != noErr {
            NSLog("Failed to install hotkey handler: \(installStatus)")
            return
        }

        let modifiers = UInt32(controlKey | optionKey | cmdKey)
        let activeId = EventHotKeyID(signature: hotKeySignature, id: 1)
        let inactiveId = EventHotKeyID(signature: hotKeySignature, id: 2)

        let activeStatus = RegisterEventHotKey(
            UInt32(kVK_ANSI_J),
            modifiers,
            activeId,
            GetApplicationEventTarget(),
            0,
            &hotKeyActive
        )
        if activeStatus != noErr {
            NSLog("Failed to register active hotkey: \(activeStatus)")
        }

        let inactiveStatus = RegisterEventHotKey(
            UInt32(kVK_ANSI_L),
            modifiers,
            inactiveId,
            GetApplicationEventTarget(),
            0,
            &hotKeyInactive
        )
        if inactiveStatus != noErr {
            NSLog("Failed to register inactive hotkey: \(inactiveStatus)")
        }
    }

    private func unregisterHotKeys() {
        if let hotKeyActive = hotKeyActive {
            UnregisterEventHotKey(hotKeyActive)
        }
        if let hotKeyInactive = hotKeyInactive {
            UnregisterEventHotKey(hotKeyInactive)
        }
        if let hotKeyHandler = hotKeyHandler {
            RemoveEventHandler(hotKeyHandler)
        }
    }

    private func handleHotKey(id: UInt32) {
        if id == 1 {
            cycleActiveInstance()
        } else if id == 2 {
            cycleInactiveInstance()
        }
    }

    private func cycleActiveInstance() {
        hotKeyQueue.async {
            let list = self.fetchInstanceList()
            guard !list.active.isEmpty else {
                return
            }
            let idx = self.activeIndex % list.active.count
            self.activeIndex = (self.activeIndex + 1) % list.active.count
            self.focusPid(list.active[idx].pid)
        }
    }

    private func cycleInactiveInstance() {
        hotKeyQueue.async {
            let list = self.fetchInstanceList()
            guard !list.inactive.isEmpty else {
                return
            }
            let idx = self.inactiveIndex % list.inactive.count
            self.inactiveIndex = (self.inactiveIndex + 1) % list.inactive.count
            self.focusPid(list.inactive[idx])
        }
    }

    private func focusPid(_ pid: Int) {
        if pid <= 0 {
            return
        }

        let agentURL = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/claude-sleep-preventer")

        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            process.executableURL = agentURL
            process.arguments = ["focus", String(pid)]
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            do {
                try process.run()
                process.waitUntilExit()
            } catch {
                NSLog("Failed to focus instance pid=\(pid): \(error)")
            }
        }
    }

    private func disabledItem(_ title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        return item
    }

    private func isHooksInstalled() -> Bool {
        let hooksPath = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude/hooks/prevent-sleep.sh")
        return FileManager.default.fileExists(atPath: hooksPath.path)
    }

    private func promptInstallHooksIfNeeded() {
        if isHooksInstalled() {
            return
        }
        showInstallDialog(canCancel: false)
    }

    private func showInstallDialog(canCancel: Bool = true) {
        let alert = NSAlert()
        alert.messageText = "Install Claude Code Hooks"
        alert.informativeText = """
            This will configure Claude Code hooks to automatically prevent sleep while Claude is working.

            The hooks will:
            • Prevent system sleep when Claude starts a task
            • Re-enable sleep when Claude finishes

            Administrator password required.
            """
        alert.addButton(withTitle: "Install Hooks")
        if canCancel {
            alert.addButton(withTitle: "Cancel")
        }

        let contentView = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 30))
        let debugCheck = NSButton(checkboxWithTitle: "Enable debug logging", target: nil, action: nil)
        debugCheck.state = .off
        debugCheck.frame = NSRect(x: 0, y: 5, width: 300, height: 20)
        contentView.addSubview(debugCheck)
        alert.accessoryView = contentView

        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            installHooks(debug: debugCheck.state == .on)
        }
    }

    private func installHooks(debug: Bool = false) {
        if isInstalling {
            return
        }
        isInstalling = true

        // TODO: Use debug flag when implemented in CLI
        _ = debug

        let cliPath = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/claude-sleep-preventer")
            .path
        let command = "\(cliPath) install -y"
        let applescript = "do shell script \"\(escapeForAppleScript(command))\" with administrator privileges"

        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
            process.arguments = ["-e", applescript]
            let errPipe = Pipe()
            process.standardError = errPipe

            do {
                try process.run()
            } catch {
                DispatchQueue.main.async {
                    self.isInstalling = false
                    self.showInstallError("Failed to start installer: \(error)")
                }
                return
            }

            process.waitUntilExit()
            let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
            let errText = String(data: errData, encoding: .utf8) ?? ""
            let status = process.terminationStatus

            DispatchQueue.main.async {
                self.isInstalling = false
                if status == 0 {
                    self.showInstallSuccess()
                    self.refreshMenu()
                    return
                }
                if errText.contains("-128") || errText.contains("User canceled") {
                    return
                }
                self.showInstallError(errText.isEmpty ? "Install failed." : errText)
            }
        }
    }

    private func showInstallSuccess() {
        let alert = NSAlert()
        alert.messageText = "Setup complete"
        alert.informativeText = "Restart Claude Code to activate sleep prevention."
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    private func showInstallError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Setup failed"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    private func escapeForAppleScript(_ text: String) -> String {
        return text
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
    }

    private func showUninstallDialog() {
        let alert = NSAlert()
        alert.messageText = "Uninstall Claude Sleep Preventer"
        alert.informativeText = "Select what to remove:"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Uninstall")
        alert.addButton(withTitle: "Cancel")

        let contentView = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))

        let hooksCheck = NSButton(checkboxWithTitle: "Remove Claude Code hooks", target: nil, action: nil)
        hooksCheck.state = .on
        hooksCheck.frame = NSRect(x: 0, y: 70, width: 300, height: 20)

        let modelCheck = NSButton(checkboxWithTitle: "Remove Whisper model (~1.5 GB)", target: nil, action: nil)
        modelCheck.state = .on
        modelCheck.frame = NSRect(x: 0, y: 45, width: 300, height: 20)

        let dataCheck = NSButton(checkboxWithTitle: "Remove app data and logs", target: nil, action: nil)
        dataCheck.state = .on
        dataCheck.frame = NSRect(x: 0, y: 20, width: 300, height: 20)

        contentView.addSubview(hooksCheck)
        contentView.addSubview(modelCheck)
        contentView.addSubview(dataCheck)
        alert.accessoryView = contentView

        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            performUninstall(
                removeHooks: hooksCheck.state == .on,
                removeModel: modelCheck.state == .on,
                removeData: dataCheck.state == .on
            )
        }
    }

    private func performUninstall(removeHooks: Bool, removeModel: Bool, removeData: Bool) {
        if isUninstalling {
            return
        }
        isUninstalling = true

        var args = ["uninstall"]
        if !removeModel {
            args.append("-k")
        }
        if !removeHooks {
            args.append("--keep-hooks")
        }
        if !removeData {
            args.append("--keep-data")
        }

        let cliPath = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/claude-sleep-preventer")
            .path
        let command = ([cliPath] + args).joined(separator: " ")
        let applescript = "do shell script \"\(escapeForAppleScript(command))\" with administrator privileges"

        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
            process.arguments = ["-e", applescript]
            let errPipe = Pipe()
            process.standardError = errPipe

            do {
                try process.run()
            } catch {
                DispatchQueue.main.async {
                    self.isUninstalling = false
                    self.showUninstallError("Failed to start uninstaller: \(error)")
                }
                return
            }

            process.waitUntilExit()
            let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
            let errText = String(data: errData, encoding: .utf8) ?? ""
            let status = process.terminationStatus

            DispatchQueue.main.async {
                self.isUninstalling = false
                if status == 0 {
                    self.showUninstallSuccess()
                    NSApp.terminate(nil)
                    return
                }
                if errText.contains("-128") || errText.contains("User canceled") {
                    return
                }
                self.showUninstallError(errText.isEmpty ? "Uninstall failed." : errText)
            }
        }
    }

    private func showUninstallSuccess() {
        let alert = NSAlert()
        alert.messageText = "Uninstall complete"
        alert.informativeText = "Claude Sleep Preventer has been removed."
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    private func showUninstallError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Uninstall failed"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }
}

@main
struct CCSPMenubarApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }
}
