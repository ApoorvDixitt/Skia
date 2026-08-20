// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Window stealth: capture exclusion and presence invisibility, plus honest
//! reporting of what is *actually* active on the current OS.
//!
//! The reporting matters as much as the mechanism. Capture exclusion is
//! documented and supported on Windows, but on macOS it rests on
//! `NSWindow.sharingType = .none`, which Apple's own documentation now calls
//! "a legacy constant that macOS no longer uses" and advises against relying on
//! for exactly this purpose — while the shipping SDK header still describes it
//! as preventing capture. We measured it working on macOS 26.5 (see
//! `tools/macos-capture-harness`), so it is offered, but it is reported as
//! measured-not-guaranteed and never as a promise.
//!
//! Both mechanisms come from Tauri's `set_content_protected`, which maps to
//! `NSWindow.sharingType = .none` on macOS and `SetWindowDisplayAffinity` with
//! `WDA_EXCLUDEFROMCAPTURE` on Windows — the same calls the harness measured.

use serde::{Deserialize, Serialize};
use tauri::{Runtime, WebviewWindow};

use crate::panel;

/// How much the platform vendor actually stands behind a mechanism.
///
/// The distinction between `Documented` and `Measured` is the whole point: one
/// is a contract, the other is an observation that a point release may undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportLevel {
    /// The vendor documents and supports this. Safe to state plainly.
    Documented,
    /// Observed to work, but undocumented or documented as discouraged. Must be
    /// presented as a bonus, never a guarantee.
    Measured,
    /// No mechanism exists on this platform.
    Unavailable,
}

/// Whether the overlay's pixels are withheld from screen capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureExclusion {
    /// The user asked for it.
    pub requested: bool,
    /// It was actually applied on this platform.
    pub active: bool,
    /// The concrete native mechanism, so the claim is auditable.
    pub mechanism: Option<String>,
    pub support: SupportLevel,
    /// One honest sentence about how much this can be relied on.
    pub guarantee: String,
}

/// Presence invisibility. Unlike capture exclusion, all of this is ordinary
/// supported window configuration and holds on every platform we target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub no_dock_icon: bool,
    pub no_taskbar_entry: bool,
    pub no_alt_tab: bool,
    pub never_steals_focus: bool,
    /// The macOS mechanism behind the two fields above, when one is in play.
    /// `None` off macOS, where neither question arises the same way.
    pub mechanism: Option<String>,
    /// How much the two panel-dependent claims can be relied on. `Measured`
    /// even when applied: the panel is documented AppKit, but *this app's
    /// overlay staying visible under it* is an observation, and the previous
    /// approach failed exactly there.
    pub support: SupportLevel,
    /// Whether the overlay can still receive typed input. Only interesting
    /// alongside `never_steals_focus`: a non-activating panel refuses key
    /// status by default, and `becomesKeyOnlyIfNeeded` is what gives it back
    /// to a text field the user clicked. Reported because an overlay that
    /// cannot be typed into would be a worse regression than the focus steal.
    pub accepts_typing: bool,
}

