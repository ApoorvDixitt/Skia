// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0
//
// Watches what Core Audio reports when the default device changes mid-capture.
//
// `docs/ARCHITECTURE.md` calls device hot-swap "the main crash risk" and the
// roadmap lists a proof of concept as Phase 0 work. The reason is mundane and
// unavoidable: people plug in headphones during calls. An engine that assumed a
// stable device gets a stream whose device has gone away, and the usual result
// is a crash on a real-time thread — which, per the same document, must not be
// able to take the webview down with it.
//
// Bluetooth is the harder case and the one to test properly. A headset switching
// A2DP to HFP looks like a device change, a rate change, and often several of
// each within a second or two, which is why WS1.4 debounces the rebuild rather
// than reacting to every notification.
//
// This probe does not rebuild anything. It reports what arrives and when, so the
// debounce window is chosen from observed timings rather than from a guess.
//
// Usage:
//   swiftc -O hotswap-probe.swift -o hotswap-probe
//   ./hotswap-probe [seconds]

import AVFoundation
import CoreAudio
import Foundation

let seconds = CommandLine.arguments.dropFirst().first.flatMap(Double.init) ?? 45.0
let system = AudioObjectID(kAudioObjectSystemObject)
let started = CFAbsoluteTimeGetCurrent()

func property<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> T? {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain
    )
    var size = UInt32(MemoryLayout<T>.size)
    let value = UnsafeMutablePointer<T>.allocate(capacity: 1)
    defer { value.deallocate() }
    guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, value) == noErr else {
        return nil
    }
    return value.pointee
}

func stringProperty(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> String? {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var size = UInt32(MemoryLayout<CFString?>.size)
    var value: CFString? = nil
    let status = withUnsafeMutablePointer(to: &value) {
        AudioObjectGetPropertyData(object, &address, 0, nil, &size, $0)
    }
    guard status == noErr, let value else { return nil }
    return value as String
}

/// Name and nominal rate of a device, for the log line.
func describeDevice(_ device: AudioObjectID) -> String {
    guard device != kAudioObjectUnknown else { return "(none)" }
    let name = stringProperty(device, kAudioObjectPropertyName) ?? "unnamed"
    let rate: Float64 = property(device, kAudioDevicePropertyNominalSampleRate) ?? 0
    return rate > 0 ? "\(name) @ \(Int(rate)) Hz" : name
}

/// Every notification seen, with the time it arrived relative to launch.
final class Log {
    var events: [(at: Double, what: String)] = []

    func record(_ what: String) {
        let at = CFAbsoluteTimeGetCurrent() - started
        events.append((at, what))
        print(String(format: "  %7.3fs  %@", at, what))
    }

    /// Gaps between consecutive events, which is what sizes the debounce.
    var gaps: [Double] {
        guard events.count > 1 else { return [] }
        return (1..<events.count).map { events[$0].at - events[$0 - 1].at }
    }
}

nonisolated(unsafe) let log = Log()

print("=== Skia device hot-swap probe ===\n")
print("output device              \(describeDevice(property(system, kAudioHardwarePropertyDefaultOutputDevice) ?? kAudioObjectUnknown))")
print("input device               \(describeDevice(property(system, kAudioHardwarePropertyDefaultInputDevice) ?? kAudioObjectUnknown))")

// The three notifications an engine has to survive. Watched separately because
// they do not arrive together and an engine that only listens for one of them
// will miss the case that actually breaks it.
let watched: [(AudioObjectPropertySelector, String)] = [
    (kAudioHardwarePropertyDefaultOutputDevice, "default OUTPUT device changed"),
    (kAudioHardwarePropertyDefaultInputDevice, "default INPUT device changed"),
    (kAudioHardwarePropertyDevices, "device list changed"),
]

var listeners: [(AudioObjectPropertyAddress, AudioObjectPropertyListenerBlock)] = []

for (selector, label) in watched {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    let block: AudioObjectPropertyListenerBlock = { _, _ in
        switch selector {
        case kAudioHardwarePropertyDefaultOutputDevice:
            let device: AudioObjectID = property(system, selector) ?? kAudioObjectUnknown
            log.record("\(label) -> \(describeDevice(device))")
        case kAudioHardwarePropertyDefaultInputDevice:
            let device: AudioObjectID = property(system, selector) ?? kAudioObjectUnknown
            log.record("\(label) -> \(describeDevice(device))")
        default:
            log.record(label)
        }
    }
    let status = AudioObjectAddPropertyListenerBlock(system, &address, nil, block)
    if status == noErr {
        listeners.append((address, block))
    } else {
        print("could not listen for \(label) (status \(status))")
    }
}

// A live microphone stream, so the notifications arrive against a device that is
// actually in use. A listener with no stream open is a much weaker test: the
// interesting failures happen because something is mid-capture.
let engine = AVAudioEngine()
let input = engine.inputNode
let format = input.outputFormat(forBus: 0)

/// Frames seen and whether they ever stopped. A stream that silently dies is the
/// failure this probe is looking for, and it does not raise anything.
final class Flow {
    var frames = 0
    var lastArrival: Double? = nil
    var longestGap = 0.0
}
nonisolated(unsafe) let flow = Flow()

if format.sampleRate > 0 {
    input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
        let now = CFAbsoluteTimeGetCurrent()
        if let last = flow.lastArrival {
            flow.longestGap = max(flow.longestGap, now - last)
        }
        flow.lastArrival = now
        flow.frames += Int(buffer.frameLength)
    }
    do {
        try engine.start()
        print("microphone stream          open at \(Int(format.sampleRate)) Hz")
    } catch {
        print("microphone stream          could NOT start: \(error.localizedDescription)")
    }
} else {
    print("microphone stream          no usable input device, listening only")
}

