// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Turn the overlay into a non-activating `NSPanel` (macOS only).
//!
//! This is the fix for the two Tier-B properties `stealth.rs` has been
//! reporting as `false` since the overlay shipped: the dock icon that cannot
//! be hidden, and the single focus steal when the overlay opens. Both had the
//! same cause, and `lib.rs` documented it with measurements — an accessory
//! activation policy hides the dock icon, but an *ordinary* window in an
//! accessory app is never ordered onto the screen (0/5 cold launches, in every
//! ordering tried), because nothing activates the app and so nothing calls
//! `makeKeyAndOrderFront`.
//!
//! An `NSPanel` with `NSWindowStyleMaskNonactivatingPanel` breaks the
//! dependency: it can be ordered front *without* activating the app, so the
//! overlay becomes visible and the app can stay accessory. That is the
//! documented AppKit mechanism for exactly this, and it is what the TRD
//! anticipated.
//!
//! # The trap this module exists to avoid
//!
//! A non-activating panel does not take key focus, and the overlay has a text
//! input for Ask mode. Left there, this change would trade one honest
//! limitation for a worse one: an overlay nobody can type into.
//! `becomesKeyOnlyIfNeeded` is the answer — the panel refuses key status by
//! default, but grants it to a view that genuinely needs it (a text field the
//! user clicked). So typing works, and clicking the panel's background still
//! does not steal focus from the meeting.
//!
//! # What this module does not claim
//!
//! Whether the overlay is *actually* on screen across cold launches is an
//! observation, not a promise — the previous approach failed exactly there, at
//! 0/5. [`outcome`] records what was applied, `stealth.rs` reports it, and the
//! caveats say plainly that it needs re-verifying per macOS release. The
//! measurement protocol is the harness's, unchanged.

/// What the panel conversion actually achieved, recorded once at startup.
///
/// Every field is a fact about a call that either happened or did not, never
/// an assumption. `stealth.rs` reads this rather than guessing, so the status
/// panel cannot claim a property the OS refused to give.
#[derive(Debug, Clone, Copy, Default)]
pub struct PanelOutcome {
    /// The overlay is an `NSPanel` with the non-activating style mask.
    pub non_activating: bool,
    /// The app runs under `ActivationPolicy::Accessory`, so no dock icon.
    pub accessory_policy: bool,
    /// `becomesKeyOnlyIfNeeded` is set, so text input still works.
    pub key_only_if_needed: bool,
}

impl PanelOutcome {
    /// True only when the panel exists *and* the dock icon is hidden. Either
    /// alone is the broken state the old code measured, so they are reported
    /// together.
    pub fn fully_applied(self) -> bool {
        self.non_activating && self.accessory_policy
    }
}

#[cfg(target_os = "macos")]
mod platform {
    // `tauri-nspanel` v2 still reaches AppKit through the `cocoa` crate, which
    // is deprecated in favour of `objc2-app-kit`. Allowed rather than worked
    // around: the alternative is hand-rolling the panel subclass, and the
    // crate's own lib.rs carries the same allow. When it migrates, this goes.
    #![allow(deprecated)]

    use std::sync::OnceLock;

    use tauri::{Manager, Runtime, WebviewWindow};
    use tauri_nspanel::cocoa::appkit::NSWindowCollectionBehavior;
    use tauri_nspanel::WebviewWindowExt;

    use super::PanelOutcome;

    /// `NSWindowStyleMaskNonactivatingPanel`. Hard-coded because AppKit's
    /// constant is not re-exported by the binding crate; the value is stable
    /// AppKit ABI and is what every NSPanel overlay uses.
    const NONACTIVATING_PANEL: i32 = 1 << 7;

    /// One above `NSMainMenuWindowLevel` (24), i.e. status-item level. High
    /// enough to sit over a full-screen video call, low enough to stay under
    /// system alerts — an overlay that covers a permission dialog is a bug.
    const OVERLAY_LEVEL: i32 = 25;

    /// Recorded once, at setup, then read by the status panel. A `OnceLock`
    /// rather than app state because it describes a moment that happens once
    /// per process and can never change afterwards.
    static OUTCOME: OnceLock<PanelOutcome> = OnceLock::new();

    pub fn outcome() -> PanelOutcome {
        OUTCOME.get().copied().unwrap_or_default()
    }

