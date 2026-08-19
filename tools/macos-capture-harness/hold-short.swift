import AppKit
func makeWindow(_ title: String, _ sharing: NSWindow.SharingType, _ x: CGFloat, _ color: NSColor) -> NSWindow {
    let w = NSWindow(contentRect: NSRect(x: x, y: 460, width: 460, height: 240),
                     styleMask: [.borderless], backing: .buffered, defer: false)
    w.sharingType = sharing
    w.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.assistiveTechHighWindow)) - 1)
    w.backgroundColor = color; w.isOpaque = true; w.hasShadow = false
    w.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .transient, .ignoresCycle]
    w.isReleasedWhenClosed = false
    let v = NSView(frame: w.contentView!.bounds); v.wantsLayer = true
    v.layer?.backgroundColor = color.cgColor
    let l = NSTextField(labelWithString: title)
    l.font = .boldSystemFont(ofSize: 40); l.textColor = .white; l.backgroundColor = .clear; l.isBordered = false
    l.frame = NSRect(x: 24, y: 96, width: 420, height: 56); v.addSubview(l)
    w.contentView = v; w.orderFrontRegardless()
    return w
}
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
var ws: [NSWindow] = []
DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
    ws.append(makeWindow("SHARING = .none", .none, 120, .systemRed))
    ws.append(makeWindow("SHARING = .readOnly", .readOnly, 640, .systemGreen))
    print("windows up")
}
DispatchQueue.main.asyncAfter(deadline: .now() + 25) { exit(0) }
app.run()
