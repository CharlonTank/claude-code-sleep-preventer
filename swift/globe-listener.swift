import Cocoa
import Foundation

// Global state
var fnIsDown = false
var shiftIsDown = false
var isDictating = false
var eventTap: CFMachPort?
var debugMode = true  // Enable debug output

// Event tap callback
func eventTapCallback(
    proxy: CGEventTapProxy,
    type: CGEventType,
    event: CGEvent,
    refcon: UnsafeMutableRawPointer?
) -> Unmanaged<CGEvent>? {
    // Re-enable tap if it was disabled
    if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
        if debugMode {
            fputs("DEBUG: Event tap was disabled, re-enabling...\n", stderr)
        }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: true)
        }
        return Unmanaged.passUnretained(event)
    }

    let flags = event.flags
    let keyCode = event.getIntegerValueField(.keyboardEventKeycode)

    // Check various flags
    let containsFn = flags.contains(.maskSecondaryFn)
    let containsShift = flags.contains(.maskShift)
    let containsCmd = flags.contains(.maskCommand)
    let containsAlt = flags.contains(.maskAlternate)
    let containsCtrl = flags.contains(.maskControl)

    // Debug: print all flag changes
    if type == .flagsChanged && debugMode {
        var activeFlags: [String] = []
        if containsFn { activeFlags.append("Fn") }
        if containsShift { activeFlags.append("Shift") }
        if containsCmd { activeFlags.append("Cmd") }
        if containsAlt { activeFlags.append("Alt") }
        if containsCtrl { activeFlags.append("Ctrl") }

        let flagsStr = activeFlags.isEmpty ? "(none)" : activeFlags.joined(separator: "+")
        fputs("DEBUG: flagsChanged - keyCode=\(keyCode) flags=\(flagsStr) rawFlags=\(flags.rawValue)\n", stderr)
    }

    // Track modifier state changes (flagsChanged events)
    if type == .flagsChanged {
        let wasDictating = isDictating

        fnIsDown = containsFn
        shiftIsDown = containsShift

        // Start dictation when both Fn AND Shift are held
        let shouldDictate = fnIsDown && shiftIsDown

        if shouldDictate && !wasDictating {
            isDictating = true
            print("DICTATE_START")
            fflush(stdout)
            if debugMode {
                fputs("DEBUG: >>> DICTATE_START\n", stderr)
            }
        } else if !shouldDictate && wasDictating {
            isDictating = false
            print("DICTATE_STOP")
            fflush(stdout)
            if debugMode {
                fputs("DEBUG: >>> DICTATE_STOP\n", stderr)
            }
        }
    }

    return Unmanaged.passUnretained(event)
}

// Main
func main() {
    fputs("DEBUG: Starting globe-listener...\n", stderr)

    // Event mask: flagsChanged for modifiers
    let eventMask = CGEventMask(1 << CGEventType.flagsChanged.rawValue)

    fputs("DEBUG: Creating event tap...\n", stderr)

    // Create event tap
    guard let tap = CGEvent.tapCreate(
        tap: .cgSessionEventTap,
        place: .headInsertEventTap,
        options: .listenOnly,
        eventsOfInterest: eventMask,
        callback: eventTapCallback,
        userInfo: nil
    ) else {
        fputs("ERROR:EVENT_TAP_FAILED\n", stderr)
        fputs("DEBUG: Failed to create event tap!\n", stderr)
        fputs("DEBUG: This usually means Accessibility permission is not granted.\n", stderr)
        fputs("DEBUG: Go to: System Settings > Privacy & Security > Accessibility\n", stderr)
        fputs("DEBUG: And add this app or your terminal.\n", stderr)
        exit(1)
    }

    eventTap = tap
    fputs("DEBUG: Event tap created successfully\n", stderr)

    // Add to run loop
    let runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
    CFRunLoopAddSource(CFRunLoopGetCurrent(), runLoopSource, .commonModes)
    CGEvent.tapEnable(tap: tap, enable: true)
    fputs("DEBUG: Event tap enabled and added to run loop\n", stderr)

    // Handle SIGTERM gracefully
    signal(SIGTERM, SIG_IGN)
    let signalSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    signalSource.setEventHandler {
        CFRunLoopStop(CFRunLoopGetCurrent())
        exit(0)
    }
    signalSource.resume()

    // Signal ready
    print("READY")
    fflush(stdout)
    fputs("DEBUG: Ready and listening for Fn+Shift...\n", stderr)
    fputs("DEBUG: Press Fn+Shift to test. You should see flag changes below.\n", stderr)

    // Run the event loop
    CFRunLoopRun()
}

main()
