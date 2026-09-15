// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Incremental analysis operations over planar PCM blocks.

use dasp::envelope::detect::Peak;
use dasp::envelope::Detector;

use field_audio_model::MARKER_TYPE_TRANSIENT;

use crate::peaks::PEAK_BLOCK;

/// Stable id for a built-in analysis operation / stream kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisKind {
    /// Overview min/max bins (waveform paint).
    MinMax,
    /// Peak envelope follower with a fixed smoothing window.
    EnvelopePeak,
    /// Onset / transient marker detection.
    Transients,
}

impl AnalysisKind {
    /// Stable string id used in caches and progress labels.
    pub fn id(self) -> &'static str {
        match self {
            Self::MinMax => "minmax",
            Self::EnvelopePeak => "envelope_peak",
            Self::Transients => "transients",
        }
    }

    /// Human-readable background job label.
    pub fn progress_label(self) -> &'static str {
        match self {
            Self::MinMax => "building peaks",
            Self::EnvelopePeak => "building envelope",
            Self::Transients => "detecting transients",
        }
    }
}

/// Shared context passed at the start of an analysis pass.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisCtx {
    /// Composition sample rate in Hz.
    pub sample_rate: u32,
    /// Number of planar channels in the read path.
    pub channel_count: usize,
}

/// Marker emission collected during a pass (applied by the host).
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisMarker {
    /// Timeline frame of the marker.
    pub frame: u64,
    /// Marker type name (for example `"Transient"`).
    pub marker_type: String,
    /// Optional note.
    pub note: Option<String>,
}

/// Sink for discrete analysis outputs produced at pass end (or mid-pass flush).
#[derive(Debug, Default)]
pub struct AnalysisSink {
    /// Markers to insert after replacing the prior set in-range.
    pub markers: Vec<AnalysisMarker>,
}

/// Fold one planar channel chunk into overview `(min, max)` bins.
pub fn fold_minmax_bins(
    samples: &[f32],
    peaks: &mut Vec<(f32, f32)>,
    global_min: &mut f32,
    global_max: &mut f32,
) {
    for chunk in samples.chunks(PEAK_BLOCK) {
        let mut pmin = f32::MAX;
        let mut pmax = f32::MIN;
        for &s in chunk {
            pmin = pmin.min(s);
            pmax = pmax.max(s);
            *global_min = (*global_min).min(s);
            *global_max = (*global_max).max(s);
        }
        peaks.push(if pmin <= pmax {
            (pmin, pmax)
        } else {
            (0.0, 0.0)
        });
    }
}

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
            sink.markers.push(AnalysisMarker {
                frame,
                marker_type: MARKER_TYPE_TRANSIENT.into(),
                note: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peaks::build_peaks;

    #[test]
    fn envelope_decays_after_impulse() {
        let mut op = EnvelopePeakOp::new(1_000, 1);
        let mut impulse = vec![0.0f32; 400];
        impulse[0] = 1.0;
        let mut bins = Vec::new();
        op.consume_channel(0, &impulse, &mut bins);
        op.flush_channel(0, &mut bins);
        assert!(!bins.is_empty());
        // Later bins should be quieter than the first hop that saw the impulse.
        if bins.len() >= 2 {
            assert!(bins[0] >= bins[bins.len() - 1]);
        }
    }

    #[test]
    fn transient_detects_click_in_silence() {
        let mut op = TransientDetectOp::new(8_000, 1);
        op.begin(0);
        let mut samples = vec![0.0f32; 8_000];
        // Short burst so the fast follower leads the slow one.
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

    #[test]
    fn fold_minmax_matches_build_peaks() {
        let samples: Vec<f32> = (0..1_000).map(|i| (i as f32) / 1000.0 - 0.5).collect();
        let expected = build_peaks(&samples);
        let mut got = Vec::new();
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        fold_minmax_bins(&samples, &mut got, &mut min, &mut max);
        assert_eq!(got, expected);
    }
}
