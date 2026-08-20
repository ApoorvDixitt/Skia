// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0
//
// Captures far-end audio with a Core Audio process tap and reports what it
// actually got.
//
// This is the load-bearing probe. `docs/ARCHITECTURE.md` names ScreenCaptureKit
// as the default path for system audio and the process tap as "worth evaluating
// as a cleaner replacement". For a meeting assistant that framing is backwards:
// SCK requires Screen Recording permission, and asking a user for permission to
// record their screen so you can hear a call is the kind of prompt that gets an
// app deleted. The tap's consent story is the reason to prefer it, so the thing
// to measure is whether a tap can be created and pulled at all, and what it
// costs to ask.
//
// Usage:
//   swiftc -O loopback-probe.swift -o loopback-probe
//   ./loopback-probe [seconds] [--exclude-self] [--pid N]
//
// Writes loopback.wav (float32, whatever rate the tap reports).

import AVFoundation
import CoreAudio
import Foundation

// MARK: - Arguments

var seconds = 10.0
var excludeSelf = false
var targetPID: pid_t? = nil

var arguments = Array(CommandLine.arguments.dropFirst())
while let argument = arguments.first {
    arguments.removeFirst()
    switch argument {
    case "--exclude-self":
        excludeSelf = true
    case "--pid":
        guard let raw = arguments.first, let value = pid_t(raw) else {
            FileHandle.standardError.write(Data("--pid needs a process id\n".utf8))
            exit(2)
        }
        arguments.removeFirst()
        targetPID = value
    default:
        guard let value = Double(argument) else {
            FileHandle.standardError.write(Data("unrecognised argument: \(argument)\n".utf8))
            exit(2)
        }
        seconds = value
    }
}

guard #available(macOS 14.2, *) else {
    print("This OS predates macOS 14.2, so there is no process tap API. Run")
    print("tap-preflight and record that; nothing here applies.")
    exit(1)
}

// MARK: - Core Audio helpers

func property<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> T? {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
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
    var values = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &values) == noErr else {
        return []
    }
    return Array(values.prefix(Int(size) / MemoryLayout<AudioObjectID>.size))
}

/// Turn an OSStatus into the four-character code Core Audio actually documents,
/// because "-10851" is not something anyone can look up.
func describe(_ status: OSStatus) -> String {
    let value = UInt32(bitPattern: status)
    let bytes = [
        UInt8((value >> 24) & 0xFF), UInt8((value >> 16) & 0xFF),
        UInt8((value >> 8) & 0xFF), UInt8(value & 0xFF),
    ]
    if bytes.allSatisfy({ $0 >= 0x20 && $0 < 0x7F }) {
        return "\(status) '\(String(decoding: bytes, as: UTF8.self))'"
    }
    return "\(status)"
}

// MARK: - Build the tap

print("=== Skia loopback probe (Core Audio process tap) ===\n")

let system = AudioObjectID(kAudioObjectSystemObject)

/// Translate a pid into the Core Audio process object the tap API wants.
///
/// Uses `kAudioHardwarePropertyTranslatePIDToProcessObject`, which is the
/// documented route and the one AudioCap uses. Scanning
/// `kAudioHardwarePropertyProcessObjectList` and comparing
/// `kAudioProcessPropertyPID` also works and was what this probe did first, but
/// it only finds processes already in the list — so it misses one that has not
/// touched audio yet and reports "no such process" for something plainly running.
func processObject(for pid: pid_t) -> AudioObjectID? {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var input = pid
    var object = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    let status = AudioObjectGetPropertyData(
        system,
        &address,
        UInt32(MemoryLayout<pid_t>.size),
        &input,
        &size,
        &object
    )
    guard status == noErr, object != kAudioObjectUnknown else { return nil }
    return object
}

