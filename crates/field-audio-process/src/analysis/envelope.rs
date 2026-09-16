// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Peak-envelope follower analysis op.

use dasp::envelope::detect::Peak;
use dasp::envelope::Detector;

use super::minmax::PEAK_BLOCK;

/// Peak-envelope follower: 5 ms attack / 300 ms release, hop-downsampled bins.
#[derive(Debug)]
pub struct EnvelopePeakOp {
    detectors: Vec<Detector<[f32; 1], Peak>>,
    hop: usize,
    hop_acc: Vec<f32>,
    hop_count: Vec<usize>,
    sample_rate: u32,
}

impl EnvelopePeakOp {
    /// Attack window in seconds.
    pub const ATTACK_SECS: f32 = 0.005;
    /// Release window in seconds.
    pub const RELEASE_SECS: f32 = 0.3;

    /// Create detectors for `channel_count` channels at `sample_rate`.
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let attack = (Self::ATTACK_SECS * sample_rate as f32).max(1.0);
        let release = (Self::RELEASE_SECS * sample_rate as f32).max(1.0);
        let detectors = (0..channel_count)
            .map(|_| Detector::peak(attack, release))
            .collect();
        Self {
            detectors,
            hop: PEAK_BLOCK,
            hop_acc: vec![0.0; channel_count],
            hop_count: vec![0; channel_count],
            sample_rate,
        }
    }

    /// Frames folded into each output bin.
    pub fn hop_frames(&self) -> usize {
        self.hop
    }

    /// Sample rate used to size the follower.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Consume planar channel samples and append hop bins to `out`.
    pub fn consume_channel(&mut self, channel: usize, samples: &[f32], out: &mut Vec<f32>) {
        let detector = &mut self.detectors[channel];
        for &sample in samples {
            let env = detector.next([sample])[0];
            let acc = &mut self.hop_acc[channel];
            let count = &mut self.hop_count[channel];
            *acc = acc.max(env);
            *count += 1;
            if *count >= self.hop {
                out.push(*acc);
                *acc = 0.0;
                *count = 0;
            }
        }
    }

    /// Flush a partial hop bin (if any) into `out`.
    pub fn flush_channel(&mut self, channel: usize, out: &mut Vec<f32>) {
        if self.hop_count[channel] > 0 {
            out.push(self.hop_acc[channel]);
            self.hop_acc[channel] = 0.0;
            self.hop_count[channel] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_decays_after_impulse() {
        let mut op = EnvelopePeakOp::new(1_000, 1);
        let mut impulse = vec![0.0f32; 400];
        impulse[0] = 1.0;
        let mut bins = Vec::new();
        op.consume_channel(0, &impulse, &mut bins);
        op.flush_channel(0, &mut bins);
        assert!(!bins.is_empty());
        if bins.len() >= 2 {
            assert!(bins[0] >= bins[bins.len() - 1]);
        }
    }
}
