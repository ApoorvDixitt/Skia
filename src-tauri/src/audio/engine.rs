// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The engine thread: owns the streams, survives their deaths, reports.
//!
//! One dedicated thread owns every `cpal::Stream`, because streams are not
//! `Send` and because `docs/ARCHITECTURE.md` requires real-time audio to be
//! isolated: a panic here is caught at the thread boundary and becomes a
//! status the UI can render, never a dead webview.
//!
//! The loop is a plain `recv_timeout` pump rather than an async runtime.
//! Audio arrives ~100 times a second, commands arrive when a human clicks;
//! at that cadence a blocking loop with a 25 ms tick is simpler to reason
//! about than executors, and simpler survives device flaps better.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::level::LevelWindow;
use super::mic::{self, FromStream, OpenMic};
use super::resample::{downmix, MonoResampler};
use super::swap::{SwapDetector, Verdict, DEBOUNCE_MS};
use super::wav;
use super::{
    lock_or_recover, AudioError, AudioStatus, Command, EngineEvent, EngineState, ProbeOutcome,
    TARGET_RATE_HZ,
};

/// How the loop paces itself: command wait, therefore worst-case latency from
/// "audio arrived" to "meter event emitted".
const LOOP_WAIT: Duration = Duration::from_millis(25);

/// How often device changes are checked and retries considered.
const TICK: Duration = Duration::from_millis(250);

/// Minimum spacing between attempts to reopen a stream that keeps failing —
/// without it, a missing device would be retried forty times a second and the
/// status panel would scroll with identical errors.
const REOPEN_BACKOFF: Duration = Duration::from_secs(1);

/// Probe lengths are clamped here, not errored: the numbers arrive from UI
/// controls, and a slider that can express an invalid value is the UI's bug,
/// not something to punish the user for.
const PROBE_SECONDS_MIN: f32 = 1.0;
const PROBE_SECONDS_MAX: f32 = 15.0;

/// The peak below which a recording is reported as silent. Matches the audio
/// harness: consent-denied capture on macOS measures exactly 0.0, and a real
/// room with a real microphone never does.
const SILENCE_PEAK: f32 = 1e-5;

// ---------------------------------------------------------------- handle ----

/// The rest of the app's view of the engine: cloneable-free, blocking, honest.
///
/// Every method sends a command and waits for that command's own reply
/// channel, with a timeout — so a dead engine surfaces as [`AudioError::EngineGone`]
/// rather than a hang, and no caller can ever receive another caller's reply.
pub struct Handle {
    tx: Mutex<mpsc::Sender<Command>>,
}

impl Handle {
    /// Start the engine thread. `emit` carries events to the frontend and must
    /// be callable from the engine thread, hence `Send + Sync`.
    pub fn spawn(emit: impl Fn(&EngineEvent) + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let emit: std::sync::Arc<dyn Fn(&EngineEvent) + Send + Sync> = std::sync::Arc::new(emit);

        let thread_emit = emit.clone();
        let spawned = std::thread::Builder::new()
            .name("skia-audio".to_string())
            .spawn(move || {
                // The supervision boundary. A panic in stream handling must
                // not take the process down (the webview lives there) and must
                // not vanish either: it becomes a status the UI renders.
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Engine::new(thread_emit.clone()).run(&rx);
                }));
                if let Err(panic) = run {
                    let detail = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "panicked without a message".to_string());
                    thread_emit(&EngineEvent::Status(AudioStatus {
                        state: EngineState::Idle,
                        device: None,
                        native_rate_hz: None,
                        native_channels: None,
                        rebuilds: 0,
                        last_error: Some(format!(
                            "the audio engine crashed and is no longer running: {detail}"
                        )),
                    }));
                }
            });
        if let Err(e) = spawned {
            // Out of threads at startup is not recoverable from here; the
            // handle will report EngineGone on every call, which is the truth.
            eprintln!("skia: could not start the audio engine thread: {e}");
        }

        Self { tx: Mutex::new(tx) }
    }

    fn send(&self, command: Command) -> Result<(), AudioError> {
        lock_or_recover(&self.tx)
            .send(command)
            .map_err(|_| AudioError::EngineGone)
    }

    /// Open the microphone for the level meter. Returns the resulting status.
    pub fn meter_start(&self) -> Result<AudioStatus, AudioError> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::MeterStart(reply))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| AudioError::EngineGone)?
    }

    pub fn meter_stop(&self) -> Result<AudioStatus, AudioError> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::MeterStop(reply))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| AudioError::EngineGone)?
    }

    /// Record `seconds` of 16 kHz mono to `path`. Blocks until done — call it
    /// from a blocking task, not an event loop.
    pub fn probe(&self, seconds: f32, path: PathBuf) -> Result<ProbeOutcome, AudioError> {
        let seconds = seconds.clamp(PROBE_SECONDS_MIN, PROBE_SECONDS_MAX);
        let (reply, rx) = mpsc::channel();
        self.send(Command::Probe {
            seconds,
            path,
            reply,
        })?;
        // Generous: recording time, plus rebuild retries, plus margin. The
        // engine's own deadline fires first and sends an error; this timeout
        // only matters if the engine died mid-probe.
        let wait = Duration::from_secs_f32(seconds * 2.0 + 15.0);
        rx.recv_timeout(wait).map_err(|_| AudioError::EngineGone)?
    }

    pub fn status(&self) -> Result<AudioStatus, AudioError> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::Status(reply))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| AudioError::EngineGone)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Best effort: the engine also exits when the channel disconnects.
        let _ = lock_or_recover(&self.tx).send(Command::Shutdown);
    }
}