let description: CATapDescription
if let targetPID {
    guard let object = processObject(for: targetPID) else {
        print("no Core Audio process object for pid \(targetPID).")
        print("It has to have touched audio at least once. Run tap-preflight")
        print("while it is playing to get a list.")
        exit(1)
    }
    // A stereo mixdown of exactly one process: the narrowest possible tap, and
    // the shape a meeting app wants. Tapping only Zoom's audio is both a better
    // transcript and a smaller privacy claim than tapping everything.
    description = CATapDescription(stereoMixdownOfProcesses: [object])
    print("tap scope                  pid \(targetPID) only")
} else if excludeSelf {
    // A global tap that excludes this process. This is the one that matters for
    // Skia: the app has to hear the far end without hearing whatever it plays
    // back itself, or a spoken answer feeds straight into the transcript.
    let own = processObject(for: getpid())
    description = CATapDescription(stereoGlobalTapButExcludeProcesses: own.map { [$0] } ?? [])
    print("tap scope                  everything except this process"
        + (own == nil ? " (WARNING: own process object not found, so nothing was excluded)" : ""))
} else {
    description = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
    print("tap scope                  everything (pass --exclude-self to exclude this process)")
}

// Private: the tap and its aggregate device must not appear in the user's Sound
// settings or in other apps' device lists. A meeting assistant that litters the
// system with visible virtual devices is not one anyone trusts.
description.isPrivate = true
description.name = "Skia loopback probe"
// Assigned rather than left implicit, because this uuid is the handle the
// aggregate device references the tap by. Reading `kAudioTapPropertyUID` off the
// created tap works too, but the description's own uuid is what Apple's API
// expects and what AudioCap documents, and it is known before the tap exists.
description.uuid = UUID()

var tap = AudioObjectID(kAudioObjectUnknown)
let tapStatus = AudioHardwareCreateProcessTap(description, &tap)
guard tapStatus == noErr, tap != kAudioObjectUnknown else {
    print("\nAudioHardwareCreateProcessTap failed: \(describe(tapStatus))")
    print("""

      This is the answer the harness exists to get. Record the status code and
      whether any consent dialog appeared. A permission failure here means the
      tap path needs the same TCC grant as any other audio capture, which
      changes the onboarding flow but not the architecture.
    """)
    exit(1)
}
print("tap created                yes (object \(tap))")

defer { AudioHardwareDestroyProcessTap(tap) }

let tapUID = description.uuid.uuidString

// MARK: - Wrap the tap in a private aggregate device

// A tap is not a device and cannot be read directly. It is read by building an
// aggregate device whose tap list names it, which is the part of this API that
// is genuinely awkward and the main reason to measure it before committing.
let aggregateUID = "dev.skia.harness.loopback.\(getpid())"
let aggregateDescription: [String: Any] = [
    kAudioAggregateDeviceNameKey as String: "Skia loopback probe",
    kAudioAggregateDeviceUIDKey as String: aggregateUID,
    kAudioAggregateDeviceIsPrivateKey as String: true,
    // No sub-devices: this aggregate exists only to expose the tap, so it must
    // not pull in real hardware and must not become a selectable output.
    kAudioAggregateDeviceSubDeviceListKey as String: [],
    kAudioAggregateDeviceTapListKey as String: [
        [kAudioSubTapUIDKey as String: tapUID]
    ],
]

var device = AudioObjectID(kAudioObjectUnknown)
let deviceStatus = AudioHardwareCreateAggregateDevice(aggregateDescription as CFDictionary, &device)
guard deviceStatus == noErr, device != kAudioObjectUnknown else {
    print("AudioHardwareCreateAggregateDevice failed: \(describe(deviceStatus))")
    exit(1)
}
print("aggregate device           yes (object \(device))")
defer { AudioHardwareDestroyAggregateDevice(device) }

// MARK: - Read the format it will actually hand us

// Read from the tap itself via `kAudioTapPropertyFormat`, which is the
// documented source. The aggregate device's stream also carries a virtual format
// and agrees in practice, but it is a property of the wrapper rather than of the
// thing being measured.
var streamFormat = AudioStreamBasicDescription()
var formatAddress = AudioObjectPropertyAddress(
    mSelector: kAudioTapPropertyFormat,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain
)
var formatSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
if AudioObjectGetPropertyData(tap, &formatAddress, 0, nil, &formatSize, &streamFormat) == noErr {
    print(String(
        format: "tap format                 %.0f Hz, %u channel(s), %u bits",
        streamFormat.mSampleRate,
        streamFormat.mChannelsPerFrame,
        streamFormat.mBitsPerChannel
    ))
} else {
    print("tap format                 kAudioTapPropertyFormat could not be read")
}

