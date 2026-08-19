// Long-lived display-only harness for testing capture exclusion against a REAL
// third-party ScreenCaptureKit consumer (Zoom / Meet / Teams screen share).
//
// Identical window setup to hold.swift / main.swift — same geometry, same
// sharingType values — but it captures nothing and stays up long enough to
// start a screen share. Pure display, so it needs no Screen Recording
// permission of its own; the capturing app needs its own.

import AppKit

let LIFETIME_SECONDS = 900.0 // 15 minutes

func makeWindow(_ line1: String, _ line2: String, _ sharing: NSWindow.SharingType,
                _ x: CGFloat, _ color: NSColor) -> NSWindow {
    let w = NSWindow(contentRect: NSRect(x: x, y: 460, width: 460, height: 240),
                     styleMask: [.borderless], backing: .buffered, defer: false)
    w.sharingType = sharing
    w.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.assistiveTechHighWindow)) - 1)
    w.backgroundColor = color
    w.isOpaque = true
    w.hasShadow = false
    w.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .transient, .ignoresCycle]
    w.isReleasedWhenClosed = false

    let v = NSView(frame: w.contentView!.bounds)
    v.wantsLayer = true
    v.layer?.backgroundColor = color.cgColor

    let l1 = NSTextField(labelWithString: line1)
    l1.font = .boldSystemFont(ofSize: 38)
    l1.textColor = .white
    l1.backgroundColor = .clear
    l1.isBordered = false
    l1.frame = NSRect(x: 20, y: 132, width: 430, height: 52)
    v.addSubview(l1)

    let l2 = NSTextField(labelWithString: line2)
    l2.font = .boldSystemFont(ofSize: 25)
    l2.textColor = .white
    l2.backgroundColor = .clear
    l2.isBordered = false
    l2.frame = NSRect(x: 20, y: 74, width: 430, height: 40)
    v.addSubview(l2)

    w.contentView = v
    w.orderFrontRegardless()
    print("  window '\(line1)': sharingType set=\(sharing.rawValue) readback=\(w.sharingType.rawValue) id=\(w.windowNumber)")
    return w
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

var windows: [NSWindow] = []

DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
    windows.append(makeWindow(".none", "SHOULD BE HIDDEN", .none, 120, .systemRed))
    windows.append(makeWindow(".readOnly", "SHOULD BE VISIBLE", .readOnly, 640, .systemGreen))
    print("WINDOWS UP — both on screen for \(Int(LIFETIME_SECONDS / 60)) minutes")
    fflush(stdout)
}

// Heartbeat so the log shows how much time is left.
var elapsed = 0.0
Timer.scheduledTimer(withTimeInterval: 60.0, repeats: true) { _ in
    elapsed += 60.0
    print("  still up — \(Int((LIFETIME_SECONDS - elapsed) / 60)) min remaining")
    fflush(stdout)
}

DispatchQueue.main.asyncAfter(deadline: .now() + LIFETIME_SECONDS) {
    print("LIFETIME REACHED — exiting")
    exit(0)
}

app.run()
