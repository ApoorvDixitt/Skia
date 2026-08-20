// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Downmix to mono and resample to 16 kHz.
//!
//! This is the one place sample rates are converted. The hardware delivers
//! whatever it delivers — 44.1 kHz, 48 kHz, sometimes 96 — and everything
//! downstream of capture speaks 16 kHz mono, because that is what every
//! transcription backend in the plan expects. Doing the conversion once at the
//! capture boundary means nothing later ever branches on the hardware rate.
//!
//! Resampling is rubato's sinc interpolation with anti-aliasing, not linear
//! interpolation, because the output feeds a speech recogniser: aliasing folds
//! high frequencies down into the speech band as noise that no model was
//! trained on. The parameters below are rubato's own example settings for
//! exactly this fixed-ratio case.
//!
//! Downmix averages the channels. A meeting microphone is one voice arriving
//! on one or two capsules, not a mix where a channel carries something unique;
//! averaging keeps the level independent of the channel count.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

use super::{AudioError, TARGET_RATE_HZ};

/// Frames fed to the resampler per processing call.
///
/// At 48 kHz this is ~21 ms of audio, which keeps the added latency well under
/// the level meter's own 100 ms window. Smaller chunks cost more sinc overlap
/// work per output sample; larger ones add latency for no quality gain.
const CHUNK_FRAMES: usize = 1024;

/// Interleaved frames → mono, by averaging each frame's channels.
///
/// A trailing partial frame — fewer samples than `channels` — is dropped
/// rather than padded: it can only exist if the platform delivered a torn
/// buffer, and inventing a zero sample would put a click in the audio.
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// A streaming mono resampler from a fixed input rate to [`TARGET_RATE_HZ`].
///
/// Feed it arbitrary-length pieces with [`push`](Self::push); it buffers
/// internally and processes in [`CHUNK_FRAMES`] blocks, so callers never care
/// what block size the underlying resampler wants. [`finish`](Self::finish)
/// drains the tail.
pub struct MonoResampler {
    inner: Async<f32>,
    /// Mono frames waiting to fill the next fixed-size input chunk.
    pending: Vec<f32>,
    /// Scratch for the resampler to write into, sized once at construction.
    out_scratch: Vec<f32>,
    /// Output frames over input frames — 1/3 for 48 kHz in.
    ratio: f64,
    /// Real input frames accepted so far. Excludes the silence padding that
    /// partial chunks and the final flush feed the filter.
    input_frames: u64,
    /// Output frames handed to the caller so far, across push and finish —
    /// the number [`finish`](Self::finish) reconciles against the target.
    emitted: u64,
    /// Filter-delay frames still to be dropped from the front of the output.
    ///
    /// A sinc filter cannot see the future, so its output starts with
    /// `output_delay()` frames of transient before the first real sample.
    /// Passing that through would prepend silence to every recording and make
    /// one second of input come out longer than one second — the tests
    /// caught exactly that.
    skip: usize,
}

impl MonoResampler {
    pub fn new(input_rate_hz: u32) -> Result<Self, AudioError> {
        if input_rate_hz == 0 {
            return Err(AudioError::Resample {
                detail: "the input rate is 0 Hz, which is not a rate".to_string(),
            });
        }

        // rubato's ratio is output over input: 1/3 for 48 kHz → 16 kHz.
        let ratio = f64::from(TARGET_RATE_HZ) / f64::from(input_rate_hz);

        // The example parameters rubato ships for the fixed-ratio case. The
        // ratio never changes after construction (1.0 relative), so the sinc
        // filter bank is computed once.
        let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
            .oversampling_factor(256)
            .interpolation(SincInterpolationType::Quadratic);

        let inner = Async::<f32>::new_sinc(ratio, 1.0, &params, CHUNK_FRAMES, 1, FixedAsync::Input)
            .map_err(|e| AudioError::Resample {
                detail: e.to_string(),
            })?;

        let out_scratch = vec![0.0; inner.output_frames_max()];
        let skip = inner.output_delay();

        Ok(Self {
            inner,
            pending: Vec::with_capacity(CHUNK_FRAMES * 2),
            out_scratch,
            ratio,
            input_frames: 0,
            emitted: 0,
            skip,
        })
    }