/// The full honest picture, shaped for display without further interpretation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StealthStatus {
    pub platform: String,
    pub os_version: String,
    pub capture_exclusion: CaptureExclusion,
    pub presence: Presence,
    /// Always `true`. Neither macOS nor Windows offers a public way to hide a
    /// window's *existence*: it stays discoverable through
    /// `SCShareableContent` / `CGWindowListCopyWindowInfo` on macOS and the
    /// window list on Windows, along with its owning process and geometry.
    /// Pixels can be withheld; presence cannot.
    pub window_enumerable: bool,
    pub caveats: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StealthError {
    #[error("failed to apply window stealth settings: {0}")]
    Window(#[from] tauri::Error),
}

/// What capture exclusion means on the platform we were compiled for.
fn capture_mechanism() -> (Option<&'static str>, SupportLevel, &'static str) {
    #[cfg(target_os = "windows")]
    {
        (
            Some("SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)"),
            SupportLevel::Documented,
            "Documented and supported by Microsoft. Reliable on Windows 10 2004 and later.",
        )
    }
    #[cfg(target_os = "macos")]
    {
        (
            Some("NSWindow.sharingType = .none"),
            SupportLevel::Measured,
            "Measured working on macOS 26.5, but undocumented: Apple advises against relying on \
             this and has an open bug where exclusion breaks after a capture filter is rebuilt. \
             Treat it as a bonus, not a guarantee, and re-verify after each macOS update.",
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        (
            None,
            SupportLevel::Unavailable,
            "This platform has no supported way to exclude a window from screen capture.",
        )
    }
}

fn caveats(active: bool, requested: bool, presence: &Presence) -> Vec<String> {
    let mut out = vec![
        "Other applications can still see that this window exists, along with the process \
         that owns it and its size and position. Only the pixels are withheld."
            .to_string(),
        "No user-space application can defend against device management, kernel-level \
         monitoring, or a camera pointed at your screen."
            .to_string(),
    ];

    // These two were unconditional while both were permanently false. They are
    // now conditional on what the panel conversion achieved, because stating a
    // limitation that no longer exists is its own kind of dishonesty.
    if !presence.never_steals_focus {
        out.push(
            "The overlay takes focus once when it opens, because it is not running as a \
             non-activating panel on this system."
                .to_string(),
        );
    }
    if cfg!(target_os = "macos") && !presence.no_dock_icon {
        out.push(
            "On macOS the app still shows a dock icon. It is hidden only once the overlay is \
             confirmed on screen as a panel, because hiding it while the overlay is invisible \
             is the worse failure."
                .to_string(),
        );
    }
    if cfg!(target_os = "macos") && presence.never_steals_focus {
        out.push(
            "The overlay is a non-activating panel, which is documented AppKit — but that this \
             app's overlay stays reliably on screen under it is an observation, not a promise. \
             Re-check it after a macOS update, the same way capture exclusion is re-checked."
                .to_string(),
        );
        // A non-activating panel that never grants key status cannot be typed
        // into at all. That would be a worse regression than the focus steal
        // this change removed, so it is a warning rather than a footnote.
        if !presence.accepts_typing {
            out.push(
                "The overlay is non-activating but did not enable key-on-demand, so the Ask \
                 input may not accept typing. Use the dashboard until this is fixed."
                    .to_string(),
            );
        }
    }

    if cfg!(target_os = "macos") && active {
        out.push(
            "On macOS this relies on undocumented behaviour that a system update could change \
             without notice."
                .to_string(),
        );
    }
    if requested && !active {
        out.push(
            "Capture exclusion was requested but is not active on this system. Assume the \
             overlay is visible in screen shares and recordings."
                .to_string(),
        );
    }
    out
}

/// Applies presence invisibility to `window`. Separate from capture exclusion
/// because this part is supported everywhere and should not be entangled with
/// the part that isn't.
fn apply_presence<R: Runtime>(window: &WebviewWindow<R>) -> Result<Presence, StealthError> {
    // Keep the overlay above other windows and present on every space/desktop.
    window.set_always_on_top(true)?;
    window.set_visible_on_all_workspaces(true)?;

    // No taskbar button. On macOS the dock icon is handled by the app's
    // activation policy instead, set during setup.
    window.set_skip_taskbar(true)?;

    // Deliberately NOT calling set_focusable(false). That would permanently
    // prevent keyboard focus, which breaks any text input the overlay needs.

    // Read what the panel conversion actually achieved rather than assuming.
    // Both macOS claims below hang off it, and it records calls that either
    // happened or did not — see `panel.rs`.
    let panel = panel::outcome();

    Ok(Presence {
        // On macOS this is exactly "the app is accessory", which `panel.rs`
        // only applies once the panel is confirmed on screen — hiding the dock
        // icon while the overlay is invisible was the measured 0/5 failure and
        // must never be reachable. Windows has no dock, so the question does
        // not arise there.
        no_dock_icon: if cfg!(target_os = "macos") {
            panel.accessory_policy
        } else {
            true
        },
        no_taskbar_entry: true,
        // A skip-taskbar window with no dock icon is not offered in the window
        // switcher on either platform.
        no_alt_tab: true,
        // True only with a genuinely non-activating panel. An ordinary window
        // is ordered front by activating the app, which is the focus steal
        // this claim is about; `NSWindowStyleMaskNonactivatingPanel` plus
        // `orderFrontRegardless` is what removes it.
        never_steals_focus: if cfg!(target_os = "macos") {
            panel.non_activating
        } else {
            // The Windows overlay is created skip-taskbar and never activates
            // the app to show itself.
            true
        },
        mechanism: if cfg!(target_os = "macos") {
            Some(if panel.non_activating {
                "NSPanel with NSWindowStyleMaskNonactivatingPanel, becomesKeyOnlyIfNeeded, \
                 and ActivationPolicy::Accessory"
                    .to_string()
            } else {
                "ordinary NSWindow — the panel conversion did not take effect".to_string()
            })
        } else {
            None
        },
        support: if cfg!(target_os = "macos") {
            SupportLevel::Measured
        } else {
            SupportLevel::Documented
        },
        // Off macOS the overlay is an ordinary focusable window, so typing was
        // never in question there.
        accepts_typing: if cfg!(target_os = "macos") {
            !panel.non_activating || panel.key_only_if_needed
        } else {
            true
        },
    })
}

/// Applies stealth to `window` and reports precisely what took effect.
///
/// `requested` only expresses intent. The returned status reflects reality, so
/// a caller cannot accidentally present a capability the OS did not deliver.
pub fn apply<R: Runtime>(
    window: &WebviewWindow<R>,
    requested: bool,
) -> Result<StealthStatus, StealthError> {
    let presence = apply_presence(window)?;

    let (mechanism, support, guarantee) = capture_mechanism();
    let available = support != SupportLevel::Unavailable;

    // Only claim exclusion is active if the platform has a mechanism AND the
    // call succeeded. Failing closed is deliberate: over-reporting here would
    // mislead a user about whether they are visible on a call.
    let active = if available && requested {
        window.set_content_protected(true)?;
        true
    } else {
        if available {
            window.set_content_protected(false)?;
        }
        false
    };

    let info = os_info::get();

    Ok(StealthStatus {
        platform: std::env::consts::OS.to_string(),
        os_version: info.version().to_string(),
        capture_exclusion: CaptureExclusion {
            requested,
            active,
            mechanism: mechanism.map(str::to_string),
            support,
            guarantee: guarantee.to_string(),
        },
        window_enumerable: true,
        caveats: caveats(active, requested, &presence),
        presence,
    })
}

/// Reports current status without changing anything.
pub fn status<R: Runtime>(
    window: &WebviewWindow<R>,
    requested: bool,
) -> Result<StealthStatus, StealthError> {
    // Re-applying is idempotent and keeps the report honest about what is
    // genuinely in effect rather than what we set once at startup.
    apply(window, requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_level_serialises_lowercase() {
        let json = serde_json::to_string(&SupportLevel::Measured).expect("serialise");
        assert_eq!(json, "\"measured\"");
        let json = serde_json::to_string(&SupportLevel::Documented).expect("serialise");
        assert_eq!(json, "\"documented\"");
    }

    #[test]
    fn windows_is_documented_macos_is_only_measured() {
        let (mechanism, support, guarantee) = capture_mechanism();

        if cfg!(target_os = "windows") {
            assert_eq!(support, SupportLevel::Documented);
            assert!(mechanism.is_some_and(|m| m.contains("WDA_EXCLUDEFROMCAPTURE")));
        } else if cfg!(target_os = "macos") {
            // Must never be reported as documented: Apple advises against it.
            assert_eq!(support, SupportLevel::Measured);
            assert!(mechanism.is_some_and(|m| m.contains("sharingType")));
            assert!(guarantee.contains("not a guarantee"));
        } else {
            assert_eq!(support, SupportLevel::Unavailable);
            assert!(mechanism.is_none());
        }
    }

    /// A presence with the panel applied, as macOS reports it when the
    /// conversion worked.
    fn with_panel() -> Presence {
        Presence {
            no_dock_icon: true,
            no_taskbar_entry: true,
            no_alt_tab: true,
            never_steals_focus: true,
            mechanism: Some("NSPanel".to_string()),
            support: SupportLevel::Measured,
            accepts_typing: true,
        }
    }

    /// A presence with the conversion having failed — the state the overlay
    /// shipped in before the panel landed.
    fn without_panel() -> Presence {
        Presence {
            no_dock_icon: false,
            no_taskbar_entry: true,
            no_alt_tab: true,
            never_steals_focus: false,
            mechanism: Some("ordinary NSWindow".to_string()),
            support: SupportLevel::Measured,
            accepts_typing: true,
        }
    }

    #[test]
    fn caveats_always_mention_that_presence_is_not_hidden() {
        for presence in [with_panel(), without_panel()] {
            let c = caveats(true, true, &presence);
            assert!(c.iter().any(|s| s.contains("this window exists")));
            assert!(c.iter().any(|s| s.contains("Only the pixels are withheld")));
        }
    }

    #[test]
    fn requesting_exclusion_without_getting_it_is_stated_plainly() {
        let c = caveats(false, true, &with_panel());
        assert!(
            c.iter()
                .any(|s| s.contains("Assume the overlay is visible")),
            "a failed request must warn the user, not fail silently: {c:?}"
        );
    }

    #[test]
    fn the_focus_and_dock_limitations_are_disclosed_exactly_when_they_apply() {
        // Without the panel, both limitations are real and must be stated.
        let broken = without_panel();
        for (active, requested) in [(true, true), (false, false), (false, true)] {
            let c = caveats(active, requested, &broken);
            assert!(
                c.iter().any(|s| s.contains("takes focus once")),
                "the focus limitation must be disclosed while it is real: {c:?}"
            );
            if cfg!(target_os = "macos") {
                assert!(
                    c.iter().any(|s| s.contains("dock icon")),
                    "the dock limitation must be disclosed while it is real: {c:?}"
                );
            }
        }

        // With the panel, stating them would be false — and the replacement
        // caveat about re-verification must appear instead.
        let fixed = with_panel();
        let c = caveats(true, true, &fixed);
        assert!(
            !c.iter().any(|s| s.contains("takes focus once")),
            "a limitation that no longer exists must not be claimed: {c:?}"
        );
        if cfg!(target_os = "macos") {
            assert!(
                c.iter().any(|s| s.contains("not a promise")),
                "the panel's measured-not-guaranteed status must still be stated: {c:?}"
            );
        }
    }

    #[test]
    fn a_panel_that_cannot_be_typed_into_is_warned_about_loudly() {
        let mut mute = with_panel();
        mute.accepts_typing = false;
        let c = caveats(true, true, &mute);
        if cfg!(target_os = "macos") {
            assert!(
                c.iter().any(|s| s.contains("may not accept typing")),
                "an untypeable overlay is worse than a focus steal and must be said: {c:?}"
            );
        }
        // And the healthy case must not cry wolf.
        let c = caveats(true, true, &with_panel());
        assert!(!c.iter().any(|s| s.contains("may not accept typing")));
    }

    #[test]
    fn no_spurious_failure_warning_when_exclusion_was_not_requested() {
        let c = caveats(false, false, &with_panel());
        assert!(!c
            .iter()
            .any(|s| s.contains("Assume the overlay is visible")));
    }
}