print("""

--- watching for \(seconds)s ---

  Do all of these while it runs, and note roughly when:

    1. Plug in wired headphones, then unplug them.
    2. Connect a Bluetooth headset, then disconnect it. This is the one that
       matters — watch for a burst of notifications rather than a single one.
    3. Change the output device in System Settings > Sound.
    4. If you have one, switch to an external interface at a different rate.

""")

Thread.sleep(forTimeInterval: seconds)

engine.stop()
if format.sampleRate > 0 { input.removeTap(onBus: 0) }
for (var address, block) in listeners {
    AudioObjectRemovePropertyListenerBlock(system, &address, nil, block)
}

// MARK: - Report

print("\n--- result ---")
print("notifications              \(log.events.count)")

if format.sampleRate > 0 {
    let captured = Double(flow.frames) / format.sampleRate
    print(String(format: "microphone captured        %.2fs of %.0fs", captured, seconds))
    print(String(format: "longest gap in the stream  %.0f ms", flow.longestGap * 1000))

    // A long gap means the stream stalled and recovered; no frames at all after
    // a device change means it died. Both are the engine's problem to handle,
    // and both are invisible without measuring.
    if flow.longestGap > 1.0 {
        print("""

          The stream stalled for over a second. AVAudioEngine did not rebuild
          promptly, and an engine built on cpal has no more magic available. This
          is the case WS1.4 has to handle explicitly.
        """)
    }
    if captured < seconds * 0.5, !log.events.isEmpty {
        print("""

          Less than half the wall clock was captured. The stream very likely died
          on a device change and never came back — which is the failure mode that
          matters, because nothing raised an error.
        """)
    }
}

let gaps = log.gaps
if !gaps.isEmpty {
    let tight = gaps.filter { $0 < 1.0 }
    print(String(format: "closest two notifications  %.0f ms apart", (gaps.min() ?? 0) * 1000))
    print("notifications < 1s apart   \(tight.count) of \(gaps.count)")
    print("""

      This is the number that sets the debounce window. Rebuilding a stream once
      per notification means rebuilding it \(tight.count + 1) times for what the user
      experienced as one action, and each rebuild is a gap in the transcript.
    """)
} else if log.events.isEmpty {
    print("""

      Nothing changed, so nothing was learned. Re-run and actually swap a device;
      an empty result here is not evidence that hot-swap is safe.
    """)
}

print("""

=== What to record ===

  The notification count per physical action, the closest gap between two, and
  whether the microphone stream survived. Then pick the debounce window from the
  gap distribution rather than from intuition, and write down which devices were
  tested — a laptop's built-in speakers and a Bluetooth headset behave nothing
  alike here.
""")