print("""

  Note the rate. Skia resamples both streams to 16 kHz mono before
  transcription, so whatever this reports is the input side of that conversion,
  and a tap that changes rate mid-call is a stream rebuild, not a resample.
""")

// MARK: - Pull audio

/// Everything the IO callback touches. A class so the callback closure captures
/// one reference rather than a pile of mutable locals, and `nonisolated(unsafe)`
/// because the callback runs on a real-time thread that owns it exclusively for
/// the duration of the capture.
final class Capture {
    var samples: [Float] = []
    var channels = 0
    var callbacks = 0
    var silentCallbacks = 0
    var peak: Float = 0
    /// Largest gap between callbacks, in seconds. A dropout shows up here long
    /// before it is audible.
    var longestGap = 0.0
    var lastCallback: Double? = nil
}

nonisolated(unsafe) let capture = Capture()
capture.channels = Int(streamFormat.mChannelsPerFrame)

var procID: AudioDeviceIOProcID? = nil
let ioStatus = AudioDeviceCreateIOProcIDWithBlock(&procID, device, nil) {
    _, inputData, _, _, _ in
    let now = CFAbsoluteTimeGetCurrent()
    if let last = capture.lastCallback {
        capture.longestGap = max(capture.longestGap, now - last)
    }
    capture.lastCallback = now
    capture.callbacks += 1

    let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inputData))
    var sawSignal = false
    for buffer in buffers {
        guard let raw = buffer.mData else { continue }
        let count = Int(buffer.mDataByteSize) / MemoryLayout<Float>.size
        let floats = raw.bindMemory(to: Float.self, capacity: count)
        for index in 0..<count {
            let sample = floats[index]
            capture.samples.append(sample)
            let magnitude = abs(sample)
            if magnitude > capture.peak { capture.peak = magnitude }
            if magnitude > 1e-5 { sawSignal = true }
        }
    }
    if !sawSignal { capture.silentCallbacks += 1 }
}

guard ioStatus == noErr, let procID else {
    print("AudioDeviceCreateIOProcIDWithBlock failed: \(describe(ioStatus))")
    exit(1)
}
defer { AudioDeviceDestroyIOProcID(device, procID) }

let startStatus = AudioDeviceStart(device, procID)
guard startStatus == noErr else {
    print("AudioDeviceStart failed: \(describe(startStatus))")
    print("""

      If this is where it fails rather than at tap creation, the tap exists but
      cannot be pulled — which usually means consent. Record whether a dialog
      appeared and which one.
    """)
    exit(1)
}

print("\n--- capturing for \(seconds)s ---")
print("Play the far end now: start a call, or just play something.\n")
Thread.sleep(forTimeInterval: seconds)
AudioDeviceStop(device, procID)

// MARK: - Report

let frames = capture.channels > 0 ? capture.samples.count / capture.channels : 0
let rate = streamFormat.mSampleRate > 0 ? streamFormat.mSampleRate : 48_000
let captured = Double(frames) / rate

print("callbacks                  \(capture.callbacks)")
print(String(format: "captured                   %.3fs of audio in %.1fs of wall clock", captured, seconds))
print(String(format: "longest gap between them   %.1f ms", capture.longestGap * 1000))
print(String(format: "peak amplitude             %.4f", capture.peak))
print("silent callbacks           \(capture.silentCallbacks) of \(capture.callbacks)")

