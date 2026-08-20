// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0
//
// Captures the microphone and the far end at the same time, then measures how
// much of the far end leaked into the microphone.
//
// This probe exists to settle one decision, and it is a correctness decision
// rather than a polish one. `docs/ARCHITECTURE.md` keeps the two streams
// separate all the way to transcription, because that separation is what makes
// speaker labelling possible, and it forbids OS echo cancellation on the
// loopback. But if the speakers are audible to the microphone, the far end
// arrives twice — once cleanly through the tap and once, delayed and distorted,
// through the mic. Two transcribers then produce two overlapping transcripts of
// the same sentence and the speaker labels are wrong in the worst possible way:
// the far end's words attributed to the user.
//
// So: either run acoustic echo cancellation keyed to the loopback, or require
// headphones. That is not a decision to take on intuition. Run this twice, once
// on speakers and once on headphones, and read the number.
//
// Usage:
//   swiftc -O dual-probe.swift -o dual-probe
//   ./dual-probe [seconds]
//
// Writes mic.wav and farend.wav.

import AVFoundation
import CoreAudio
import Foundation

let seconds = CommandLine.arguments.dropFirst().first.flatMap(Double.init) ?? 15.0

guard #available(macOS 14.2, *) else {
    print("This OS predates macOS 14.2, so there is no process tap API.")
    exit(1)
}

// MARK: - Consent, requested explicitly for both streams
//
// Neither stream triggers a consent dialog on its own — measured, not assumed.
// The tap vends zeros without the audio-capture grant, and AVAudioEngine's
// input reaches the mic through the HAL, which per Apple's own documentation
// only auto-prompts when an `AVCaptureDeviceInput` is created (this probe
// creates none). A leak measurement over two silent streams would correlate
// perfectly and mean nothing, so both grants are secured — or their absence
// reported — before anything is captured.

/// Microphone consent has a public API; use it.
func ensureMicConsent() -> Bool {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized:
        print("microphone consent         already authorized")
        return true
    case .notDetermined:
        print("microphone consent         requesting — answer the dialog…")
        let semaphore = DispatchSemaphore(value: 0)
        var granted = false
        AVCaptureDevice.requestAccess(for: .audio) { ok in
            granted = ok
            semaphore.signal()
        }
        _ = semaphore.wait(timeout: .now() + 300)
        print("microphone consent         \(granted ? "granted" : "DENIED — the mic track will be zeros")")
        return granted
    default:
        print("microphone consent         DENIED — grant it in System Settings and re-run")
        return false
    }
}

/// Audio-capture consent has no public request API; the TCC SPI is the only
/// route, as AudioCap documents. Diagnostic-tool-only code.
typealias TCCPreflightFn = @convention(c) (CFString, CFDictionary?) -> Int
typealias TCCRequestFn = @convention(c) (CFString, CFDictionary?, @escaping (Bool) -> Void) -> Void

func ensureAudioCaptureConsent() -> Bool {
    guard
        let handle = dlopen("/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC", RTLD_NOW),
        let preflightSymbol = dlsym(handle, "TCCAccessPreflight"),
        let requestSymbol = dlsym(handle, "TCCAccessRequest")
    else {
        print("audio-capture consent      TCC SPI unavailable — far end may be silent")
        return false
    }
    let preflight = unsafeBitCast(preflightSymbol, to: TCCPreflightFn.self)
    if preflight("kTCCServiceAudioCapture" as CFString, nil) == 0 {
        print("audio-capture consent      already authorized")
        return true
    }
    print("audio-capture consent      requesting — answer the dialog if one appears…")
    let request = unsafeBitCast(requestSymbol, to: TCCRequestFn.self)
    let semaphore = DispatchSemaphore(value: 0)
    var granted = false
    request("kTCCServiceAudioCapture" as CFString, nil) { ok in
        granted = ok
        semaphore.signal()
    }
    _ = semaphore.wait(timeout: .now() + 300)
    print("audio-capture consent      \(granted ? "granted" : "NOT granted — the far end will be zeros")")
    return granted
}

_ = ensureMicConsent()
_ = ensureAudioCaptureConsent()

// MARK: - Core Audio helpers

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

/// A mono recording plus the wall-clock time its first sample arrived.
///
/// The timestamps are what make the two streams comparable: they start
/// independently, so the correlation below has to know how far apart their first
/// samples were or it would report that offset as acoustic delay.
final class Track {
    var samples: [Float] = []
    var rate: Double = 0
    var firstArrival: Double? = nil
    var peak: Float = 0

