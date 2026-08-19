import AppKit
import ScreenCaptureKit
import ImageIO
import UniformTypeIdentifiers

func savePNG(_ img: CGImage, _ path: String) -> Bool {
    let url = URL(fileURLWithPath: path)
    guard let dst = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else { return false }
    CGImageDestinationAddImage(dst, img, nil)
    return CGImageDestinationFinalize(dst)
}

func makeWindow(_ title: String, _ sharing: NSWindow.SharingType, _ x: CGFloat, _ color: NSColor) -> NSWindow {
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
    let label = NSTextField(labelWithString: title)
    label.font = .boldSystemFont(ofSize: 40)
    label.textColor = .white
    label.backgroundColor = .clear
    label.isBordered = false
    label.frame = NSRect(x: 24, y: 96, width: 420, height: 56)
    v.addSubview(label)
    w.contentView = v
    w.orderFrontRegardless()
    print("window '\(title)' sharingType set=\(sharing.rawValue) readback=\(w.sharingType.rawValue) id=\(w.windowNumber)")
    return w
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

var windows: [NSWindow] = []

func runCaptures() async {
    // ---- 1. ScreenCaptureKit (the modern path Zoom/Teams/OBS/screencapture use) ----
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let display = content.displays.first else { print("SCK: no display"); return }
        let filter = SCContentFilter(display: display, excludingWindows: [])
        let cfg = SCStreamConfiguration()
        cfg.width = display.width
        cfg.height = display.height
        cfg.showsCursor = false
        let img = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg)
        print("SCK: captured \(img.width)x\(img.height) -> \(savePNG(img, "/tmp/sharetest/sck.png"))")

        // Does SCK even report our sharingType=.none window as shareable content?
        let mine = content.windows.filter { $0.owningApplication?.processID == ProcessInfo.processInfo.processIdentifier }
        print("SCK: shareable windows owned by this process = \(mine.count)")
        for w in mine { print("   - id=\(w.windowID) title=\(w.title ?? "nil") onScreen=\(w.isOnScreen) frame=\(w.frame)") }
    } catch {
        print("SCK ERROR: \(error)")
    }

    // ---- 2. Legacy CoreGraphics path (obsoleted in SDK 15.0; reach it via dlsym) ----
    typealias WLCIFn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
    if let sym = dlsym(dlopen(nil, RTLD_NOW), "CGWindowListCreateImage") {
        let fn = unsafeBitCast(sym, to: WLCIFn.self)
        // CGRectInfinite, kCGWindowListOptionOnScreenOnly = 1, kCGNullWindowID = 0, bestResolution = 0
        if let ref = fn(CGRect.infinite, 1, 0, 0) {
            let cg = ref.takeRetainedValue()
            print("LEGACY CGWindowListCreateImage: captured \(cg.width)x\(cg.height) -> \(savePNG(cg, "/tmp/sharetest/cglegacy.png"))")
        } else {
            print("LEGACY CGWindowListCreateImage: returned nil (blocked or no permission)")
        }
    } else {
        print("LEGACY CGWindowListCreateImage: symbol not found in dyld")
    }

    // ---- 3. What does the window-list metadata say? ----
    if let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] {
        let pid = ProcessInfo.processInfo.processIdentifier
        for d in info where (d[kCGWindowOwnerPID as String] as? Int32) == pid {
            print("CGWindowList meta: num=\(d[kCGWindowNumber as String] ?? "?") sharingState=\(d[kCGWindowSharingState as String] ?? "?") bounds=\(d[kCGWindowBounds as String] ?? "?")")
        }
    }
    exit(0)
}

DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
    windows.append(makeWindow("SHARING = .none", .none, 120, .systemRed))
    windows.append(makeWindow("SHARING = .readOnly", .readOnly, 640, .systemGreen))
    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
        Task { await runCaptures() }
    }
}

app.run()