if capture.callbacks == 0 {
    print("""

      Nothing arrived at all. The tap and the device were created, so this is
      not a permission failure at creation time -- it is a tap that produces no
      audio, which is the worst of the possible outcomes because it looks like
      success. Record it.
    """)
} else if capture.peak < 1e-5 {
    // The signature that matters. Callbacks pacing at real time while every
    // sample is zero is what a tap without audio-capture consent looks like --
    // it does not fail, it succeeds and returns nothing. Measured on macOS 26.5
    // as 281 callbacks and 2.997s captured in 3.0s at peak 0.0000, with audio
    // definitely playing. So this must never be reported as "probably nothing
    // was playing".
    let ratio = captured / seconds
    print("""

      Callbacks arrived at \(String(format: "%.0f%%", ratio * 100)) of real time and every sample was zero.

      That is most likely MISSING CONSENT, not missing audio. A process tap
      without the audio-capture grant is created successfully, wraps in an
      aggregate device successfully, and hands back silence. There is no error to
      catch.

      Distinguish the two cases before recording anything:

        1. Run tap-preflight. If kTCCServiceAudioCapture reports anything other
           than authorized, that is the answer.
        2. If you ran this as a loose binary, it cannot hold the grant at all.
           Wrap it: ./bundle.sh loopback-probe.swift, then run the binary inside
           the bundle. Only that version can prompt.
        3. Only once consent is authorized is "nothing was playing" worth
           considering -- and then play something audible and re-run.
    """)
}

// Frames arriving materially slower than real time means the capture cannot keep
// up, which for a live meeting is a correctness failure rather than a slow path.
if captured > 0, captured < seconds * 0.95 {
    print(String(
        format: "\n  WARNING: %.1f%% of real time captured. Frames are being dropped.",
        captured / seconds * 100
    ))
}

// MARK: - Write it out

let url = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
    .appendingPathComponent("loopback.wav")

if frames > 0, capture.channels > 0 {
    guard let format = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: rate,
        channels: AVAudioChannelCount(capture.channels),
        interleaved: false
    ) else {
        print("\ncould not describe the captured format, so nothing was written")
        exit(1)
    }

    // Written interleaved even though the buffer above is not. A WAV file is
    // interleaved by definition, and handing AVAudioFile `format.settings` from
    // a non-interleaved format makes it log "Audio files cannot be
    // non-interleaved" and carry on — a warning in a diagnostic tool is worse
    // than in an app, because the next person cannot tell it from a real finding.
    let fileSettings: [String: Any] = [
        AVFormatIDKey: kAudioFormatLinearPCM,
        AVSampleRateKey: rate,
        AVNumberOfChannelsKey: capture.channels,
        AVLinearPCMBitDepthKey: 32,
        AVLinearPCMIsFloatKey: true,
        AVLinearPCMIsBigEndianKey: false,
        AVLinearPCMIsNonInterleaved: false,
    ]

    do {
        let file = try AVAudioFile(forWriting: url, settings: fileSettings)
        guard let buffer = AVAudioPCMBuffer(
            pcmFormat: format,
            frameCapacity: AVAudioFrameCount(frames)
        ) else {
            print("\ncould not allocate a buffer for \(frames) frames")
            exit(1)
        }
        buffer.frameLength = AVAudioFrameCount(frames)
        // De-interleave: the callback appended frames as they arrived, which is
        // channel-interleaved, and AVAudioPCMBuffer here is not.
        for channel in 0..<capture.channels {
            guard let destination = buffer.floatChannelData?[channel] else { continue }
            for frame in 0..<frames {
                destination[frame] = capture.samples[frame * capture.channels + channel]
            }
        }
        try file.write(from: buffer)
        print("\nwrote                      \(url.path)")
        print("Listen to it. A transcript can be plausible and wrong; the audio cannot.")
    } catch {
        print("\ncould not write \(url.path): \(error.localizedDescription)")
    }
} else {
    print("\nnothing captured, so no file was written")
}

print("""

=== What to record ===

  1. Did any permission dialog appear, and what did it say? This is the whole
     reason for preferring a tap over ScreenCaptureKit.
  2. Did the tap and the aggregate device get created, and at what format?
  3. Was there audio, and is loopback.wav actually the far end?
  4. Did --exclude-self keep this process's own output out? Run it again while
     this process plays something to check, or the exclusion is only assumed.

  Then run dual-probe, which is where the echo-cancellation decision gets made.
""")