    func append(_ value: Float) {
        samples.append(value)
        let magnitude = abs(value)
        if magnitude > peak { peak = magnitude }
    }
}

nonisolated(unsafe) let mic = Track()
nonisolated(unsafe) let farEnd = Track()

print("=== Skia dual-stream probe ===\n")

// MARK: - Far end, via a process tap

let description = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
description.isPrivate = true
description.name = "Skia dual probe"
description.uuid = UUID()

var tap = AudioObjectID(kAudioObjectUnknown)
guard AudioHardwareCreateProcessTap(description, &tap) == noErr, tap != kAudioObjectUnknown else {
    print("could not create the process tap. Run loopback-probe for the detail.")
    exit(1)
}
defer { AudioHardwareDestroyProcessTap(tap) }

let tapUID = description.uuid.uuidString

let aggregate: [String: Any] = [
    kAudioAggregateDeviceNameKey as String: "Skia dual probe",
    kAudioAggregateDeviceUIDKey as String: "dev.skia.harness.dual.\(getpid())",
    kAudioAggregateDeviceIsPrivateKey as String: true,
    kAudioAggregateDeviceSubDeviceListKey as String: [],
    kAudioAggregateDeviceTapListKey as String: [[kAudioSubTapUIDKey as String: tapUID]],
]

var device = AudioObjectID(kAudioObjectUnknown)
guard AudioHardwareCreateAggregateDevice(aggregate as CFDictionary, &device) == noErr,
      device != kAudioObjectUnknown
else {
    print("could not create the aggregate device that exposes the tap.")
    exit(1)
}
defer { AudioHardwareDestroyAggregateDevice(device) }

var tapFormat = AudioStreamBasicDescription()
var formatAddress = AudioObjectPropertyAddress(
    mSelector: kAudioTapPropertyFormat,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain
)
var formatSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
_ = AudioObjectGetPropertyData(tap, &formatAddress, 0, nil, &formatSize, &tapFormat)
farEnd.rate = tapFormat.mSampleRate > 0 ? tapFormat.mSampleRate : 48_000
let tapChannels = max(Int(tapFormat.mChannelsPerFrame), 1)

var procID: AudioDeviceIOProcID? = nil
guard AudioDeviceCreateIOProcIDWithBlock(&procID, device, nil, { _, inputData, _, _, _ in
    if farEnd.firstArrival == nil { farEnd.firstArrival = CFAbsoluteTimeGetCurrent() }
    let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inputData))
    for buffer in buffers {
        guard let raw = buffer.mData else { continue }
        let count = Int(buffer.mDataByteSize) / MemoryLayout<Float>.size
        let floats = raw.bindMemory(to: Float.self, capacity: count)
        // Downmixed to mono. The correlation below needs one signal per stream,
        // and stereo detail buys nothing for a transcript either.
        var index = 0
        while index + tapChannels <= count {
            var sum: Float = 0
            for channel in 0..<tapChannels { sum += floats[index + channel] }
            farEnd.append(sum / Float(tapChannels))
            index += tapChannels
        }
    }
}) == noErr, let procID else {
    print("could not install the far-end IO callback.")
    exit(1)
}
defer { AudioDeviceDestroyIOProcID(device, procID) }

// MARK: - Microphone, via AVAudioEngine

// AVAudioEngine rather than a second raw HAL device: the mic path is not what is
// under test here, and the engine's input node already handles device selection
// and format conversion. WS1 will use `cpal` for this in the app; the point of
// this probe is the relationship between the two streams, not the mic plumbing.
let engine = AVAudioEngine()
let input = engine.inputNode
let micFormat = input.outputFormat(forBus: 0)
mic.rate = micFormat.sampleRate

guard mic.rate > 0 else {
    print("the input node reports a sample rate of 0, which means no usable input device.")
    print("Check that a microphone exists and that consent was granted.")
    exit(1)
}

input.installTap(onBus: 0, bufferSize: 1024, format: micFormat) { buffer, _ in
    if mic.firstArrival == nil { mic.firstArrival = CFAbsoluteTimeGetCurrent() }
    guard let channels = buffer.floatChannelData else { return }
    let frames = Int(buffer.frameLength)
    let count = Int(buffer.format.channelCount)
    for frame in 0..<frames {
        var sum: Float = 0
        for channel in 0..<count { sum += channels[channel][frame] }
        mic.append(sum / Float(count))
    }
}

