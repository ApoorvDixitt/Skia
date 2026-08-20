// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The audio engine: microphone capture, resampling, and honest reporting.
//!
//! # What this is, and what it is not
//!
//! This is the **microphone half** of the dual-stream design in
//! `docs/ARCHITECTURE.md`. The far-end half — CoreAudio process taps on macOS,
//! WASAPI loopback on Windows — is separate work, and the two will stay
//! separate streams all the way to transcription when it lands, because that
//! separation is what makes speaker labelling possible.
//!
//! What exists here today: enumerate input devices, capture the default
//! microphone, downmix to mono and resample to 16 kHz (the rate every
//! transcription backend expects), meter the level so the UI can show that the
//! microphone is alive, and record a short WAV probe the user can listen to.
//! The probe is the point at this stage: a transcript can be plausible and
//! wrong, but the audio cannot, so "listen to what the pipeline heard" is the
//! test everything downstream builds on.
//!
//! # The engine is a thread, and the streams live on it
//!
//! `cpal::Stream` is not `Send`, so every stream is created, used, and dropped
//! on one dedicated engine thread. The rest of the app talks to that thread
//! through a channel via [`Handle`], and the thread supervises itself: a panic
//! is caught, reported as a status the UI renders, and kills only the engine —
//! never the webview. That is the isolation `docs/ARCHITECTURE.md` requires,
//! in its cheapest correct form.
//!
//! # Silence is a finding, not an absence
//!
//! The audio harness measured the trap this module must not fall into: capture
//! without consent does not fail on macOS, it *succeeds and returns zeros* —
//! real-time callbacks, peak amplitude 0.0000, no error anywhere. So a probe
//! that records silence says so explicitly ([`ProbeOutcome::silent`]) and the
//! UI treats it as a consent-or-device problem, never as a quiet room and
//! never as success. Same fail-closed reporting as `stealth.rs`: what actually
//! happened, not what was requested.
//!
//! # Device hot-swap
//!
//! People plug in headphones mid-call, and `docs/ARCHITECTURE.md` calls the
//! resulting rebuild the main crash risk. The engine polls the default device
//! and rebuilds its stream when the default *stays* changed for a debounce
//! window — see [`swap`] for why the window exists (Bluetooth profile flaps)
//! and the audio harness's `hotswap-probe` for how its length gets measured
//! rather than guessed.

mod consent;
mod engine;
mod level;
mod mic;
mod resample;
mod swap;
mod wav;

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Mutex;

use serde::Serialize;

pub use consent::ensure_microphone;
pub use engine::Handle;
pub use mic::list_devices;

/// The sample rate everything downstream of capture speaks, in Hz.
///
/// Every transcription backend in the plan — sherpa-onnx, Deepgram, Whisper —
/// wants 16 kHz mono, so conversion happens once, here, at the capture
/// boundary, and nothing after it ever asks what the hardware rate was.
pub const TARGET_RATE_HZ: u32 = 16_000;

/// Everything that can go wrong capturing audio.
///
/// Every variant says what to do about it or what it means, because the
/// audible symptom of most audio failures is nothing at all — no sound, no
/// crash — and an error that just says "failed" leaves the user staring at a
/// silent meter.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error(
        "no input device is available. Plug in or enable a microphone, then \
         open this section again"
    )]
    NoInputDevice,

    #[error("the input device could not be queried: {detail}")]
    Device { detail: String },

    #[error("the microphone stream could not be opened: {detail}")]
    Stream { detail: String },

    #[error("resampling to 16 kHz failed: {detail}")]
    Resample { detail: String },

    #[error("could not write {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error(
        "the audio engine is not running. It reported its last error in the \
         status panel; reopening the app restarts it"
    )]
    EngineGone,

    #[error("microphone access was not granted: {detail}")]
    MicAccessDenied { detail: String },

    #[error("a recording is already in progress; wait for it to finish")]
    Busy,
}

/// One input device, as shown in the dashboard's device list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    /// Whether this is the device the engine would capture from right now.
    pub is_default: bool,
    pub sample_rate_hz: u32,
    pub channels: u16,
}

/// What the engine is doing, for the status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineState {
    /// No stream is open. The microphone is untouched.
    Idle,
    /// A stream is open for the level meter only; nothing is stored.
    Listening,
    /// A probe is being recorded to disk.
    Recording,
}

/// A snapshot of the engine, always describing what is actually happening.
///
/// `rebuilds` and `last_error` are part of the surface on purpose: a stream
/// that died and was rebuilt is information the user can act on ("my Bluetooth
/// headset flapped"), and hiding it would make the meter's gap look like a bug
/// in whatever the user happened to be doing at the time.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub state: EngineState,
    /// The device the open stream captures from. `None` when idle.
    pub device: Option<String>,
    /// The rate the hardware delivers, before resampling. `None` when idle.
    pub native_rate_hz: Option<u32>,
    pub native_channels: Option<u16>,
    /// Streams rebuilt after a device change or failure, since launch.
    pub rebuilds: u32,
    /// The most recent stream failure, kept until a rebuild succeeds.
    pub last_error: Option<String>,
}

/// One level-meter reading, over roughly 100 ms of microphone signal.
///
/// Values are linear in [0, 1]. The UI converts to a dB-ish bar; the raw
/// numbers stay linear here so the tests can assert exact arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelUpdate {
    /// Root mean square over the window — perceived loudness.
    pub rms: f32,
    /// Largest absolute sample in the window.
    pub peak: f32,
    /// Whether the window touched full scale, i.e. the input is clipping.
    pub clipped: bool,
}

/// The result of recording a probe.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOutcome {
    /// Where the WAV was written. Absolute, so the user can find it.
    pub path: String,
    /// Duration actually recorded, in seconds of 16 kHz output.
    pub seconds: f32,
    /// Always [`TARGET_RATE_HZ`]; carried so the file is self-describing.
    pub sample_rate_hz: u32,
    /// Largest absolute sample in the recording.
    pub peak: f32,
    /// True when the whole recording is indistinguishable from zero.
    ///
    /// Reported rather than inferred by the caller, because the harness
    /// measured what silence means on macOS: capture without consent succeeds
    /// and returns zeros. A silent probe is a consent-or-device finding, and
    /// the UI must present it as one.
    pub silent: bool,
}

/// What the engine pushes to the frontend as it runs.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    Level(LevelUpdate),
    Status(AudioStatus),
}

/// Commands the [`Handle`] sends to the engine thread.
///
/// Each carries its own reply channel, so a caller blocks only on its own
/// command and a dead engine turns into a recv error rather than a hang.
enum Command {
    MeterStart(mpsc::Sender<Result<AudioStatus, AudioError>>),
    MeterStop(mpsc::Sender<Result<AudioStatus, AudioError>>),
    Probe {
        seconds: f32,
        path: PathBuf,
        reply: mpsc::Sender<Result<ProbeOutcome, AudioError>>,
    },
    Status(mpsc::Sender<AudioStatus>),
    Shutdown,
}

/// Lock a mutex, surviving a poisoned lock.
///
/// Same policy as `AppState` in `lib.rs`: the data behind these locks stays
/// consistent even if a panic interrupted a holder, and refusing all further
/// audio work because one call panicked would turn one bug into a dead engine.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