// ---------------------------------------------------------------- engine ----

/// An open stream and its per-stream companions.
///
/// The channel receiver lives here because it is replaced together with the
/// stream on every rebuild: frames from a dead stream must never mingle with
/// frames from its replacement.
struct StreamBundle {
    mic: OpenMic,
    rx: mpsc::Receiver<FromStream>,
    level: LevelWindow,
}

/// A probe in progress.
struct ActiveProbe {
    recorder: ProbeRecorder,
    path: PathBuf,
    reply: mpsc::Sender<Result<ProbeOutcome, AudioError>>,
    /// When to give up: a stream that died and cannot be rebuilt must turn
    /// into an error the caller sees, not a recording that never returns.
    deadline: Instant,
}

struct Engine {
    emit: std::sync::Arc<dyn Fn(&EngineEvent) + Send + Sync>,
    stream: Option<StreamBundle>,
    meter_on: bool,
    probe: Option<ActiveProbe>,
    swap: SwapDetector,
    rebuilds: u32,
    last_error: Option<String>,
    last_open_attempt: Option<Instant>,
    started: Instant,
}

impl Engine {
    fn new(emit: std::sync::Arc<dyn Fn(&EngineEvent) + Send + Sync>) -> Self {
        Self {
            emit,
            stream: None,
            meter_on: false,
            probe: None,
            swap: SwapDetector::new(DEBOUNCE_MS),
            rebuilds: 0,
            last_error: None,
            last_open_attempt: None,
            started: Instant::now(),
        }
    }

