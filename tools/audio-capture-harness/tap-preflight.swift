// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0
//
// Reports what this machine will allow before anything is captured.
//
// Run this first. It creates no tap, opens no stream, and writes no audio, so
// it cannot trigger a consent prompt — which is the point: the prompt itself is
// one of the things being measured, and a probe that provokes it before you are
// watching has destroyed the measurement.

import AVFoundation
import CoreAudio
import Foundation

// MARK: - Core Audio property helpers

/// Read a fixed-size Core Audio property, or nil if the object does not answer.
func property<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> T? {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain
    )
    var size = UInt32(MemoryLayout<T>.size)
    let value = UnsafeMutablePointer<T>.allocate(capacity: 1)
    defer { value.deallocate() }

    let status = AudioObjectGetPropertyData(object, &address, 0, nil, &size, value)
    guard status == noErr else { return nil }
    return value.pointee
}

/// Read a variable-length Core Audio property holding a list of object ids.
///
/// Deliberately not generic. A generic version tempts the compiler into forming
/// a raw pointer to an array whose element type might contain an object
/// reference, and `AudioObjectID` is the only list this harness ever reads.
func objectList(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> [AudioObjectID] {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(object, &address, 0, nil, &size) == noErr, size > 0 else {
        return []
    }

    // Zero-filled, so a read that fails partway leaves zeros rather than a
    // garbage object id that the caller would happily query.
    var values = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &values) == noErr else {
        return []
    }
    return Array(values.prefix(Int(size) / MemoryLayout<AudioObjectID>.size))
}

/// Read a CFString-typed Core Audio property as a Swift string.
func stringProperty(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> String? {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var size = UInt32(MemoryLayout<CFString?>.size)
    var value: CFString? = nil
    let status = withUnsafeMutablePointer(to: &value) { pointer in
        AudioObjectGetPropertyData(object, &address, 0, nil, &size, pointer)
    }
    guard status == noErr, let value else { return nil }
    return value as String
}

// MARK: - Report

print("=== Skia audio capture preflight ===\n")

let os = ProcessInfo.processInfo.operatingSystemVersion
print("macOS \(os.majorVersion).\(os.minorVersion).\(os.patchVersion)")
print("Record this. Tap behaviour is version-dependent and this file is the")
print("only thing tying a measurement to the OS it was measured on.\n")

// -- Does the tap API exist at all? ------------------------------------------
//
// Two separate questions, and conflating them is the classic mistake:
// `#available` is a runtime check against the OS, while the symbol having
// linked at all is a check against the SDK this binary was built with.

print("--- Core Audio process tap API ---")
if #available(macOS 14.2, *) {
    print("CATapDescription           available (macOS 14.2+ satisfied)")
} else {
    print("CATapDescription           NOT available — this OS predates macOS 14.2.")
    print("                           Far-end capture would have to fall back to")
    print("                           ScreenCaptureKit, which needs Screen Recording")
    print("                           permission. Stop here and record that.")
}

// -- Audio-capture consent ----------------------------------------------------
//
// This is the finding that matters, and there is no public API for it. Apple
// ships no way to check or request audio-capture permission: the prompt is
// triggered implicitly by the first capture, and its wording comes from
// `NSAudioCaptureUsageDescription` in the app's Info.plist.
//
// So the state is read through TCC's private SPI, exactly as
// https://github.com/insidegui/AudioCap does — the reference implementation for
// this API, and the only real documentation of it. **This is diagnostic code and
// the SPI must never appear in Skia itself.** A private symbol that disappears
// in a point release is acceptable in a harness you re-run per OS version and
// unacceptable in a shipped app.
//
// Why it is worth the ugliness: without consent a process tap does not fail. It
// is created, an aggregate device wraps it, callbacks arrive at exactly real
// time — and every sample is zero. Measured on macOS 26.5: 281 callbacks,
// 2.997 s captured in 3.0 s, peak amplitude 0.0000, with audio definitely
// playing. That is indistinguishable from "nothing was playing" unless you know
// to check, which is the same quiet-degradation trap the capture harness
// documents for legacy CoreGraphics.

print("\n--- Audio-capture consent (TCC private SPI, does not prompt) ---")

typealias TCCPreflight = @convention(c) (CFString, CFDictionary?) -> Int

