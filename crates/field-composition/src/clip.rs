// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use field_audio_model::MediaId;
use field_audio_process::PEAK_BLOCK;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// ClipId.
pub struct ClipId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// ClipMarkerId.
pub struct ClipMarkerId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// ClipSource.
pub struct ClipSource {
    /// media_id.
    pub media_id: MediaId,
    /// offset.
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// ClipMarker.
pub struct ClipMarker {
    /// id.
    pub id: ClipMarkerId,
    /// offset.
    pub offset: u64,
    /// color.
    pub color: Option<[f32; 4]>,
    /// label_type.
    pub label_type: Option<String>,
    /// message.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
/// ClipCache.
pub struct ClipCache {
    /// min.
    pub min: Option<f32>,
    /// max.
    pub max: Option<f32>,
    /// peaks.
    pub peaks: Vec<Vec<(f32, f32)>>,
}

impl ClipCache {
    /// `is_empty`.
    pub fn is_empty(&self) -> bool {
        self.min.is_none() && self.peaks.is_empty()
    }

    /// True when overview bins are absent or a channel vec was left empty.
    /// Initial load and roll clear the cache; splits keep suffix bins.
    pub fn is_missing_peaks(&self) -> bool {
        self.peaks.is_empty() || self.peaks.iter().any(|channel| channel.is_empty())
    }

