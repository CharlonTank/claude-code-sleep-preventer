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

    static let empty = InstanceList(active: [], inactive: [], hooksInstalled: false)

    var inactiveCount: Int {
        inactive.count
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var statusItem: NSStatusItem?
    private var agentProcess: Process?
    private let menu = NSMenu()
    private var isRefreshing = false
    private var isInstalling = false
    private var isUninstalling = false
    private var hotKeyHandler: EventHandlerRef?
    private var hotKeyActive: EventHotKeyRef?
    private var hotKeyInactive: EventHotKeyRef?
    private let hotKeySignature: OSType = 0x63637370 // 'ccsp'
    private let hotKeyQueue = DispatchQueue(label: "ccsp.hotkeys")
    private var activeIndex = 0
    private var inactiveIndex = 0

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
        promptInstallHooksIfNeeded()
    }

    func applicationWillTerminate(_ notification: Notification) {
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
        installHooks()
    }

    @objc private func uninstallAction() {
        showUninstallDialog()
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
            return InstanceList(active: [], inactive: [], hooksInstalled: hooksInstalled)
        }

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return InstanceList(active: [], inactive: [], hooksInstalled: hooksInstalled)
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

        return InstanceList(active: active, inactive: inactive, hooksInstalled: hooksInstalled)
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

        menu.addItem(disabledItem("Claude Hooks"))
        let hooksStatus = list.hooksInstalled ? "  Installed" : "  Not installed"
        menu.addItem(disabledItem(hooksStatus))
        let hooksTitle = list.hooksInstalled ? "Reinstall Claude Hooks..." : "Install Claude Hooks..."
        let hooksItem = NSMenuItem(title: hooksTitle, action: #selector(installHooksAction), keyEquivalent: "i")
        hooksItem.target = self
        menu.addItem(hooksItem)

        menu.addItem(NSMenuItem.separator())

        let logsItem = NSMenuItem(title: "Open Logs", action: #selector(openLogs), keyEquivalent: "l")
        logsItem.target = self
        let uninstallItem = NSMenuItem(title: "Uninstall...", action: #selector(uninstallAction), keyEquivalent: "")
        uninstallItem.target = self
        let quitItem = NSMenuItem(title: "Quit", action: #selector(quit), keyEquivalent: "q")
        quitItem.target = self
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

        let alert = NSAlert()
        alert.messageText = "Install Claude Code hooks?"
        alert.informativeText = "This sets up Claude Code hooks and sleep control. Administrator password required."
        alert.addButton(withTitle: "Install")
        alert.addButton(withTitle: "Later")
        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            installHooks()
        }
    }

    private func installHooks() {
        if isInstalling {
            return
        }
        isInstalling = true

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
