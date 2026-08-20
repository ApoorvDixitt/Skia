// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The level meter's arithmetic, kept separate so it can be tested exactly.
//!
//! The meter exists for one reason: the measured failure mode of audio capture
//! on macOS is silence that looks like success. A moving bar is how the user
//! sees, before any transcription exists, that the microphone is actually
//! being heard — and a bar that stays at zero while they speak is the honest
//! signal that consent or device selection is wrong.

use super::LevelUpdate;

/// Sample-level threshold under which a window counts as clipping.
///
/// Full scale minus a hair, because a converter that clipped rarely reports
/// 0.9997… rather than exactly 1.0.
const CLIP_THRESHOLD: f32 = 0.999;

/// Accumulates mono samples and emits one [`LevelUpdate`] per full window.
pub struct LevelWindow {
    /// Samples per update. Chosen by the caller from the stream's rate.
    window: usize,
    count: usize,
    /// Sum of squares in f64: a 100 ms window at 48 kHz is 4 800 squares, and
    /// f32 accumulation would lose the small ones next to the large.
    sum_squares: f64,
    peak: f32,
}

impl LevelWindow {
    pub fn new(window: usize) -> Self {
        Self {
            // A zero window would emit infinitely many updates for no samples.
            window: window.max(1),
            count: 0,
            sum_squares: 0.0,
            peak: 0.0,
        }
    }

    /// Feed samples; get an update for each window boundary crossed.
    ///
    /// Usually returns nothing or one update. Returns several only if the
    /// caller delivered more than a window's worth at once, which happens
    /// when a stream rebuild flushed a backlog — those windows are still real
    /// audio and still deserve their readings.
    pub fn push(&mut self, mono: &[f32]) -> Vec<LevelUpdate> {
        let mut updates = Vec::new();
        for &sample in mono {
            let magnitude = sample.abs();
            self.sum_squares += f64::from(sample) * f64::from(sample);
            if magnitude > self.peak {
                self.peak = magnitude;
            }
            self.count += 1;

            if self.count == self.window {
                #[allow(clippy::cast_possible_truncation)]
                let rms = (self.sum_squares / self.count as f64).sqrt() as f32;
                updates.push(LevelUpdate {
                    rms,
                    peak: self.peak,
                    clipped: self.peak >= CLIP_THRESHOLD,
                });
                self.count = 0;
                self.sum_squares = 0.0;
                self.peak = 0.0;
            }
        }
        updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_window_emits_exactly_one_update() {
        let mut meter = LevelWindow::new(4);
        assert!(
            meter.push(&[0.5, 0.5, 0.5]).is_empty(),
            "window not full yet"
        );

        let updates = meter.push(&[0.5]);
        assert_eq!(updates.len(), 1);
        // RMS of a constant is the constant; peak likewise.
        assert!((updates[0].rms - 0.5).abs() < 1e-6);
        assert!((updates[0].peak - 0.5).abs() < 1e-6);
        assert!(!updates[0].clipped);
    }

    #[test]
    fn the_window_resets_between_updates() {
        let mut meter = LevelWindow::new(2);
        let loud = meter.push(&[1.0, 1.0]).remove(0);
        let quiet = meter.push(&[0.0, 0.0]).remove(0);
        assert!(loud.peak > 0.9, "first window saw the loud samples");
        assert!(
            quiet.peak < 1e-6 && quiet.rms < 1e-6,
            "the loud window must not bleed into the quiet one: {quiet:?}"
        );
    }

    #[test]
    fn one_big_push_yields_every_window_it_spans() {
        let mut meter = LevelWindow::new(2);
        // Six samples over window 2 = three complete windows.
        let updates = meter.push(&[0.1, 0.1, 0.9, 0.9, 0.2, 0.2]);
        assert_eq!(updates.len(), 3);
        assert!(
            (updates[1].peak - 0.9).abs() < 1e-6,
            "windows stay in order"
        );
    }

    #[test]
    fn clipping_is_detected_from_either_polarity() {
        let mut meter = LevelWindow::new(2);
        assert!(meter.push(&[1.0, 0.0]).remove(0).clipped);
        assert!(meter.push(&[-1.0, 0.0]).remove(0).clipped);
        assert!(!meter.push(&[0.9, -0.9]).remove(0).clipped);
    }

    #[test]
    fn rms_is_actually_root_mean_square() {
        let mut meter = LevelWindow::new(2);
        // Samples 0.6 and 0.8: mean square (0.36 + 0.64) / 2 = 0.5.
        let update = meter.push(&[0.6, 0.8]).remove(0);
        assert!((update.rms - 0.5f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn a_zero_window_is_clamped_rather_than_dividing_by_zero() {
        let mut meter = LevelWindow::new(0);
        let updates = meter.push(&[0.5]);
        assert_eq!(updates.len(), 1, "window clamps to 1");
    }
}