    /// `split`.
    pub fn split(&self, at: u64) -> (Self, Self) {
        let at = at as usize;
        let block = PEAK_BLOCK;
        let bin = at / block;
        let mut left_peaks = Vec::with_capacity(self.peaks.len());
        let mut right_peaks = Vec::with_capacity(self.peaks.len());
        for channel in &self.peaks {
            let split = bin.min(channel.len());
            left_peaks.push(channel[..split].to_vec());
            // Bins stay aligned to the pre-split origin. Reusing the suffix
            // after an unaligned cut phase-shifts overview by at most one
            // PEAK_BLOCK (sub-pixel at typical zoom). Rebuilding the rest of
            // the file from PCM is much more expensive.
            right_peaks.push(channel[split..].to_vec());
        }
        (
            Self {
                min: extrema_of(&left_peaks).map(|(mn, _)| mn),
                max: extrema_of(&left_peaks).map(|(_, mx)| mx),
                peaks: left_peaks,
            },
            Self {
                min: extrema_of(&right_peaks).map(|(mn, _)| mn),
                max: extrema_of(&right_peaks).map(|(_, mx)| mx),
                peaks: right_peaks,
            },
        )
    }
}

fn extrema_of(peaks: &[Vec<(f32, f32)>]) -> Option<(f32, f32)> {
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    let mut any = false;
    for channel in peaks {
        for &(pmin, pmax) in channel {
            min = min.min(pmin);
            max = max.max(pmax);
            any = true;
        }
    }
    any.then_some((min, max))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Clip.
pub struct Clip {
    /// id.
    pub id: ClipId,
    /// len.
    pub len: u64,
    /// source.
    pub source: Option<ClipSource>,
    /// fade_in.
    pub fade_in: u64,
    /// fade_out.
    pub fade_out: u64,
    /// cache.
    pub cache: ClipCache,
    /// markers.
    pub markers: Vec<ClipMarker>,
}

impl Clip {
    /// `silence`.
    pub fn silence(id: ClipId, len: u64) -> Self {
        Self {
            id,
            len,
            source: None,
            fade_in: 0,
            fade_out: 0,
            cache: ClipCache::default(),
            markers: Vec::new(),
        }
    }

    /// `from_media`.
    pub fn from_media(id: ClipId, media_id: MediaId, offset: u64, len: u64) -> Self {
        Self {
            id,
            len,
            source: Some(ClipSource { media_id, offset }),
            fade_in: 0,
            fade_out: 0,
            cache: ClipCache::default(),
            markers: Vec::new(),
        }
    }

    /// `needs_peak_cache`.
    pub fn needs_peak_cache(&self) -> bool {
        self.source.is_some() && self.cache.is_missing_peaks()
    }

    /// `gain_at`.
    pub fn gain_at(&self, local: u64) -> f32 {
        if self.len == 0 || local >= self.len {
            return 0.0;
        }
        let mut gain = 1.0;
        if self.fade_in > 0 && local < self.fade_in {
            gain *= local as f32 / self.fade_in as f32;
        }
        if self.fade_out > 0 && local + self.fade_out >= self.len {
            let faded = local + self.fade_out - self.len;
            gain *= 1.0 - faded as f32 / self.fade_out as f32;
        }
        gain.clamp(0.0, 1.0)
    }

    /// `split`.
    pub fn split(&self, at: u64, right_id: ClipId) -> (Clip, Clip) {
        let at = at.min(self.len);
        let left_len = at;
        let right_len = self.len - at;
        let (left_markers, mut right_markers) = split_markers(&self.markers, at);
        for marker in &mut right_markers {
            marker.offset -= at;
        }

        let left_fade_in = self.fade_in.min(left_len);
        let right_fade_in = self.fade_in.saturating_sub(left_len).min(right_len);
        let left_fade_out = self.fade_out.saturating_sub(right_len).min(left_len);
        let right_fade_out = self.fade_out.min(right_len);
        let (left_cache, right_cache) = self.cache.split(at);

        let left = Clip {
            id: self.id,
            len: left_len,
            source: self.source.clone(),
            fade_in: left_fade_in,
            fade_out: left_fade_out,
            cache: left_cache,
            markers: left_markers,
        };
        let right_source = self.source.as_ref().map(|source| ClipSource {
            media_id: source.media_id,
            offset: source.offset + at,
        });
        let right = Clip {
            id: right_id,
            len: right_len,
            source: right_source,
            fade_in: right_fade_in,
            fade_out: right_fade_out,
            cache: right_cache,
            markers: right_markers,
        };
        (left, right)
    }

    /// `with_rolled_offset`.
    pub fn with_rolled_offset(&self, delta: i64, media_frames: u64) -> Clip {
        let Some(source) = &self.source else {
            return self.clone();
        };
        let max_offset = media_frames.saturating_sub(self.len);
        let rolled = if delta >= 0 {
            source.offset.saturating_add(delta as u64)
        } else {
            source.offset.saturating_sub(delta.unsigned_abs())
        };
        let mut clip = self.clone();
        clip.source = Some(ClipSource {
            media_id: source.media_id,
            offset: rolled.min(max_offset),
        });
        clip.cache = ClipCache::default();
        clip
    }
}

fn split_markers(markers: &[ClipMarker], at: u64) -> (Vec<ClipMarker>, Vec<ClipMarker>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for marker in markers {
        if marker.offset < at {
            left.push(marker.clone());
        } else {
            right.push(marker.clone());
        }
    }
    (left, right)
}

#[derive(Debug, Clone, PartialEq)]
/// ClipSpan.
pub struct ClipSpan {
    /// start.
    pub start: u64,
    /// clip.
    pub clip: Arc<Clip>,
}

impl ClipSpan {
    /// `end`.
    pub fn end(&self) -> u64 {
        self.start + self.clip.len
    }

    /// `contains`.
    pub fn contains(&self, frame: u64) -> bool {
        frame >= self.start && frame < self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_divides_len_source_and_markers() {
        let clip = Clip {
            id: ClipId(1),
            len: 10,
            source: Some(ClipSource {
                media_id: MediaId(7),
                offset: 100,
            }),
            fade_in: 3,
            fade_out: 2,
            cache: ClipCache {
                min: Some(-0.5),
                max: Some(0.5),
                peaks: vec![vec![(-0.4, 0.1), (-0.2, 0.5)]],
            },
            markers: vec![
                ClipMarker {
                    id: ClipMarkerId(1),
                    offset: 2,
                    color: None,
                    label_type: None,
                    message: None,
                },
                ClipMarker {
                    id: ClipMarkerId(2),
                    offset: 7,
                    color: None,
                    label_type: None,
                    message: None,
                },
            ],
        };
        let (left, right) = clip.split(4, ClipId(2));
        assert_eq!(left.len, 4);
        assert_eq!(right.len, 6);
        assert_eq!(left.source.as_ref().unwrap().offset, 100);
        assert_eq!(right.source.as_ref().unwrap().offset, 104);
        assert_eq!(left.markers.len(), 1);
        assert_eq!(right.markers[0].offset, 3);
        assert_eq!(left.fade_in, 3);
        assert_eq!(right.fade_in, 0);
        assert_eq!(left.fade_out, 0);
        assert_eq!(right.fade_out, 2);
        assert_eq!(left.cache.peaks[0].len(), 0);
        assert_eq!(right.cache.peaks[0], [(-0.4, 0.1), (-0.2, 0.5)]);
        assert!(right.cache.min.is_some());
        assert!(right.cache.max.is_some());
    }

    #[test]
    fn split_reuses_unaligned_right_peaks() {
        let block = PEAK_BLOCK as u64;
        let clip = Clip {
            id: ClipId(1),
            len: block * 2,
            source: Some(ClipSource {
                media_id: MediaId(1),
                offset: 0,
            }),
            fade_in: 0,
            fade_out: 0,
            cache: ClipCache {
                min: Some(-1.0),
                max: Some(1.0),
                peaks: vec![vec![(-1.0, 0.2), (-0.3, 1.0)]],
            },
            markers: Vec::new(),
        };
        let (left, right) = clip.split(block + 4, ClipId(2));
        assert_eq!(left.cache.peaks[0], [(-1.0, 0.2)]);
        assert_eq!(right.cache.peaks[0], [(-0.3, 1.0)]);
        assert!(!left.needs_peak_cache());
        assert!(!right.needs_peak_cache());
    }

    #[test]
    fn split_divides_peak_bins() {
        let clip = Clip {
            id: ClipId(1),
            len: PEAK_BLOCK as u64 * 2,
            source: Some(ClipSource {
                media_id: MediaId(1),
                offset: 0,
            }),
            fade_in: 0,
            fade_out: 0,
            cache: ClipCache {
                min: Some(-1.0),
                max: Some(1.0),
                peaks: vec![vec![(-1.0, 0.2), (-0.3, 1.0)]],
            },
            markers: Vec::new(),
        };
        let (left, right) = clip.split(PEAK_BLOCK as u64, ClipId(2));
        assert_eq!(left.cache.peaks[0], [(-1.0, 0.2)]);
        assert_eq!(right.cache.peaks[0], [(-0.3, 1.0)]);
        assert_eq!(left.cache.min, Some(-1.0));
        assert_eq!(left.cache.max, Some(0.2));
        assert_eq!(right.cache.min, Some(-0.3));
        assert_eq!(right.cache.max, Some(1.0));
    }

    #[test]
    fn roll_clamps_to_media() {
        let clip = Clip::from_media(ClipId(1), MediaId(1), 8, 4);
        let rolled = clip.with_rolled_offset(10, 12);
        assert_eq!(rolled.source.unwrap().offset, 8);
        let rolled = clip.with_rolled_offset(-20, 12);
        assert_eq!(rolled.source.unwrap().offset, 0);
    }
}
