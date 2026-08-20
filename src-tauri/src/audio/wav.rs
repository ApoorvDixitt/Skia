// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! A minimal 16-bit PCM mono WAV writer.
//!
//! Hand-rolled rather than a crate, for the same reason `providers` wrote its
//! own SSE parser: the format needed here is one fixed header plus samples,
//! about forty lines, and a dependency would bring in a full audio I/O stack
//! to avoid writing them. 16-bit integer PCM because the file exists to be
//! *listened to* — QuickTime, Explorer preview, and every player alive open
//! it — and to be fed to a transcriber, which wants exactly this shape.

use std::path::Path;

use super::AudioError;

/// Serialise mono f32 samples as a complete 16-bit PCM WAV file.
///
/// Samples are clamped to [-1, 1] before scaling: the capture path can
/// legitimately exceed full scale after downmix averaging of an already-hot
/// signal, and integer wraparound would turn a loud moment into a burst of
/// noise at the opposite polarity.
pub fn wav_bytes_mono_16bit(rate_hz: u32, samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);

    // RIFF container. The size fields are little-endian byte counts; getting
    // one wrong produces a file some players truncate and others reject, so
    // the tests below check them explicitly.
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt chunk: PCM, mono, 16-bit.
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt payload size
    out.extend_from_slice(&1u16.to_le_bytes()); // format 1 = integer PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels
    out.extend_from_slice(&rate_hz.to_le_bytes());
    out.extend_from_slice(&(rate_hz * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for &sample in samples {
        #[allow(clippy::cast_possible_truncation)]
        let quantised = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&quantised.to_le_bytes());
    }
    out
}

/// Write the WAV to disk, creating parent directories.
pub fn write_mono_16bit(path: &Path, rate_hz: u32, samples: &[f32]) -> Result<(), AudioError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| AudioError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
    }
    std::fs::write(path, wav_bytes_mono_16bit(rate_hz, samples)).map_err(|source| AudioError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_is_a_valid_riff_wave_with_correct_sizes() {
        let bytes = wav_bytes_mono_16bit(16_000, &[0.0; 100]);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(bytes.len(), 44 + 200, "44-byte header + 2 bytes a sample");

        // RIFF size = file minus the 8-byte RIFF preamble.
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(riff as usize, bytes.len() - 8);
        // data size = samples * 2.
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert_eq!(data, 200);
    }

    #[test]
    fn format_fields_say_mono_16_bit_pcm_at_the_given_rate() {
        let bytes = wav_bytes_mono_16bit(16_000, &[0.0]);
        let u16_at = |i: usize| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        assert_eq!(u16_at(20), 1, "integer PCM");
        assert_eq!(u16_at(22), 1, "mono");
        assert_eq!(u32_at(24), 16_000, "sample rate");
        assert_eq!(u32_at(28), 32_000, "byte rate");
        assert_eq!(u16_at(32), 2, "block align");
        assert_eq!(u16_at(34), 16, "bit depth");
    }

    #[test]
    fn samples_are_scaled_clamped_and_little_endian() {
        let bytes = wav_bytes_mono_16bit(16_000, &[0.0, 1.0, -1.0, 2.0, 0.5]);
        let sample_at =
            |n: usize| i16::from_le_bytes(bytes[44 + n * 2..46 + n * 2].try_into().unwrap());
        assert_eq!(sample_at(0), 0);
        assert_eq!(sample_at(1), 32767);
        assert_eq!(sample_at(2), -32767);
        assert_eq!(sample_at(3), 32767, "out of range clamps, never wraps");
        assert_eq!(sample_at(4), 16384, "0.5 rounds to nearest");
    }

    #[test]
    fn writing_creates_the_parent_directory_and_a_readable_file() {
        let dir = std::env::temp_dir().join(format!("skia-wav-{}", std::process::id()));
        let path = dir.join("probes").join("test.wav");
        write_mono_16bit(&path, 16_000, &[0.25; 16]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 32);
        std::fs::remove_dir_all(&dir).ok();
    }
}
