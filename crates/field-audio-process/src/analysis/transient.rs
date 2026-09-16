// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Fast/slow peak-envelope transient detection.

use dasp::envelope::detect::Peak;
use dasp::envelope::Detector;

use field_audio_model::{NewMarker, MARKER_TYPE_TRANSIENT};

use super::AnalysisSink;

/// Fast/slow peak-envelope transient detector.
#[derive(Debug)]
pub struct TransientDetectOp {
    fast: Vec<Detector<[f32; 1], Peak>>,
    slow: Vec<Detector<[f32; 1], Peak>>,
    /// Absolute frame offset at the start of the current pass.
    frame_cursor: u64,
    /// Frames to skip after an onset.
    refractory: u64,
    next_allowed: Vec<u64>,
    threshold: f32,
    detections: Vec<u64>,
}

impl TransientDetectOp {
    const FAST_SECS: f32 = 0.005;
    const SLOW_SECS: f32 = 0.05;
    const REFRACTORY_SECS: f32 = 0.05;
    const THRESHOLD: f32 = 0.05;

    /// Create detectors for `channel_count` at `sample_rate`.
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let fast_f = (Self::FAST_SECS * sample_rate as f32).max(1.0);
        let slow_f = (Self::SLOW_SECS * sample_rate as f32).max(1.0);
        let refractory = ((Self::REFRACTORY_SECS * sample_rate as f32) as u64).max(1);
        Self {
            fast: (0..channel_count)
                .map(|_| Detector::peak(fast_f, fast_f))
                .collect(),
            slow: (0..channel_count)
                .map(|_| Detector::peak(slow_f, slow_f))
                .collect(),
            frame_cursor: 0,
            refractory,
            next_allowed: vec![0; channel_count],
            threshold: Self::THRESHOLD,
            detections: Vec::new(),
        }
    }

    /// Reset pass state before reading from frame 0 (or a target start).
    ///
    /// Envelope followers keep their state so a short pre-roll can warm them
    /// before [`Self::begin`] moves the cursor to the real range start.
    pub fn begin(&mut self, start_frame: u64) {
        self.frame_cursor = start_frame;
        self.detections.clear();
        for allowed in &mut self.next_allowed {
            *allowed = start_frame;
        }
    }

    /// Seconds of audio before a ranged start used to settle followers.
    pub fn warmup_frames(sample_rate: u32) -> u64 {
        ((Self::SLOW_SECS * 2.0 * sample_rate as f32) as u64).max(1)
    }

    /// Consume interleaved timeline progress across all channels in `planar`.
    ///
    /// Channels are scanned per-frame; an onset on any channel emits one marker.
    pub fn consume_planar(&mut self, planar: &[Vec<f32>]) {
        if planar.is_empty() {
            return;
        }
        let frames = planar[0].len();
        for i in 0..frames {
            let frame = self.frame_cursor + i as u64;
            let mut hit = false;
            for ch in 0..planar.len() {
                let sample = planar[ch][i];
                let fast = self.fast[ch].next([sample])[0];
                let slow = self.slow[ch].next([sample])[0];
                if frame >= self.next_allowed[ch] && (fast - slow) > self.threshold {
                    hit = true;
                    self.next_allowed[ch] = frame + self.refractory;
                }
            }
            if hit {
                self.detections.push(frame);
            }
        }
        self.frame_cursor += frames as u64;
    }

    /// Drain detected onset frames into `sink` as Transient markers.
    pub fn finish(&mut self, sink: &mut AnalysisSink) {
        for frame in self.detections.drain(..) {
            sink.markers
                .push(NewMarker::new(frame, MARKER_TYPE_TRANSIENT, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_detects_click_in_silence() {
        let mut op = TransientDetectOp::new(8_000, 1);
        op.begin(0);
        let mut samples = vec![0.0f32; 8_000];
        for s in &mut samples[4_000..4_040] {
            *s = 1.0;
        }
        op.consume_planar(&[samples]);
        let mut sink = AnalysisSink::default();
        op.finish(&mut sink);
        assert!(
            !sink.markers.is_empty(),
            "expected at least one transient marker"
        );
        let frame = sink.markers[0].frame;
        assert!((3_900..=4_100).contains(&frame));
    }
}