    fn run(&mut self, commands: &mpsc::Receiver<Command>) {
        let mut last_tick = Instant::now();
        loop {
            match commands.recv_timeout(LOOP_WAIT) {
                Ok(Command::Shutdown) => break,
                // Every handle dropped: the app is shutting down.
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(command) => self.handle(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            self.pump();

            if last_tick.elapsed() >= TICK {
                last_tick = Instant::now();
                self.tick();
            }
        }
    }

    // ------------------------------------------------------------ commands --

    fn handle(&mut self, command: Command) {
        match command {
            Command::MeterStart(reply) => {
                let result = if self.stream.is_some() {
                    Ok(())
                } else {
                    self.open_stream()
                };
                let _ = reply.send(match result {
                    Ok(()) => {
                        self.meter_on = true;
                        self.emit_status();
                        Ok(self.snapshot())
                    }
                    Err(e) => Err(e),
                });
            }
            Command::MeterStop(reply) => {
                self.meter_on = false;
                // The stream stays if a probe still needs it.
                if self.probe.is_none() {
                    self.close_stream();
                }
                self.emit_status();
                let _ = reply.send(Ok(self.snapshot()));
            }
            Command::Probe {
                seconds,
                path,
                reply,
            } => {
                if self.probe.is_some() {
                    let _ = reply.send(Err(AudioError::Busy));
                    return;
                }
                if self.stream.is_none() {
                    if let Err(e) = self.open_stream() {
                        let _ = reply.send(Err(e));
                        return;
                    }
                }
                // The stream is open, so unwrap is justified — and if that
                // reasoning ever breaks, the catch_unwind boundary reports it.
                let rate = self.stream.as_ref().map(|b| b.mic.rate_hz).unwrap_or(0);
                match ProbeRecorder::new(rate, seconds) {
                    Ok(recorder) => {
                        self.probe = Some(ActiveProbe {
                            recorder,
                            path,
                            reply,
                            deadline: Instant::now()
                                + Duration::from_secs_f32(seconds * 2.0 + 10.0),
                        });
                        self.emit_status();
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::Status(reply) => {
                let _ = reply.send(self.snapshot());
            }
            // run() consumed Shutdown already; nothing else reaches here.
            Command::Shutdown => {}
        }
    }

    // -------------------------------------------------------------- audio --

    /// Drain whatever the stream callbacks sent since the last pass.
    fn pump(&mut self) {
        let Some(bundle) = &self.stream else { return };
        let channels = bundle.mic.channels;

        // Drained into a local first: processing a message can close or
        // replace the stream, which must not happen under the borrow.
        let mut messages = Vec::new();
        while let Ok(message) = bundle.rx.try_recv() {
            messages.push(message);
            // A backlog cap, so a starved loop catches up over a few passes
            // instead of stalling here while more audio piles in behind it.
            if messages.len() >= 64 {
                break;
            }
        }

        for message in messages {
            match message {
                FromStream::Frames(frames) => self.on_frames(&frames, channels),
                FromStream::Failed(detail) => self.on_stream_failed(detail),
            }
        }
    }

    fn on_frames(&mut self, interleaved: &[f32], channels: u16) {
        if self.stream.is_none() {
            // A Failed message earlier in the same batch closed the stream;
            // frames from the dead stream are not worth metering.
            return;
        }
        let mono = downmix(interleaved, channels);

        if let Some(bundle) = &mut self.stream {
            for update in bundle.level.push(&mono) {
                (self.emit)(&EngineEvent::Level(update));
            }
        }

        let done = match &mut self.probe {
            Some(active) => match active.recorder.push(&mono) {
                Ok(done) => done,
                Err(e) => {
                    self.fail_probe(e);
                    return;
                }
            },
            None => false,
        };
        if done {
            self.finish_probe();
        }
    }

    fn on_stream_failed(&mut self, detail: String) {
        // The OS's words, kept until a rebuild succeeds: "device disconnected"
        // in the status panel is what turns a mystery gap into "oh, my
        // headset".
        self.last_error = Some(detail);
        self.close_stream();
        self.emit_status();
        // tick() retries while anything still wants the stream.
    }

    // -------------------------------------------------------------- ticks --

    fn tick(&mut self) {
        // Device hot-swap, debounced. Only meaningful while a stream is open.
        if self.stream.is_some() {
            #[allow(clippy::cast_possible_truncation)]
            let now_ms = self.started.elapsed().as_millis() as u64;
            let default = mic::default_device_name();
            if let Verdict::Rebuild(device) = self.swap.observe(now_ms, default.as_deref()) {
                // The rate may differ on the new device; the recorder handles
                // that on reopen. Drop first so the OS releases the old one.
                self.close_stream();
                self.last_error = Some(format!("the default input moved to {device}"));
                // Fall through to the reopen below on this same tick.
            }
        }

        // Reopen while wanted, with backoff so a missing device is not
        // hammered — and not silently, either way.
        let wanted = self.meter_on || self.probe.is_some();
        if wanted && self.stream.is_none() {
            let due = self
                .last_open_attempt
                .is_none_or(|at| at.elapsed() >= REOPEN_BACKOFF);
            if due {
                self.last_open_attempt = Some(Instant::now());
                match self.open_stream() {
                    Ok(()) => {
                        self.rebuilds += 1;
                        self.last_error = None;
                        self.emit_status();
                    }
                    Err(e) => {
                        self.last_error = Some(e.to_string());
                        self.emit_status();
                    }
                }
            }
        }

        // A probe that cannot finish must fail while its caller still waits.
        if self
            .probe
            .as_ref()
            .is_some_and(|p| Instant::now() >= p.deadline)
        {
            let detail = self
                .last_error
                .clone()
                .unwrap_or_else(|| "the stream stopped delivering audio".to_string());
            self.fail_probe(AudioError::Stream { detail });
        }
    }

    // ------------------------------------------------------------- streams --

    fn open_stream(&mut self) -> Result<(), AudioError> {
        let (tx, rx) = mpsc::channel();
        let mic = mic::open_default(tx)?;

        // ~100 ms per meter reading, at whatever rate the hardware runs.
        let window = (mic.rate_hz / 10).max(1) as usize;
        self.swap.stream_opened(&mic.device_name);

        // A rebuild can land on a device with a different rate mid-probe; the
        // recorder resamples correctly either way, or the splice would play
        // at the wrong speed.
        if let Some(active) = &mut self.probe {
            if let Err(e) = active.recorder.set_input_rate(mic.rate_hz) {
                self.fail_probe(e);
            }
        }

        self.stream = Some(StreamBundle {
            mic,
            rx,
            level: LevelWindow::new(window),
        });
        Ok(())
    }

    fn close_stream(&mut self) {
        self.stream = None;
        self.swap.stream_closed();
    }

    // -------------------------------------------------------------- probes --

    fn finish_probe(&mut self) {
        let Some(active) = self.probe.take() else {
            return;
        };
        let _ = active
            .reply
            .send(finalise_probe(active.recorder, &active.path));
        if !self.meter_on {
            self.close_stream();
        }
        self.emit_status();
    }

    fn fail_probe(&mut self, error: AudioError) {
        if let Some(active) = self.probe.take() {
            let _ = active.reply.send(Err(error));
        }
        if !self.meter_on {
            self.close_stream();
        }
        self.emit_status();
    }

    // -------------------------------------------------------------- status --

    fn snapshot(&self) -> AudioStatus {
        let state = if self.probe.is_some() {
            EngineState::Recording
        } else if self.meter_on && self.stream.is_some() {
            EngineState::Listening
        } else {
            EngineState::Idle
        };
        AudioStatus {
            state,
            device: self.stream.as_ref().map(|b| b.mic.device_name.clone()),
            native_rate_hz: self.stream.as_ref().map(|b| b.mic.rate_hz),
            native_channels: self.stream.as_ref().map(|b| b.mic.channels),
            rebuilds: self.rebuilds,
            last_error: self.last_error.clone(),
        }
    }

    fn emit_status(&self) {
        (self.emit)(&EngineEvent::Status(self.snapshot()));
    }
}

// ------------------------------------------------------------------ probe ----

/// Accumulates a probe: native-rate mono in, exactly `target` frames of
/// 16 kHz out.
///
/// Separate from the engine so it can be tested with synthetic audio — the
/// engine's wiring needs hardware, but "five seconds in is five seconds out"
/// must not.
struct ProbeRecorder {
    /// `Option` so a mid-probe rate change can retire the old resampler
    /// (flushing its tail) and continue with a new one.
    resampler: Option<MonoResampler>,
    input_rate_hz: u32,
    out: Vec<f32>,
    target: usize,
}

impl ProbeRecorder {
    fn new(input_rate_hz: u32, seconds: f32) -> Result<Self, AudioError> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let target = (TARGET_RATE_HZ as f32 * seconds).round() as usize;
        Ok(Self {
            resampler: Some(MonoResampler::new(input_rate_hz)?),
            input_rate_hz,
            out: Vec::with_capacity(target + 4096),
            target,
        })
    }

    /// Feed native-rate mono. True once the target is reached.
    fn push(&mut self, mono: &[f32]) -> Result<bool, AudioError> {
        let Some(resampler) = &mut self.resampler else {
            return Ok(self.out.len() >= self.target);
        };
        let produced = resampler.push(mono)?;
        self.out.extend_from_slice(&produced);
        Ok(self.out.len() >= self.target)
    }

    /// The stream rebuilt onto a device running at `rate_hz`.
    ///
    /// The old resampler is flushed and retired: feeding 44.1 kHz frames into
    /// a resampler built for 48 kHz would splice audio at the wrong speed —
    /// subtly, which is the worst way.
    fn set_input_rate(&mut self, rate_hz: u32) -> Result<(), AudioError> {
        if rate_hz == self.input_rate_hz {
            return Ok(());
        }
        if let Some(old) = self.resampler.take() {
            self.out.extend_from_slice(&old.finish()?);
        }
        self.resampler = Some(MonoResampler::new(rate_hz)?);
        self.input_rate_hz = rate_hz;
        Ok(())
    }

    /// Exactly `target` frames: the tail past it is cut, a shortfall is an
    /// error at the call site, never padded with silence that would look like
    /// a working microphone that went quiet.
    fn take(mut self) -> Vec<f32> {
        self.out.truncate(self.target);
        self.out
    }
}

/// Write the recording and describe it — including the silence verdict.
fn finalise_probe(recorder: ProbeRecorder, path: &Path) -> Result<ProbeOutcome, AudioError> {
    let samples = recorder.take();
    let peak = samples.iter().fold(0.0f32, |max, s| max.max(s.abs()));

    wav::write_mono_16bit(path, TARGET_RATE_HZ, &samples)?;

    #[allow(clippy::cast_precision_loss)]
    Ok(ProbeOutcome {
        path: path.display().to_string(),
        seconds: samples.len() as f32 / TARGET_RATE_HZ as f32,
        sample_rate_hz: TARGET_RATE_HZ,
        peak,
        // The measured macOS failure mode: consent-denied capture returns
        // exact zeros at real-time pacing. Report it; never infer success
        // from a file having been written.
        silent: peak < SILENCE_PEAK,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_48k(seconds: f32) -> Vec<f32> {
        let frames = (48_000.0 * seconds) as usize;
        (0..frames)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect()
    }

    fn temp_wav(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("skia-engine-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_probe_fills_to_exactly_its_target_and_not_before() {
        let mut recorder = ProbeRecorder::new(48_000, 2.0).unwrap();
        // 1.5 s of input cannot fill a 2 s probe.
        assert!(!recorder.push(&sine_48k(1.5)).unwrap());
        // Another second crosses the line.
        assert!(recorder.push(&sine_48k(1.0)).unwrap());
        assert_eq!(recorder.take().len(), 32_000, "2 s at 16 kHz, exactly");
    }

    #[test]
    fn a_finished_probe_reports_its_audio_and_is_not_silent() {
        let mut recorder = ProbeRecorder::new(48_000, 1.0).unwrap();
        while !recorder.push(&sine_48k(0.25)).unwrap() {}
        let path = temp_wav("tone.wav");
        let outcome = finalise_probe(recorder, &path).unwrap();

        assert!((outcome.seconds - 1.0).abs() < 0.01);
        assert_eq!(outcome.sample_rate_hz, 16_000);
        assert!(
            (0.4..=0.6).contains(&outcome.peak),
            "a half-scale sine should peak near 0.5, got {}",
            outcome.peak
        );
        assert!(!outcome.silent);

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 32_000, "16 000 samples at 2 bytes each");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_silent_probe_says_so_instead_of_passing() {
        // The exact shape consent-denied capture produces: perfect timing,
        // all zeros.
        let mut recorder = ProbeRecorder::new(48_000, 1.0).unwrap();
        let zeros = vec![0.0f32; 48_000];
        while !recorder.push(&zeros).unwrap() {}
        let path = temp_wav("silent.wav");
        let outcome = finalise_probe(recorder, &path).unwrap();
        assert!(outcome.silent, "zeros must be reported, not shrugged at");
        assert_eq!(outcome.peak, 0.0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_mid_probe_rate_change_keeps_duration_honest() {
        // One second at 48 kHz, then the "device" swaps to 16 kHz. If the
        // recorder ignored the change, the remainder would be resampled as if
        // it were 48 kHz and play three times too fast — and the probe would
        // fill with the wrong amount of audio.
        let mut recorder = ProbeRecorder::new(48_000, 2.0).unwrap();
        assert!(!recorder.push(&sine_48k(1.0)).unwrap());

        recorder.set_input_rate(16_000).unwrap();
        // Quarter-second 16 kHz pieces until full. If the rate change were
        // mishandled, filling would take ~3× the pieces this bound allows.
        let piece: Vec<f32> = (0..4_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin() * 0.5)
            .collect();
        let mut pieces = 0;
        loop {
            pieces += 1;
            assert!(
                pieces <= 6,
                "2 s should fill within ~1.25 s of 16 kHz input"
            );
            if recorder.push(&piece).unwrap() {
                break;
            }
        }
        assert_eq!(recorder.take().len(), 32_000);
    }

    #[test]
    fn an_unchanged_rate_does_not_disturb_the_resampler() {
        let mut recorder = ProbeRecorder::new(48_000, 1.0).unwrap();
        assert!(!recorder.push(&sine_48k(0.5)).unwrap());
        recorder.set_input_rate(48_000).unwrap();
        assert!(recorder.push(&sine_48k(0.6)).unwrap());
    }
}