print(String(format: "microphone                 %.0f Hz, %u channel(s)", mic.rate, micFormat.channelCount))
print(String(format: "far end (tap)              %.0f Hz, %d channel(s)", farEnd.rate, tapChannels))

do {
    try engine.start()
} catch {
    print("\ncould not start the audio engine: \(error.localizedDescription)")
    print("If this is a consent failure, the dialog is the finding. Record it.")
    exit(1)
}

let startStatus = AudioDeviceStart(device, procID)
guard startStatus == noErr else {
    print("AudioDeviceStart failed: \(describe(startStatus))")
    exit(1)
}

print("""

--- capturing both streams for \(seconds)s ---

  Do this properly or the number below means nothing:

    1. Play speech through the far end — a real call, or a spoken-word recording.
       Music correlates differently from speech and will flatter the result.
    2. Say nothing yourself. The measurement is how much of the *far end* the
       microphone hears, and your own voice only adds noise to it.
    3. Run it once on speakers, then once on headphones, and compare.
""")

Thread.sleep(forTimeInterval: seconds)

AudioDeviceStop(device, procID)
engine.stop()
input.removeTap(onBus: 0)

// MARK: - Measure the leak

print("--- captured ---")
print(String(format: "microphone                 %.2fs, peak %.4f", Double(mic.samples.count) / mic.rate, mic.peak))
print(String(format: "far end                    %.2fs, peak %.4f", Double(farEnd.samples.count) / farEnd.rate, farEnd.peak))

if mic.peak < 1e-5 {
    print("""

      The microphone recorded silence. Either consent was denied — in which case
      macOS hands over silence rather than an error, which is exactly the trap
      the capture harness warns about — or the wrong input device is selected.
      Nothing below is meaningful. Fix this first.
    """)
}
if farEnd.peak < 1e-5 {
    print("""

      The far end recorded silence, so nothing was playing. The leak measurement
      needs far-end audio to leak. Re-run with a call or a recording playing.
    """)
}

/// Reduce a signal to a coarse amplitude envelope at `envelopeRate` Hz.
///
/// Correlating raw waveforms would be both expensive and fragile: the leaked
/// copy is filtered by the speaker, the room, and the mic, so its phase bears
/// little relation to the original even when it is plainly the same speech. The
/// envelope — how loud, over time — survives all of that, which is what makes
/// this a robust measure of "is the far end audible in the mic" rather than a
/// measure of playback fidelity.
func envelope(_ samples: [Float], rate: Double, envelopeRate: Double = 200) -> [Float] {
    guard rate > 0, !samples.isEmpty else { return [] }
    let window = max(Int(rate / envelopeRate), 1)
    var out: [Float] = []
    out.reserveCapacity(samples.count / window + 1)
    var index = 0
    while index < samples.count {
        let end = min(index + window, samples.count)
        var sum: Float = 0
        for position in index..<end { sum += abs(samples[position]) }
        out.append(sum / Float(end - index))
        index = end
    }
    return out
}

/// Pearson correlation of two equal-length slices, or 0 if either is flat.
func correlation(_ a: ArraySlice<Float>, _ b: ArraySlice<Float>) -> Double {
    let count = min(a.count, b.count)
    guard count > 1 else { return 0 }
    let first = Array(a.prefix(count))
    let second = Array(b.prefix(count))

    let meanA = first.reduce(0, +) / Float(count)
    let meanB = second.reduce(0, +) / Float(count)

    var covariance = 0.0
    var varianceA = 0.0
    var varianceB = 0.0
    for index in 0..<count {
        let deltaA = Double(first[index] - meanA)
        let deltaB = Double(second[index] - meanB)
        covariance += deltaA * deltaB
        varianceA += deltaA * deltaA
        varianceB += deltaB * deltaB
    }
    guard varianceA > 0, varianceB > 0 else { return 0 }
    return covariance / (varianceA * varianceB).squareRoot()
}

let envelopeRate = 200.0
let micEnvelope = envelope(mic.samples, rate: mic.rate, envelopeRate: envelopeRate)
let farEnvelope = envelope(farEnd.samples, rate: farEnd.rate, envelopeRate: envelopeRate)

