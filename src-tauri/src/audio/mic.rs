// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The cpal wiring: enumerate input devices and open the default microphone.
//!
//! Deliberately thin. Everything with logic in it — downmix, resampling,
//! metering, the swap debounce — lives in siblings that are tested against
//! synthetic input, and this module only turns cpal's callbacks into messages
//! on a channel. Hardware is the one thing a test suite cannot conjure, so
//! the less code that only runs against hardware, the better.
//!
//! Samples cross to the engine thread as messages because `cpal::Stream` is
//! not `Send`: the stream object must live its whole life on the engine
//! thread, while its data callback runs on a thread the OS owns. A channel is
//! the honest boundary between the two.

use std::sync::mpsc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};

use super::{AudioError, DeviceInfo};

/// The human-readable name of a device.
///
/// cpal 0.18 replaced `Device::name()` with a structured description; the name
/// inside it is the one users recognise from System Settings, which is what
/// the device list and the hot-swap log lines both want.
fn device_name(device: &cpal::Device) -> Result<String, AudioError> {
    device
        .description()
        .map(|d| d.name().to_string())
        .map_err(|e| AudioError::Device {
            detail: e.to_string(),
        })
}

/// What a stream callback sends the engine.
pub enum FromStream {
    /// Interleaved samples, converted to f32 whatever the hardware format.
    Frames(Vec<f32>),
    /// The stream died. Carries the OS's words; the engine decides what to do.
    Failed(String),
}

/// An open microphone stream plus the facts the engine needs about it.
///
/// Not `Send`, by construction — it exists only on the engine thread.
pub struct OpenMic {
    /// Held for its side effect: dropping it closes the stream.
    _stream: cpal::Stream,
    pub device_name: String,
    pub rate_hz: u32,
    pub channels: u16,
}

/// The name of the device the OS currently considers the default input.
pub fn default_device_name() -> Option<String> {
    let device = cpal::default_host().default_input_device()?;
    device_name(&device).ok()
}

/// Every input device, flagged with whether it is the current default.
pub fn list_devices() -> Result<Vec<DeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| device_name(&d).ok());

    let devices = host.input_devices().map_err(|e| AudioError::Device {
        detail: e.to_string(),
    })?;

    let mut out = Vec::new();
    for device in devices {
        // A device that cannot report a name or a config is skipped rather
        // than failing the whole list: one broken virtual device must not
        // blank the section for the real microphone next to it.
        let Ok(name) = device_name(&device) else {
            continue;
        };
        let Ok(config) = device.default_input_config() else {
            continue;
        };
        out.push(DeviceInfo {
            is_default: Some(&name) == default_name.as_ref(),
            name,
            sample_rate_hz: config.sample_rate(),
            channels: config.channels(),
        });
    }
    Ok(out)
}

/// Open the default microphone and start it, delivering audio to `tx`.
pub fn open_default(tx: mpsc::Sender<FromStream>) -> Result<OpenMic, AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;
    let device_name = device_name(&device)?;
    let supported = device
        .default_input_config()
        .map_err(|e| AudioError::Device {
            detail: e.to_string(),
        })?;

    let rate_hz = supported.sample_rate();
    let channels = supported.channels();
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    // One monomorphised builder per format the hardware might speak. The
    // conversion to f32 happens in the callback so the engine only ever sees
    // one sample type. `_ =>` is required — cpal marks the enum non-exhaustive
    // — and refusing an unknown format with its name beats guessing its
    // encoding.
    let stream = match sample_format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, tx),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, tx),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, tx),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, tx),
        cpal::SampleFormat::U32 => build::<u32>(&device, &config, tx),
        cpal::SampleFormat::I8 => build::<i8>(&device, &config, tx),
        cpal::SampleFormat::U8 => build::<u8>(&device, &config, tx),
        cpal::SampleFormat::F64 => build::<f64>(&device, &config, tx),
        other => Err(AudioError::Stream {
            detail: format!("the device speaks {other:?}, which this build does not"),
        }),
    }?;

    stream.play().map_err(|e| AudioError::Stream {
        detail: e.to_string(),
    })?;

    Ok(OpenMic {
        _stream: stream,
        device_name,
        rate_hz,
        channels,
    })
}

/// Build an input stream for one concrete sample type.
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: mpsc::Sender<FromStream>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let error_tx = tx.clone();
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data, _| {
                // An allocation per ~10 ms callback. A transcription pipeline
                // will want a ring buffer here; for metering and short probes
                // the simplicity is worth more than the microseconds.
                let frames = data.iter().map(|s| f32::from_sample(*s)).collect();
                // A send can only fail if the engine dropped the receiver,
                // which means this stream is already being torn down.
                let _ = tx.send(FromStream::Frames(frames));
            },
            move |error| {
                let _ = error_tx.send(FromStream::Failed(error.to_string()));
            },
            None,
        )
        .map_err(|e| AudioError::Stream {
            detail: e.to_string(),
        })
}