    /// Feed mono frames in; get whatever full chunks produced out.
    ///
    /// Output length varies call to call — zero when the internal buffer has
    /// not filled a chunk yet — and that is fine for every caller here: the
    /// probe recorder appends to a buffer and the meter never sees resampled
    /// audio at all.
    pub fn push(&mut self, mono: &[f32]) -> Result<Vec<f32>, AudioError> {
        self.pending.extend_from_slice(mono);
        self.input_frames += mono.len() as u64;

        let mut out = Vec::new();
        while self.pending.len() >= self.inner.input_frames_next() {
            let take = self.inner.input_frames_next();
            let written = self.process_chunk(take, None)?;
            self.collect(written, &mut out);
            self.pending.drain(..take);
        }
        Ok(out)
    }

    /// Drain what remains: the partial last chunk, then silence until the
    /// filter has given back everything it was fed.
    ///
    /// The target length is exact — `round(input × ratio)` — because the
    /// probe's contract is that five seconds in is five seconds out. Under-
    /// draining would clip the tail the filter still holds; over-draining
    /// would append the silence used to flush it. Both are wrong lengths, and
    /// the tests assert durations.
    pub fn finish(mut self) -> Result<Vec<f32>, AudioError> {
        // How many frames the whole stream should have produced, counting
        // what push() already handed out.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let target = (self.input_frames as f64 * self.ratio).round() as u64;
        let mut out = Vec::new();

        let tail = self.pending.len();
        if tail > 0 {
            // partial_len reads `tail` real frames and treats the rest of the
            // chunk as silence — rubato's documented way to end a stream.
            let written = self.process_chunk(tail, Some(tail))?;
            self.collect(written, &mut out);
            self.pending.clear();
        }

        // Feed silence until the delayed tail is out. The delay is a fraction
        // of one chunk's output, so this converges immediately; the bound is
        // a backstop so a broken filter cannot loop forever.
        for _ in 0..8 {
            if self.emitted >= target {
                break;
            }
            let before = self.emitted;
            let written = self.process_chunk(0, Some(0))?;
            self.collect(written, &mut out);
            // No forward progress means no more will come; stop rather than spin.
            if self.emitted == before {
                break;
            }
        }

        // Trim the flush silence past the target off this final piece.
        let overshoot = usize::try_from(self.emitted.saturating_sub(target)).unwrap_or(usize::MAX);
        out.truncate(out.len().saturating_sub(overshoot));
        Ok(out)
    }

    /// Move `written` frames from the scratch into `out`, dropping whatever
    /// remains of the filter delay first and keeping the emitted count true.
    fn collect(&mut self, written: usize, out: &mut Vec<f32>) {
        let dropped = self.skip.min(written);
        self.skip -= dropped;
        out.extend_from_slice(&self.out_scratch[dropped..written]);
        self.emitted += (written - dropped) as u64;
    }

    /// Run one resampler call over the first `available` pending frames.
    ///
    /// `partial` is `Some(n)` for the final, short read. Returns how many
    /// output frames landed in `out_scratch`.
    fn process_chunk(
        &mut self,
        available: usize,
        partial: Option<usize>,
    ) -> Result<usize, AudioError> {
        // The adapter wants a buffer of at least one full chunk even when only
        // `partial` frames of it are real, so pad a copy for the short reads.
        let chunk = self.inner.input_frames_next();
        let mut padded;
        let input: &[f32] = if available >= chunk {
            &self.pending[..chunk]
        } else {
            padded = self.pending[..available].to_vec();
            padded.resize(chunk, 0.0);
            &padded
        };

        let input_adapter =
            InterleavedSlice::new(input, 1, chunk).map_err(|e| AudioError::Resample {
                detail: e.to_string(),
            })?;
        let out_len = self.out_scratch.len();
        let mut output_adapter = InterleavedSlice::new_mut(&mut self.out_scratch, 1, out_len)
            .map_err(|e| AudioError::Resample {
                detail: e.to_string(),
            })?;

        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: partial,
            active_channels_mask: None,
        };

