// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Ask macOS for microphone permission, explicitly, before capturing.
//!
//! This module exists because of a measured, then root-caused, failure: the
//! shipped app captured five seconds of perfect 16 kHz audio in which every
//! sample was zero, and no permission dialog ever appeared. TCC logged
//! nothing. The reason is in Apple's own AVFoundation documentation:
//!
//! - *"Until access has been granted, any AVCaptureDevices for the media type
//!   will vend silent audio samples"*, and
//! - the authorization dialog is shown automatically only *"when creating an
//!   AVCaptureDeviceInput"*.
//!
//! cpal reaches the microphone through the CoreAudio HAL and never creates an
//! `AVCaptureDeviceInput`, so nothing ever triggered the dialog — the OS
//! quietly fed the stream zeros, indistinguishable from a muted microphone.
//! `NSMicrophoneUsageDescription` in the Info.plist is necessary (its absence
//! aborts the process) but not sufficient: somebody has to actually ask.
//!
//! So the commands that open a stream call [`ensure_microphone`] first. On
//! macOS that checks `AVCaptureDevice.authorizationStatus` and, when the
//! answer is "never asked", blocks on `requestAccess` — which is the call
//! that puts the dialog on screen. Everywhere else it is a no-op: Windows
//! surfaces microphone privacy as a device-level switch, and a revoked mic
//! shows up there as a failed or silent stream, reported by the meter.
//!
//! This is the crate's first `unsafe` code, and `Cargo.toml` documents why
//! the "no objc2" stance was revised: capture *protection* genuinely needs no
//! native code, but there is no safe wrapper for TCC consent, and the
//! alternative — shipping an app that records silence forever — is worse than
//! two audited `unsafe` blocks.

use super::AudioError;

/// What the OS says about microphone access, in Skia's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicConsent {
    /// Capture will deliver real audio.
    Granted,
    /// The user said no, or a profile did. Only System Settings can undo it.
    Denied,
    /// Never asked. Asking is what [`ensure_microphone`] does next.
    NotYetAsked,
}

/// Check — and if never asked, ask for — microphone permission.
///
/// Blocks until the user answers the dialog, so call it from a blocking task,
/// never an event loop. Returns `Ok(())` exactly when capture will produce
/// real samples; every other outcome is an error that names what to do,
/// because the alternative the bug shipped with was a meter that sat at zero
/// with nothing to act on.
pub fn ensure_microphone() -> Result<(), AudioError> {
    match platform::microphone_consent() {
        MicConsent::Granted => Ok(()),
        MicConsent::NotYetAsked => {
            if platform::request_microphone() {
                Ok(())
            } else {
                Err(denied())
            }
        }
        MicConsent::Denied => Err(denied()),
    }
}

fn denied() -> AudioError {
    AudioError::MicAccessDenied {
        detail: "macOS is blocking microphone access for Skia. Open System Settings → \
                 Privacy & Security → Microphone, switch Skia on, then try again — \
                 the OS delivers silence, not an error, while access is blocked"
            .to_string(),
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::StackBlock;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    use super::MicConsent;

    /// Current status, without prompting.
    pub fn microphone_consent() -> MicConsent {
        // SAFETY: `AVMediaTypeAudio` is a static the framework initialises at
        // load; `authorizationStatusForMediaType` is a class method documented
        // for exactly this constant. Neither takes ownership of anything.
        let status = unsafe {
            let Some(audio) = AVMediaTypeAudio else {
                // The constant missing means AVFoundation is not in the
                // process, which cannot happen in a Tauri app — but "cannot
                // happen" is not a reason to crash the audio path. Report the
                // unaskable state as denied so the user gets words, not
                // silence.
                return MicConsent::Denied;
            };
            AVCaptureDevice::authorizationStatusForMediaType(audio)
        };

        match status {
            AVAuthorizationStatus::Authorized => MicConsent::Granted,
            AVAuthorizationStatus::NotDetermined => MicConsent::NotYetAsked,
            // Denied and Restricted both mean "no capture, only System
            // Settings can change it" from where Skia stands.
            _ => MicConsent::Denied,
        }
    }

    /// Put the consent dialog on screen and wait for the answer.
    ///
    /// The completion handler lands on an arbitrary dispatch queue; a channel
    /// carries the verdict back. The five-minute ceiling is not a guess at
    /// how long reading a dialog takes — it is the difference between a
    /// caller that eventually reports "no answer" and one that hangs forever
    /// if the dialog is dismissed by something unusual (fast user switching,
    /// a logout) without the handler firing.
    pub fn request_microphone() -> bool {
        let (tx, rx) = mpsc::channel::<bool>();

        let handler = StackBlock::new(move |granted: objc2::runtime::Bool| {
            let _ = tx.send(granted.as_bool());
        });

        // SAFETY: same static as above; the block is copied by the framework
        // before this frame unwinds (StackBlock::copy is implied by passing
        // it as a block argument through objc2's encoding).
        unsafe {
            let Some(audio) = AVMediaTypeAudio else {
                return false;
            };
            AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler);
        }

        rx.recv_timeout(Duration::from_secs(300)).unwrap_or(false)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::MicConsent;

    /// Windows has no per-app consent dialog to trigger from here: microphone
    /// privacy is a Settings switch, and when it is off the stream fails or
    /// delivers silence — which the meter and the probe's `silent` flag
    /// already report. Claiming `Granted` here means "nothing for the app to
    /// ask", not "the OS promised audio".
    pub fn microphone_consent() -> MicConsent {
        MicConsent::Granted
    }

    pub fn request_microphone() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The real dialog needs a human and a session; what a test can pin down
    // is the mapping from consent states to caller-visible behaviour, which
    // is the part that must not drift.

    #[test]
    fn denied_maps_to_an_error_that_names_system_settings() {
        let error = denied();
        let text = error.to_string();
        assert!(
            text.contains("System Settings") && text.contains("Microphone"),
            "the user must be told where the switch is: {text}"
        );
        assert!(
            text.contains("silence, not an error"),
            "the error must explain why nothing else warned them: {text}"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_platforms_have_nothing_to_ask() {
        assert!(ensure_microphone().is_ok());
    }
}
