// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use field_core::{encode_file_url, resolve_file_url};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Stable media entry id within a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MediaId(pub u64);

/// Reference to a media file or in-memory samples.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MediaRef {
    /// Pool-local id.
    pub id: MediaId,
    /// Stored URL (relative path, `file://`, or `memory://`).
    #[cfg_attr(feature = "serde", serde(rename = "url", alias = "path"))]
    pub url: String,
    /// Resolved filesystem path (skipped on disk).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub path: PathBuf,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channel_count: usize,
    /// Total frames.
    pub frame_count: u64,
    /// Bits per sample when known.
    pub bits_per_sample: Option<u32>,
    /// Source file size in bytes.
    pub size_bytes: u64,
    /// Last modified time.
    #[cfg_attr(feature = "serde", serde(with = "rfc3339"))]
    pub modified: SystemTime,
    /// Container label.
    pub container_format: String,
    /// Codec label.
    pub codec: String,
    /// Optional content hash.
    pub hash: Option<String>,
    /// Optional fully decoded samples (skipped on disk).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub samples: Option<Arc<Vec<Vec<f32>>>>,
}

impl MediaRef {
    /// Build an in-memory media ref from planar samples.
    pub fn from_memory(id: MediaId, sample_rate: u32, samples: Vec<Vec<f32>>) -> Self {
        let frame_count = samples.first().map(|ch| ch.len() as u64).unwrap_or(0);
        let channel_count = samples.len();
        Self {
            id,
            url: format!("memory://{id:?}"),
            path: PathBuf::from(format!("memory://{id:?}")),
            sample_rate,
            channel_count,
            frame_count,
            bits_per_sample: Some(32),
            size_bytes: 0,
            modified: SystemTime::UNIX_EPOCH,
            container_format: "memory".into(),
            codec: "pcm".into(),
            hash: None,
            samples: Some(Arc::new(samples)),
        }
    }

    /// Encode `path` into `url` relative to `base` when possible.
    pub fn prepare_url(&mut self, base: Option<&Path>) {
        let lossy = self.path.to_string_lossy();
        if lossy.starts_with("memory://") {
            self.url = lossy.into_owned();
            return;
        }
        self.url = encode_file_url(&self.path, base);
    }

    /// Resolve `url` into `path` using `base` for relative URLs.
    pub fn resolve_url(&mut self, base: Option<&Path>) -> anyhow::Result<()> {
        if self.url.starts_with("memory://")
            || self.url.is_empty() && self.path.to_string_lossy().starts_with("memory://")
        {
            if self.path.as_os_str().is_empty() {
                self.path = PathBuf::from(&self.url);
            }
            return Ok(());
        }
        if self.url.is_empty() && !self.path.as_os_str().is_empty() {
            self.url = self.path.to_string_lossy().into_owned();
        }
        self.path = resolve_file_url(&self.url, base)?;
        Ok(())
    }
}