        let (_read, written) = self
            .inner
            .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
            .map_err(|e| AudioError::Resample {
                detail: e.to_string(),
            })?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine wave as a mono f32 buffer.
    fn sine(rate_hz: u32, freq_hz: f32, seconds: f32) -> Vec<f32> {
        let frames = (rate_hz as f32 * seconds) as usize;
        (0..frames)
            .map(|i| (2.0 * std::f32::consts::PI * freq_hz * i as f32 / rate_hz as f32).sin())
            .collect()
    }

    /// Zero crossings ≈ 2 × frequency × seconds; a resampler that aliased or
    /// dropped chunks moves this number far off.
    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count()
    }

    /// Push in awkward, uneven pieces to prove the internal buffering does not
    /// depend on callers delivering tidy block sizes — cpal certainly won't.
    fn resample_all(input_rate: u32, samples: &[f32]) -> Vec<f32> {
        let mut r = MonoResampler::new(input_rate).unwrap();
        let mut out = Vec::new();
        for piece in samples.chunks(479) {
            out.extend(r.push(piece).unwrap());
        }
        out.extend(r.finish().unwrap());
        out
    }

    #[test]
    fn a_48k_second_becomes_exactly_a_16k_second() {
        // Exact, not approximate: the filter delay is skipped and the flush
        // is truncated, so one second in is one second out to the frame.
        let out = resample_all(48_000, &sine(48_000, 440.0, 1.0));
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn a_44_1k_second_becomes_exactly_a_16k_second() {
        let out = resample_all(44_100, &sine(44_100, 440.0, 1.0));
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn the_tone_survives_the_trip() {
        // 440 Hz for one second is ~880 zero crossings, at any sample rate.
        // If chunking dropped or duplicated audio, the count shifts by whole
        // chunks' worth and this fails loudly.
        let out = resample_all(48_000, &sine(48_000, 440.0, 1.0));
        let crossings = zero_crossings(&out);
        assert!(
            (860..=900).contains(&crossings),
            "440 Hz should cross zero ~880 times, counted {crossings}"
        );
    }

    #[test]
    fn amplitude_is_preserved_not_just_shape() {
        let out = resample_all(48_000, &sine(48_000, 440.0, 1.0));
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            (0.9..=1.1).contains(&peak),
            "a full-scale sine should stay near full scale, peak {peak}"
        );
    }

    #[test]
    fn an_already_16k_input_passes_through_at_the_same_length() {
        let out = resample_all(16_000, &sine(16_000, 440.0, 1.0));
        assert_eq!(out.len(), 16_000, "ratio 1.0 must not change duration");
    }

    #[test]
    fn a_zero_rate_is_refused_with_words() {
        // Not `unwrap_err`: that would demand Debug on the resampler, which
        // owns a filter bank nobody wants printed.
        let Err(error) = MonoResampler::new(0) else {
            panic!("a 0 Hz input rate must be refused");
        };
        assert!(
            error.to_string().contains("0 Hz"),
            "the error must name the nonsense rate: {error}"
        );
    }

    #[test]
    fn downmix_averages_the_channels_of_each_frame() {
        // L=0.5 R=0.1 → 0.3; L=-1 R=1 → 0.
        let stereo = [0.5, 0.1, -1.0, 1.0];
        assert_eq!(downmix(&stereo, 2), vec![0.3, 0.0]);
    }

    #[test]
    fn downmix_leaves_mono_alone_and_drops_a_torn_frame() {
        assert_eq!(downmix(&[0.1, 0.2], 1), vec![0.1, 0.2]);
        // Five samples of stereo is two frames and a torn half-frame; the
        // half-frame must not become an invented sample.
        let torn = [0.2, 0.2, 0.4, 0.4, 0.9];
        assert_eq!(downmix(&torn, 2), vec![0.2, 0.4]);
    }
}