    /// Convert `window` to a non-activating panel and demote the app.
    ///
    /// Order matters and is the lesson of the failed attempt: the panel must
    /// exist and be ordered front *before* the app becomes accessory. Demoting
    /// first means the app has already opted out of activation by the time
    /// anything tries to order the window front, which is precisely the 0/5
    /// case. Nothing here is fatal — a failure leaves the previous, working
    /// behaviour (visible overlay, visible dock icon) and reports it.
    pub fn convert<R: Runtime>(window: &WebviewWindow<R>) -> PanelOutcome {
        let mut result = PanelOutcome::default();

        let panel = match window.to_panel() {
            Ok(panel) => panel,
            Err(error) => {
                eprintln!(
                    "skia: the overlay could not become an NSPanel, so it keeps a dock icon \
                     and activates once: {error}"
                );
                let _ = OUTCOME.set(result);
                return result;
            }
        };

        panel.set_level(OVERLAY_LEVEL);

        // The whole point: a panel that is ordered front without activating.
        panel.set_style_mask(NONACTIVATING_PANEL);
        result.non_activating = true;

        // Without this the Ask input could never be typed into — see the
        // module docs. Set immediately after the style mask so the two can
        // never drift apart.
        panel.set_becomes_key_only_if_needed(true);
        result.key_only_if_needed = true;

        // Present on every space, and not dragged along by Mission Control.
        // `FullScreenAuxiliary` is what lets it sit over a full-screen call,
        // which is the situation the overlay exists for.
        panel.set_collection_behaviour(
            NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
        );

        // A floating panel does not hide when the app deactivates — an overlay
        // that vanished the moment the user clicked their meeting would be
        // useless.
        panel.set_floating_panel(true);
        panel.set_hides_on_deactivate(false);

        // Ordered front *regardless* of activation: this is the call an
        // ordinary window could not make work under an accessory policy.
        panel.order_front_regardless();

        // Only now demote the app, and only if the panel is genuinely on
        // screen — hiding the dock icon while the overlay is invisible is the
        // worst of the measured outcomes, and it is better to keep a dock icon
        // than to reach it.
        if panel.is_visible() {
            match window
                .app_handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)
            {
                Ok(()) => result.accessory_policy = true,
                Err(error) => eprintln!(
                    "skia: the dock icon could not be hidden, so it stays visible: {error}"
                ),
            }
        } else {
            eprintln!(
                "skia: the overlay panel reports itself off screen, so the dock icon was left \
                 visible rather than risk an invisible overlay"
            );
        }

        let _ = OUTCOME.set(result);
        result
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use tauri::{Runtime, WebviewWindow};

    use super::PanelOutcome;

    /// Windows has no dock icon and its overlay already does not activate, so
    /// there is nothing to convert. Reported as all-false rather than
    /// all-true: these fields describe a macOS mechanism, and `stealth.rs`
    /// answers the platform's own questions separately.
    pub fn convert<R: Runtime>(_window: &WebviewWindow<R>) -> PanelOutcome {
        PanelOutcome::default()
    }

    pub fn outcome() -> PanelOutcome {
        PanelOutcome::default()
    }
}

pub use platform::{convert, outcome};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_without_the_policy_is_not_fully_applied() {
        // The half-state is the dangerous one: a panel with a dock icon still
        // works, but must not be reported as the finished article.
        let half = PanelOutcome {
            non_activating: true,
            accessory_policy: false,
            key_only_if_needed: true,
        };
        assert!(!half.fully_applied());

        // And the inverse — dock hidden with no panel — is the 0/5 invisible
        // overlay this module exists to prevent.
        let inverted = PanelOutcome {
            non_activating: false,
            accessory_policy: true,
            key_only_if_needed: false,
        };
        assert!(!inverted.fully_applied());
    }

    #[test]
    fn the_default_claims_nothing() {
        let outcome = PanelOutcome::default();
        assert!(!outcome.non_activating);
        assert!(!outcome.accessory_policy);
        assert!(!outcome.key_only_if_needed);
        assert!(!outcome.fully_applied());
    }

    #[test]
    fn both_applied_is_the_only_fully_applied_state() {
        let full = PanelOutcome {
            non_activating: true,
            accessory_policy: true,
            key_only_if_needed: true,
        };
        assert!(full.fully_applied());
    }
}