let tccPreflight: TCCPreflight? = {
    let path = "/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC"
    guard let handle = dlopen(path, RTLD_NOW),
          let symbol = dlsym(handle, "TCCAccessPreflight")
    else { return nil }
    return unsafeBitCast(symbol, to: TCCPreflight.self)
}()

if let tccPreflight {
    // 0 and 1 are AudioCap's documented readings. Anything else is "no record",
    // which for a bare CLI binary is the expected answer and is reported as
    // such rather than guessed at.
    for service in ["kTCCServiceAudioCapture", "kTCCServiceMicrophone"] {
        let result = tccPreflight(service as CFString, nil)
        let meaning: String
        switch result {
        case 0: meaning = "authorized"
        case 1: meaning = "denied"
        default: meaning = "no record — never asked, or cannot be asked"
        }
        print(pad(service, 28) + "\(result) — \(meaning)")
    }
} else {
    print("TCC SPI unavailable, so consent state cannot be read on this OS.")
}

print("""

  If audio capture is not authorized, a tap will hand you silence rather than an
  error. Read that again before interpreting any capture result.

  A bare CLI binary cannot be granted it. Consent is attributed to a bundle
  identity, and the prompt's text comes from `NSAudioCaptureUsageDescription` in
  an Info.plist — neither of which a loose executable has. Run `./bundle.sh` to
  wrap a probe in a minimal .app that does, then run it from there.
""")

print("\n--- Microphone consent (does not prompt) ---")
switch AVCaptureDevice.authorizationStatus(for: .audio) {
case .authorized:
    print("microphone                 authorized")
case .notDetermined:
    print("microphone                 not yet asked — the first capture will prompt")
case .denied:
    print("microphone                 DENIED — mic probes will return silence, not an error")
case .restricted:
    print("microphone                 restricted by policy")
@unknown default:
    print("microphone                 unknown status")
}

print("""

  Note what this does and does not tell you. A bare CLI binary has no bundle
  identity, so macOS attributes consent to the responsible process — your
  terminal — exactly as the capture harness documents. The status above is your
  terminal's, not Skia's, and Skia will have to ask for its own.
""")

// -- What is currently playing audio? ----------------------------------------
//
// The tap has to name what it is tapping, so this is the list a tap is built
// from. It is also the first honest answer to "can we capture only the meeting
// app": if the process list does not distinguish it, nothing downstream can.

print("\n--- Processes Core Audio knows about ---")

let processes = objectList(
    AudioObjectID(kAudioObjectSystemObject),
    kAudioHardwarePropertyProcessObjectList
)

/// Left-justify to `width`. `String(format:)` does not honour a field width for
/// `%@`, so the columns have to be padded here or the table arrives unaligned.
func pad(_ text: String, _ width: Int) -> String {
    text.count >= width ? text : text + String(repeating: " ", count: width - text.count)
}

if processes.isEmpty {
    print("none reported. Either this OS has no process object list, or nothing")
    print("has touched audio since boot. Play something and re-run.")
} else {
    print(pad("pid", 8) + pad("bundle id", 40) + pad("output", 8) + "input")
    var active = 0
    for process in processes {
        let pid: pid_t = property(process, kAudioProcessPropertyPID) ?? -1
        let bundle = stringProperty(process, kAudioProcessPropertyBundleID) ?? "(none)"
        let running: UInt32 = property(process, kAudioProcessPropertyIsRunningOutput) ?? 0
        let capturing: UInt32 = property(process, kAudioProcessPropertyIsRunningInput) ?? 0
        // Only the ones actually making or taking sound are interesting; the
        // list otherwise includes every process that has ever opened audio.
        guard running != 0 || capturing != 0 else { continue }
        active += 1
        print(
            pad(String(pid), 8)
                + pad(bundle, 40)
                + pad(running != 0 ? "yes" : "-", 8)
                + (capturing != 0 ? "yes" : "-")
        )
    }
    if active == 0 {
        print("(nothing is playing or recording right now)")
    }
    print("\n\(active) of \(processes.count) process object(s) currently running")
    print("input or output. Start a call and re-run to see the app you mean to tap.")
}

print("""

=== What to do with this ===

  If CATapDescription is available, run loopback-probe next. It is the real
  question: whether a tap can be created and pulled without a Screen Recording
  prompt, which is the whole reason for preferring taps over ScreenCaptureKit
  in a meeting app.

  If it is not available, record that and stop. Nothing further in this harness
  will tell you anything about this machine.
""")
