// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Bandlimited sample-rate conversion for the prefetch thread.
//!
//! Matched-rate playback bypasses this module and copies source frames
//! bit-exactly. Rate changes use `rubato::Fft` with fixed output chunks so
//! each prefetch iteration pushes a stable number of device-rate frames.

use anyhow::{Context, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use super::prefetch::PREFETCH_CHUNK_FRAMES;

/// Streaming FFT resampler owned by the prefetch scratch (not the callback).
pub(crate) struct StreamingSrc {
    resampler: Fft<f32>,
    src_rate: u32,
    out_rate: u32,
    channels: usize,
    input: Vec<f32>,
    output: Vec<f32>,
}

impl StreamingSrc {
    /// Build a fixed-output FFT resampler for `src_rate` → `out_rate`.
    pub fn new(src_rate: u32, out_rate: u32, channels: usize) -> Result<Self> {
        let channels = channels.max(1);
        let resampler = Fft::<f32>::new(
            src_rate.max(1) as usize,
            out_rate.max(1) as usize,
            PREFETCH_CHUNK_FRAMES,
            channels,
            FixedSync::Output,
        )
        .context("create prefetch FFT resampler")?;
        let input = vec![0.0f32; resampler.input_frames_max().saturating_mul(channels)];
        let output = vec![0.0f32; resampler.output_frames_max().saturating_mul(channels)];
        Ok(Self {
            resampler,
            src_rate: src_rate.max(1),
            out_rate: out_rate.max(1),
            channels,
            input,
            output,
        })
    }

    /// Whether this instance matches the active rates and channel count.
    pub fn matches(&self, src_rate: u32, out_rate: u32, channels: usize) -> bool {
        self.src_rate == src_rate.max(1)
            && self.out_rate == out_rate.max(1)
            && self.channels == channels.max(1)
    }

    /// Clear internal delay state (seek / loop wrap / epoch bump).
    pub fn reset(&mut self) {
        self.resampler.reset();
    }

    /// Source frames required for the next [`Self::process`] call (may be 0
    /// when the resampler still owes output from its internal buffer).
    pub fn input_frames_next(&self) -> usize {
        self.resampler.input_frames_next()
    }

    /// Device frames the next [`Self::process`] will produce.
    pub fn output_frames_next(&self) -> usize {
        self.resampler.output_frames_next()
    }

    /// Resample `input_frames` interleaved source samples into device-rate
    /// output. Returns `(source_frames_consumed, device_frames_written)`.
    ///
    /// When [`Self::input_frames_next`] is 0, pass an empty / ignored input and
    /// still call this to flush buffered output (rubato FixedSync::Output).
    ///
    /// `input` must hold at least `input_frames * channels` samples when
    /// `input_frames > 0`. Missing tails (past EOF) should already be
    /// zero-filled by the caller.
    pub fn process(
        &mut self,
        input: &[f32],
        input_frames: usize,
        gathered: &mut [f32],
    ) -> Result<(usize, usize)> {
        let channels = self.channels;
        let need = self.resampler.input_frames_next();
        let frames_in = input_frames.min(need);
        let need_samples = need.saturating_mul(channels);
        if self.input.len() < need_samples {
            self.input.resize(need_samples, 0.0);
        }
        if need_samples > 0 {
            self.input[..need_samples].fill(0.0);
            let copy = frames_in.saturating_mul(channels).min(input.len());
            if copy > 0 {
                self.input[..copy].copy_from_slice(&input[..copy]);
            }
        }

        let out_frames = self.resampler.output_frames_next().max(1);
        let out_samples = out_frames.saturating_mul(channels);
        if self.output.len() < out_samples {
            self.output.resize(out_samples, 0.0);
        }
        self.output[..out_samples].fill(0.0);

        let input_adapter = InterleavedSlice::new(&self.input[..need_samples], channels, need)
            .map_err(|err| anyhow::anyhow!("wrap SRC input: {err}"))?;
        let mut output_adapter =
            InterleavedSlice::new_mut(&mut self.output[..out_samples], channels, out_frames)
                .map_err(|err| anyhow::anyhow!("wrap SRC output: {err}"))?;
        let (consumed, written) = self
            .resampler
            .process_into_buffer(&input_adapter, &mut output_adapter, None)
            .context("prefetch SRC")?;
        let write_samples = written.saturating_mul(channels).min(gathered.len());
        if write_samples > 0 {
            gathered[..write_samples].copy_from_slice(&self.output[..write_samples]);
        }
        Ok((consumed, written))
    }
}
