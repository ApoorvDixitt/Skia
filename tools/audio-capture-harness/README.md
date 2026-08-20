# macOS audio-capture harness

> [!NOTE]
> **This is a diagnostic tool, not application code.** Nothing here is referenced by
> `Cargo.toml`, `tauri.conf.json`, or the Vite config, and none of it is compiled into or
> shipped with the app. Skia's own audio engine will be Rust; these probes are Swift because
> the question they answer is "what does this OS actually do", and the shortest path to that
> is the language Apple's own examples are written in. Answering it in hand-rolled Rust
> bindings first would mean debugging the bindings and the OS at the same time.
>
> It also contains **private API** (`TCCAccessPreflight`), which is acceptable in a harness
> you re-run per OS release and unacceptable in a shipped app. See
> [Consent](#consent-is-the-whole-story) for why it is unavoidable here.

Measures whether Skia can hear the far end of a call on the macOS version you are running,
and what it costs to ask.

This exists because the architecture doc had it backwards. It named ScreenCaptureKit as the
default path for system audio, with Core Audio process taps "worth evaluating as a cleaner
replacement". For a meeting assistant that ordering is wrong: SCK requires **Screen
Recording** permission, and asking someone for permission to record their screen so you can
hear a call is the prompt that gets an app deleted. The tap's consent story is the reason to
prefer it — so the thing to measure is whether a tap works at all.

## What has been measured

On **macOS 26.5** (Darwin 25.5.0), Apple Silicon, MacBook Air:

| Question | Result |
|---|---|
| Is `CATapDescription` available? | **Yes** (macOS 14.2+) |
| Does `AudioHardwareCreateProcessTap` succeed? | **Yes** |
| Does a private aggregate device wrap the tap? | **Yes**, `kAudioAggregateDeviceTapListKey` with no sub-devices |
| What format does the tap report? | **48 000 Hz, 2 channels, 32-bit float** |
| Do IO callbacks arrive at real time? | **Yes.** 375 callbacks, 4.000 s captured in 4.0 s wall clock |
| Longest gap between callbacks | **10.8 ms** (≈512 frames at 48 kHz) |
| Is Screen Recording permission involved? | **No — at no point.** This is the finding that matters |
| Is there audio in the samples? | **Not yet measured.** See below |

**The decisive result is the negative one.** A process tap captures far-end audio without
ever mentioning screen recording. That answers the architecture doc's open question — the tap
is not a "cleaner replacement" to evaluate later, it is the correct default, and
ScreenCaptureKit is the fallback for macOS before 14.2.

### What is still open

Sample values were **all zero**, on every run, including with audio definitely playing. That
is not a tap failure. It is missing consent, and it is the trap this harness exists to expose.

## Consent is the whole story

**A process tap without audio-capture consent does not fail. It succeeds and returns
silence.** Measured: 281 callbacks pacing at 99.9 % of real time, peak amplitude 0.0000, with
`afplay` audibly playing throughout. There is no error to catch and no status to check.

That is the same shape as the trap in the [capture harness](../macos-capture-harness) — where
legacy CoreGraphics capture degrades quietly instead of throwing — and it fails the same way:
it is very easy to look at a clean run and conclude the tap works.

Apple ships **no public API** to check or request this permission — and, measured later the
same day, **no capture path triggers the prompt implicitly either**. Not the tap, and not
even the microphone via the HAL: Skia's own shipped build recorded five seconds of exact
zeros, prompt-free, with `NSMicrophoneUsageDescription` correctly in place. Apple's
AVFoundation documentation states the auto-prompt belongs to `AVCaptureDeviceInput` creation
alone; everything else *"will vend silent audio samples"* until access is granted. So consent
must be **requested explicitly**: the microphone has a public API
(`AVCaptureDevice.requestAccess`, which Skia now calls), while audio capture has only TCC's
private SPI — which the probes here now use at startup, exactly as
[AudioCap](https://github.com/insidegui/AudioCap) does. AudioCap is the reference
implementation for this API and, in practice, its only real documentation. `tap-preflight`
reads state through the same SPI.

The SPI reading is **cross-checked against a public API** rather than trusted: for
`kTCCServiceScreenCapture`, `TCCAccessPreflight` returned `2` while the public
`CGPreflightScreenCaptureAccess()` returned `false`. Two independent instruments, same
answer, so `2` means "no grant".

### A loose binary can never be granted it

Consent is attributed to a **bundle identity**, and the prompt text comes from an
`Info.plist`. A bare executable has neither, so it is not merely ungranted — it is
unaskable. Compiling a probe and running it directly will "work", report clean timings, and
tell you nothing about audio.

`bundle.sh` exists for this. It wraps a probe in a minimal signed `.app` carrying
`NSAudioCaptureUsageDescription`, which is the only configuration that can prompt.

## The files

| File | What it does |
|---|---|
| `tap-preflight.swift` | Reports macOS version, tap API availability, TCC audio-capture and microphone state, and which processes Core Audio currently sees playing or recording. Creates no tap and captures nothing, so it cannot provoke the prompt before you are watching. **Run this first.** |
| `bundle.sh` | Wraps a probe in `build/Skia Audio Probe.app` with the `Info.plist` keys consent requires. Required for any probe that captures audio. |
| `loopback-probe.swift` | Creates a process tap, wraps it in a private aggregate device, pulls audio, reports format, callback timing, dropouts and peak amplitude, and writes `loopback.wav`. Diagnoses all-zero samples as missing consent rather than as missing audio. |
| `dual-probe.swift` | Captures microphone and far end **simultaneously** and measures how much of the far end leaked into the mic, by envelope cross-correlation. Writes `mic.wav` and `farend.wav`. This is where the echo-cancellation decision gets made. |
| `hotswap-probe.swift` | Watches default-device and device-list notifications during a live mic stream, and reports how many arrive per physical action and how close together. Sizes the debounce window. |

## Running it

```bash
swiftc -O tap-preflight.swift -o tap-preflight && ./tap-preflight
```

Then, for anything that captures audio — **via a bundle, or the result is meaningless**:

```bash
./bundle.sh loopback-probe.swift
"build/Skia Audio Probe.app/Contents/MacOS/probe" 10 --exclude-self
```

`loopback-probe` takes a duration in seconds, plus:

- `--exclude-self` — tap everything **except** this process. This is the shape Skia needs: the
  app must hear the far end without hearing its own spoken answers, or a generated answer
  feeds straight back into the transcript.
- `--pid N` — tap exactly one process. The narrowest possible tap, and both a better
  transcript and a smaller privacy claim than tapping everything.

Recompiling invalidates the ad-hoc signature, so expect to grant permission again after each
change. That is deliberate: a stale grant would let a broken probe look like a working one.

## The echo measurement, and why it is a correctness test

`docs/ARCHITECTURE.md` keeps the microphone and the far end separate all the way to
transcription, because that separation is what makes speaker labelling possible, and it
forbids OS echo cancellation on the loopback because that would strip the signal.

Both of those are right, and together they create a problem. If the speakers are audible to
the microphone, the far end arrives **twice** — once cleanly through the tap, once delayed and
distorted through the mic. Two transcribers then produce two overlapping transcripts of the
same sentence, and the speaker labels are wrong in the worst possible direction: the far end's
words attributed to the user.

So either run acoustic echo cancellation keyed to the loopback stream, or make headphones a
hard prerequisite. `dual-probe` measures the leak so that decision is made from a number.

It correlates **amplitude envelopes**, not waveforms. The leaked copy has been through a
speaker, a room, and a microphone, so its phase bears little relation to the original even
when it is obviously the same speech; the envelope survives all of that. The result is a
measure of "is the far end audible in the mic", which is the actual question, rather than a
measure of playback fidelity.

Run it **twice** — once on speakers, once on headphones — and record the correlation, the lag,
and which output device was in use. One number without the other two is not a result.

## Traps

**Silence is not an error.** Covered above, and it is the one that will waste your afternoon.

**Do not run capture probes as loose binaries.** They cannot hold consent. `bundle.sh` or
nothing.

**`LSBackgroundOnly` suppresses the prompt.** A background-only app cannot present UI,
including a TCC dialog, so the bundle uses `LSUIElement` instead. The failure mode if you get
this wrong is "macOS never asks", which reads like an OS bug.

**Music flatters the echo measurement.** Speech and music have different envelope statistics.
Use speech, because speech is what a meeting contains.

**Your own voice is noise in the echo measurement.** The question is how much of the *far
end* the microphone hears. Say nothing while it runs.

**One hot-swap run proves nothing.** Wired headphones, Bluetooth headsets, and external
interfaces behave nothing alike. Bluetooth is the case that matters, because an A2DP↔HFP
profile switch presents as several notifications in quick succession, and rebuilding the
stream once per notification means several transcript gaps for what the user experienced as
one action.

## Re-run this per macOS release

Same standing instruction as the capture harness. Tap behaviour is version-dependent, and
`sharingType = .none` next door is a reminder of how much can quietly change in a point
release.