if mic.peak >= 1e-5, farEnd.peak >= 1e-5, micEnvelope.count > 40, farEnvelope.count > 40 {
    // Search a lag window wide enough for speaker-to-mic acoustic delay plus
    // whatever offset the two streams started with. 500 ms is generous; real
    // acoustic delay in a room is a few milliseconds, and the rest is start skew.
    let maxLag = Int(0.5 * envelopeRate)
    var best = (correlation: 0.0, lag: 0)

    for lag in -maxLag...maxLag {
        let micSlice: ArraySlice<Float>
        let farSlice: ArraySlice<Float>
        if lag >= 0 {
            guard lag < micEnvelope.count else { continue }
            micSlice = micEnvelope[lag...]
            farSlice = farEnvelope[...]
        } else {
            guard -lag < farEnvelope.count else { continue }
            micSlice = micEnvelope[...]
            farSlice = farEnvelope[(-lag)...]
        }
        let value = correlation(micSlice, farSlice)
        if abs(value) > abs(best.correlation) { best = (value, lag) }
    }

    let lagMs = Double(best.lag) / envelopeRate * 1000
    print("\n--- leak measurement ---")
    print(String(format: "peak envelope correlation  %.3f", best.correlation))
    print(String(format: "at a lag of                %.0f ms", lagMs))

    // Thresholds are judgement, not physics, and they are written down here so
    // the next person can disagree with a number rather than with a vibe.
    print("")
    switch best.correlation {
    case 0.7...:
        print("VERDICT: the far end is plainly audible to the microphone.")
        print("")
        print("  Two transcribers will both hear the far end's speech and the")
        print("  speaker labels will be wrong — the far end's words attributed to")
        print("  the user. Echo cancellation keyed to the loopback stream is")
        print("  required, or headphones are a hard prerequisite rather than a")
        print("  recommendation. This is WS1.5 and it is not optional.")
    case 0.35..<0.7:
        print("VERDICT: partial leak.")
        print("")
        print("  Enough to corrupt a transcript during overlapping speech, not")
        print("  enough to duplicate every sentence. Worth cancelling, and worth")
        print("  re-measuring at a higher output volume before deciding — this is")
        print("  the regime where the answer depends on the room.")
    default:
        print("VERDICT: little or no leak at this volume.")
        print("")
        print("  Do not generalise from one run. Re-measure on speakers at a")
        print("  realistic call volume, on a laptop with no external audio, and")
        print("  with the mic gain the OS actually chose. A low number here with")
        print("  headphones on says nothing about the speaker case.")
    }

    print("""

      How to read the lag. A few milliseconds is speaker-to-microphone acoustic
      delay and means genuine echo. A large value is mostly the offset between
      the two streams' first samples, since they start independently — so treat
      the correlation as the finding and the lag as a sanity check on it.
    """)
} else {
    print("\n--- leak measurement skipped ---")
    print("Both streams need real audio in them. See above.")
}

// MARK: - Write both streams out

func write(_ track: Track, to name: String) {
    guard !track.samples.isEmpty, track.rate > 0 else {
        print("\(name): nothing captured, not written")
        return
    }
    let url = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        .appendingPathComponent(name)
    let settings: [String: Any] = [
        AVFormatIDKey: kAudioFormatLinearPCM,
        AVSampleRateKey: track.rate,
        AVNumberOfChannelsKey: 1,
        AVLinearPCMBitDepthKey: 32,
        AVLinearPCMIsFloatKey: true,
        AVLinearPCMIsBigEndianKey: false,
        AVLinearPCMIsNonInterleaved: false,
    ]
    guard let format = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: track.rate,
        channels: 1,
        interleaved: false
    ) else {
        print("\(name): could not describe the format")
        return
    }
    do {
        let file = try AVAudioFile(forWriting: url, settings: settings)
        guard let buffer = AVAudioPCMBuffer(
            pcmFormat: format,
            frameCapacity: AVAudioFrameCount(track.samples.count)
        ), let destination = buffer.floatChannelData?[0] else {
            print("\(name): could not allocate a buffer")
            return
        }
        buffer.frameLength = AVAudioFrameCount(track.samples.count)
        for index in 0..<track.samples.count { destination[index] = track.samples[index] }
        try file.write(from: buffer)
        print("wrote                      \(url.path)")
    } catch {
        print("\(name): \(error.localizedDescription)")
    }
}

print("")
write(mic, to: "mic.wav")
write(farEnd, to: "farend.wav")

print("""

=== What to record ===

  The correlation, the lag, and which output device was in use — speakers or
  headphones — for each run. One number without the other two is not a result.

  Then listen to mic.wav. If you can hear the far end in it, the number has a
  second, independent witness, and that is the standard the capture harness set
  for this project: measured, then confirmed by something that is not the same
  measurement.
""")
