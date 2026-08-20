// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Decides *when* a device change warrants rebuilding the stream.
//!
//! `docs/ARCHITECTURE.md` names device hot-swap the main crash risk: people
//! plug in headphones mid-call, and an engine that assumed a stable device
//! dies on a real-time thread. The engine therefore watches the default input
//! device and rebuilds — but not on the first sighting of a change.
//!
//! The debounce exists because of Bluetooth. A headset switching between its
//! A2DP and HFP profiles presents as several device changes within a second
//! or two — the audio harness's `hotswap-probe` exists to measure exactly how
//! many and how close — and rebuilding once per notification would put one
//! transcript gap per notification into what the user experienced as a single
//! action. So a change only triggers a rebuild once the *same* new device has
//! stayed the default for a full window.
//!
//! Time arrives as a plain milliseconds argument rather than `Instant::now()`
//! so the tests can drive the clock; this logic is exactly the kind that only
//! ever misbehaves in the field if it is not tested against a fake clock.

/// How long a new default device must hold before the stream is rebuilt.
///
/// A guess pending real numbers: comfortably longer than a single A2DP↔HFP
/// flap, comfortably shorter than anyone's patience. When `hotswap-probe`
/// measurements arrive, this constant is where they land.
pub const DEBOUNCE_MS: u64 = 750;

/// What [`SwapDetector::observe`] concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to do: the default still matches the open stream, or a change
    /// has not yet held for the debounce window.
    Hold,
    /// The default has settled on a different device; rebuild onto it.
    Rebuild(String),
}

/// Watches the reported default device and applies the debounce.
pub struct SwapDetector {
    /// The device the currently open stream was built on.
    active: Option<String>,
    /// A different default that has been sighted but not yet settled.
    candidate: Option<String>,
    candidate_since_ms: u64,
    debounce_ms: u64,
}

impl SwapDetector {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            active: None,
            candidate: None,
            candidate_since_ms: 0,
            debounce_ms,
        }
    }

    /// Record that a stream was (re)built on `device`.
    pub fn stream_opened(&mut self, device: &str) {
        self.active = Some(device.to_string());
        self.candidate = None;
    }

    /// Record that the stream is gone, whatever the reason.
    pub fn stream_closed(&mut self) {
        self.active = None;
        self.candidate = None;
    }

    /// Consider the current default device at time `now_ms`.
    ///
    /// `default` is `None` when the OS reports no input device at all — mid
    /// unplug, for instance. That never triggers a rebuild by itself; there is
    /// nothing to rebuild onto, and the stream's own error callback covers the
    /// case where the device under it died.
    pub fn observe(&mut self, now_ms: u64, default: Option<&str>) -> Verdict {
        let Some(active) = self.active.as_deref() else {
            // No stream is open, so there is nothing to swap.
            return Verdict::Hold;
        };
        let Some(default) = default else {
            self.candidate = None;
            return Verdict::Hold;
        };

        if default == active {
            // The flap resolved back to the device already in use. Dropping
            // the candidate here is the debounce doing its job.
            self.candidate = None;
            return Verdict::Hold;
        }

        match self.candidate.as_deref() {
            Some(candidate) if candidate == default => {
                if now_ms.saturating_sub(self.candidate_since_ms) >= self.debounce_ms {
                    Verdict::Rebuild(default.to_string())
                } else {
                    Verdict::Hold
                }
            }
            // A new (or different) candidate: start its clock.
            _ => {
                self.candidate = Some(default.to_string());
                self.candidate_since_ms = now_ms;
                Verdict::Hold
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector_on(device: &str) -> SwapDetector {
        let mut detector = SwapDetector::new(DEBOUNCE_MS);
        detector.stream_opened(device);
        detector
    }

    #[test]
    fn a_stable_default_never_triggers_a_rebuild() {
        let mut d = detector_on("Built-in");
        for t in (0..10_000).step_by(250) {
            assert_eq!(d.observe(t, Some("Built-in")), Verdict::Hold);
        }
    }

    #[test]
    fn a_change_rebuilds_only_after_it_holds_for_the_window() {
        let mut d = detector_on("Built-in");
        assert_eq!(
            d.observe(0, Some("AirPods")),
            Verdict::Hold,
            "first sighting"
        );
        assert_eq!(
            d.observe(500, Some("AirPods")),
            Verdict::Hold,
            "still early"
        );
        assert_eq!(
            d.observe(750, Some("AirPods")),
            Verdict::Rebuild("AirPods".to_string()),
            "held for the full window"
        );
    }

    #[test]
    fn a_bluetooth_flap_that_resolves_back_never_rebuilds() {
        // The measured Bluetooth shape: away and back within the window.
        let mut d = detector_on("Built-in");
        assert_eq!(d.observe(0, Some("AirPods")), Verdict::Hold);
        assert_eq!(
            d.observe(300, Some("Built-in")),
            Verdict::Hold,
            "flapped back"
        );
        // AirPods again much later: the earlier sighting must not count
        // towards the window — the clock restarts.
        assert_eq!(d.observe(5_000, Some("AirPods")), Verdict::Hold);
        assert_eq!(d.observe(5_500, Some("AirPods")), Verdict::Hold);
        assert_eq!(
            d.observe(5_750, Some("AirPods")),
            Verdict::Rebuild("AirPods".to_string())
        );
    }

    #[test]
    fn a_change_of_candidate_restarts_the_clock() {
        // A→B→C within one window: C's clock starts at its own sighting.
        let mut d = detector_on("A");
        assert_eq!(d.observe(0, Some("B")), Verdict::Hold);
        assert_eq!(d.observe(400, Some("C")), Verdict::Hold, "new candidate");
        assert_eq!(
            d.observe(400 + DEBOUNCE_MS - 1, Some("C")),
            Verdict::Hold,
            "B's time must not count for C"
        );
        assert_eq!(
            d.observe(400 + DEBOUNCE_MS, Some("C")),
            Verdict::Rebuild("C".to_string())
        );
    }

    #[test]
    fn no_device_at_all_is_not_a_rebuild_and_clears_the_candidate() {
        let mut d = detector_on("Built-in");
        assert_eq!(d.observe(0, Some("AirPods")), Verdict::Hold);
        // Devices list went empty mid-swap; the candidate is stale now.
        assert_eq!(d.observe(200, None), Verdict::Hold);
        // AirPods must start a fresh window after the gap.
        assert_eq!(d.observe(300, Some("AirPods")), Verdict::Hold);
        assert_eq!(
            d.observe(300 + DEBOUNCE_MS - 1, Some("AirPods")),
            Verdict::Hold
        );
    }

    #[test]
    fn nothing_happens_when_no_stream_is_open() {
        let mut d = SwapDetector::new(DEBOUNCE_MS);
        assert_eq!(d.observe(0, Some("AirPods")), Verdict::Hold);
        let mut d2 = detector_on("Built-in");
        d2.stream_closed();
        assert_eq!(d2.observe(10_000, Some("AirPods")), Verdict::Hold);
    }
}
