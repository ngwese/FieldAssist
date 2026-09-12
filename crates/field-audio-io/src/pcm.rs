// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use crate::spec::PcmFormat;

/// Clamp a sample to the unit interval `[-1.0, 1.0]`.
pub fn clamp_unit(sample: f32) -> f32 {
    sample.clamp(-1.0, 1.0)
}

/// Convert a unit-float sample to a signed integer of `bits` width.
pub fn to_signed(sample: f32, bits: u32) -> i32 {
    let max = (1i32 << (bits - 1)) - 1;
    (clamp_unit(sample) * max as f32).round() as i32
}

/// Convert a unit-float sample to unsigned 8-bit PCM.
pub fn to_u8(sample: f32) -> u8 {
    ((clamp_unit(sample) * 0.5 + 0.5) * 255.0).round() as u8
}

/// Shortest channel length across planar buffers.
pub fn planar_frames(planar: &[Vec<f32>]) -> usize {
    planar.iter().map(|ch| ch.len()).min().unwrap_or(0)
}

/// Interleave planar f32 into frame-major order.
pub fn interleave_f32(planar: &[Vec<f32>]) -> Vec<f32> {
    let frames = planar_frames(planar);
    let channels = planar.len();
    let mut out = vec![0.0; frames * channels];
    for frame in 0..frames {
        for (ch, plane) in planar.iter().enumerate() {
            out[frame * channels + ch] = plane[frame];
        }
    }
    out
}

/// Interleave planar f32 as signed integers of `bits` width.
pub fn interleave_i32(planar: &[Vec<f32>], bits: u32) -> Vec<i32> {
    let frames = planar_frames(planar);
    let channels = planar.len();
    let mut out = vec![0; frames * channels];
    for frame in 0..frames {
        for (ch, plane) in planar.iter().enumerate() {
            out[frame * channels + ch] = to_signed(plane[frame], bits);
        }
    }
    out
}

/// Select a subset of planar channels by index.
pub fn select_channels(planar: &[Vec<f32>], indices: &[usize]) -> Vec<Vec<f32>> {
    indices
        .iter()
        .map(|index| planar.get(*index).cloned().unwrap_or_default())
        .collect()
}

/// Integer bit depth for signed PCM formats, if applicable.
pub fn bits_for_integer(format: PcmFormat) -> Option<u32> {
    match format {
        PcmFormat::S8 => Some(8),
        PcmFormat::S16 => Some(16),
        PcmFormat::S24 => Some(24),
        PcmFormat::S32 => Some(32),
        _ => None,
    }
}