#[cfg(feature = "serde")]
mod rfc3339 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format_rfc3339(*time))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        parse_rfc3339(&text).map_err(serde::de::Error::custom)
    }

    pub fn format_rfc3339(time: SystemTime) -> String {
        let dur = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let (year, month, day, hour, min, sec) = civil_from_days(dur.as_secs());
        let nanos = dur.subsec_nanos();
        if nanos == 0 {
            format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
        } else {
            format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{nanos:09}Z")
        }
    }

    pub fn parse_rfc3339(text: &str) -> Result<SystemTime, String> {
        let text = text.trim();
        let (date, rest) = text
            .split_once('T')
            .or_else(|| text.split_once('t'))
            .ok_or_else(|| format!("invalid RFC3339 timestamp: {text}"))?;
        let mut parts = date.split('-');
        let year: i32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid RFC3339 date: {text}"))?;
        let month: u32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid RFC3339 date: {text}"))?;
        let day: u32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid RFC3339 date: {text}"))?;
        let rest = rest
            .strip_suffix('Z')
            .or_else(|| rest.strip_suffix('z'))
            .ok_or_else(|| format!("RFC3339 timestamp must be UTC (ending in Z): {text}"))?;
        let (hms, frac) = match rest.split_once('.') {
            Some((hms, frac)) => (hms, Some(frac)),
            None => (rest, None),
        };
        let mut hms_parts = hms.split(':');
        let hour: u32 = hms_parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid RFC3339 time: {text}"))?;
        let min: u32 = hms_parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid RFC3339 time: {text}"))?;
        let sec: u32 = hms_parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid RFC3339 time: {text}"))?;
        let nanos = match frac {
            Some(frac) if frac.is_empty() => 0u32,
            Some(frac) => {
                let mut padded = frac.to_string();
                padded.truncate(9);
                while padded.len() < 9 {
                    padded.push('0');
                }
                padded
                    .parse()
                    .map_err(|_| format!("invalid RFC3339 fraction: {text}"))?
            }
            None => 0,
        };
        let days = days_from_civil(year, month, day)?;
        let secs = days
            .checked_mul(86_400)
            .and_then(|d| d.checked_add(i64::from(hour) * 3600))
            .and_then(|d| d.checked_add(i64::from(min) * 60))
            .and_then(|d| d.checked_add(i64::from(sec)))
            .ok_or_else(|| format!("RFC3339 timestamp out of range: {text}"))?;
        if secs < 0 {
            return Err(format!("RFC3339 timestamp before Unix epoch: {text}"));
        }
        Ok(UNIX_EPOCH + Duration::new(secs as u64, nanos))
    }

    fn civil_from_days(unix_secs: u64) -> (i32, u32, u32, u32, u32, u32) {
        let days = (unix_secs / 86_400) as i64;
        let rem = (unix_secs % 86_400) as u32;
        let hour = rem / 3600;
        let min = (rem % 3600) / 60;
        let sec = rem % 60;
        let (year, month, day) = civil_from_unix_days(days);
        (year, month, day, hour, min, sec)
    }

    fn civil_from_unix_days(mut z: i64) -> (i32, u32, u32) {
        z += 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y as i32, m as u32, d as u32)
    }

    fn days_from_civil(year: i32, month: u32, day: u32) -> Result<i64, String> {
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Err(format!("invalid calendar date {year}-{month:02}-{day:02}"));
        }
        let y = if month <= 2 {
            i64::from(year) - 1
        } else {
            i64::from(year)
        };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u64;
        let mp = if month > 2 {
            u64::from(month) - 3
        } else {
            u64::from(month) + 9
        };
        let doy = (153 * mp + 2) / 5 + u64::from(day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Ok(era * 146_097 + doe as i64 - 719_468)
    }
}

#[cfg(all(test, feature = "serde"))]
mod rfc3339_tests {
    use super::rfc3339::{format_rfc3339, parse_rfc3339};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn rfc3339_round_trips_epoch_and_nanos() {
        let t = UNIX_EPOCH + Duration::new(1_693_612_800, 123_456_789);
        let text = format_rfc3339(t);
        assert!(text.ends_with('Z'));
        assert_eq!(parse_rfc3339(&text).unwrap(), t);
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z").unwrap(), UNIX_EPOCH);
    }
}

/// Pool of media refs keyed by [`MediaId`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MediaPool {
    next_id: u64,
    media: HashMap<u64, MediaRef>,
}

impl MediaPool {
    /// Empty pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `media`, assigning an id when `media.id` is zero.
    pub fn insert(&mut self, mut media: MediaRef) -> MediaId {
        if media.id.0 == 0 {
            self.next_id += 1;
            media.id = MediaId(self.next_id);
        } else {
            self.next_id = self.next_id.max(media.id.0);
        }
        let id = media.id;
        self.media.insert(id.0, media);
        id
    }

    /// Look up media by id.
    pub fn get(&self, id: MediaId) -> Option<&MediaRef> {
        self.media.get(&id.0)
    }

    /// Iterate all media refs.
    pub fn iter(&self) -> impl Iterator<Item = &MediaRef> {
        self.media.values()
    }

    /// First media by lowest id, if any.
    pub fn first(&self) -> Option<&MediaRef> {
        let min_id = self.media.keys().copied().min()?;
        self.media.get(&min_id)
    }

    /// Whether the pool has no entries.
    pub fn is_empty(&self) -> bool {
        self.media.is_empty()
    }

    /// Consume the pool into id-sorted refs.
    pub fn into_refs(self) -> Vec<MediaRef> {
        let mut refs: Vec<_> = self.media.into_values().collect();
        refs.sort_by_key(|m| m.id.0);
        refs
    }
}
