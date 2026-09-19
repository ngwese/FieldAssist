// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Media identity, descriptors, and pool.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use field_core::{encode_file_url, resolve_file_url};
#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Stable media identity derived from basename + audio/file stats (not mtime).
///
/// Display / serde form is `media:<64 hex>` (blake3 of the identity tuple).
/// `modified` is intentionally **not** hashed: a filesystem touch must not mint
/// a new id or session reload and multi-doc sharing break. Freshness (including
/// mtime) is checked separately via [`descriptor_mismatch`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaId(pub [u8; 32]);

impl MediaId {
    /// Prefix used in Display / serde.
    pub const PREFIX: &'static str = "media:";

    /// Hex string without the `media:` prefix.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// Build from a raw blake3 digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for MediaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", Self::PREFIX, self.to_hex())
    }
}

impl fmt::Debug for MediaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaId({self})")
    }
}

impl FromStr for MediaId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let hex = s
            .strip_prefix(Self::PREFIX)
            .ok_or_else(|| format!("expected `media:<hex>`, got `{s}`"))?;
        if hex.len() != 64 {
            return Err(format!("media id hex must be 64 chars, got {}", hex.len()));
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let text = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
            bytes[i] = u8::from_str_radix(text, 16)
                .map_err(|e| format!("invalid media id hex `{s}`: {e}"))?;
        }
        Ok(Self(bytes))
    }
}

#[cfg(feature = "serde")]
impl Serialize for MediaId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for MediaId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Fields that participate in [`MediaId`] hashing.
///
/// Keep this list in sync with [`compute_media_id`] — tests assert the split
/// between identity and freshness (`modified`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaIdentityFields<'a> {
    /// Basename only (no parent directory).
    pub basename: &'a str,
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
    /// Container label.
    pub container_format: &'a str,
    /// Codec label.
    pub codec: &'a str,
}

/// Compute `media:<blake3>` from identity fields. Does **not** include mtime.
pub fn compute_media_id(fields: &MediaIdentityFields<'_>) -> MediaId {
    let mut hasher = blake3::Hasher::new();
    // Length-prefixed UTF-8 fields so separators cannot collide.
    fn put(hasher: &mut blake3::Hasher, text: &str) {
        let bytes = text.as_bytes();
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    put(&mut hasher, fields.basename);
    hasher.update(&fields.sample_rate.to_le_bytes());
    hasher.update(&(fields.channel_count as u64).to_le_bytes());
    hasher.update(&fields.frame_count.to_le_bytes());
    match fields.bits_per_sample {
        Some(bits) => {
            hasher.update(&[1u8]);
            hasher.update(&bits.to_le_bytes());
        }
        None => {
            hasher.update(&[0u8]);
        }
    }
    hasher.update(&fields.size_bytes.to_le_bytes());
    put(&mut hasher, fields.container_format);
    put(&mut hasher, fields.codec);
    MediaId(*hasher.finalize().as_bytes())
}

fn basename_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Serializable media metadata persisted in `.fasession` / `.facomp`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MediaDescriptor {
    /// Stable identity (`media:<hex>`).
    pub id: MediaId,
    /// Stored URL (relative path, `file://`, or `memory://`).
    #[cfg_attr(feature = "serde", serde(rename = "url", alias = "path"))]
    pub url: String,
    /// Basename used for identity (no parent dir).
    pub basename: String,
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
    /// Last modified time (freshness only — not part of [`MediaId`]).
    #[cfg_attr(feature = "serde", serde(with = "rfc3339"))]
    pub modified: SystemTime,
    /// Container label.
    pub container_format: String,
    /// Codec label.
    pub codec: String,
}

impl MediaDescriptor {
    /// Identity fields for hashing / comparison.
    pub fn identity_fields(&self) -> MediaIdentityFields<'_> {
        MediaIdentityFields {
            basename: &self.basename,
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            frame_count: self.frame_count,
            bits_per_sample: self.bits_per_sample,
            size_bytes: self.size_bytes,
            container_format: &self.container_format,
            codec: &self.codec,
        }
    }

    /// Recompute id from identity fields (does not mutate `self.id`).
    pub fn compute_id(&self) -> MediaId {
        compute_media_id(&self.identity_fields())
    }

    /// Assign `id` from identity fields.
    pub fn with_computed_id(mut self) -> Self {
        self.id = self.compute_id();
        self
    }
}

/// Why a probed file does not match a recorded descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorMismatch {
    /// Identity hash (basename + stats) differs.
    Identity,
    /// Same identity, but mtime (or other freshness field) differs.
    Freshness,
}

/// Compare recorded descriptor to a freshly probed one.
///
/// - Same identity fields, different mtime → [`DescriptorMismatch::Freshness`]
/// - Different identity fields → [`DescriptorMismatch::Identity`]
pub fn descriptor_mismatch(
    recorded: &MediaDescriptor,
    probed: &MediaDescriptor,
) -> Option<DescriptorMismatch> {
    if recorded.identity_fields() != probed.identity_fields() {
        return Some(DescriptorMismatch::Identity);
    }
    if recorded.modified != probed.modified {
        return Some(DescriptorMismatch::Freshness);
    }
    None
}

/// Reference to a media file or in-memory samples (runtime + persistence).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MediaRef {
    /// Stable identity.
    pub id: MediaId,
    /// Stored URL (relative path, `file://`, or `memory://`).
    #[cfg_attr(feature = "serde", serde(rename = "url", alias = "path"))]
    pub url: String,
    /// Basename used for identity.
    #[cfg_attr(feature = "serde", serde(default))]
    pub basename: String,
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
    /// Optional fully decoded samples (skipped on disk).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub samples: Option<Arc<Vec<Vec<f32>>>>,
}

impl MediaRef {
    /// Build an in-memory media ref from planar samples.
    pub fn from_memory(_ignored: MediaId, sample_rate: u32, samples: Vec<Vec<f32>>) -> Self {
        Self::from_memory_samples(sample_rate, samples)
    }

    /// Build an in-memory media ref (id computed from identity fields).
    pub fn from_memory_samples(sample_rate: u32, samples: Vec<Vec<f32>>) -> Self {
        let frame_count = samples.first().map(|ch| ch.len() as u64).unwrap_or(0);
        let channel_count = samples.len();
        let mut media = Self {
            id: MediaId([0u8; 32]),
            url: String::new(),
            basename: "memory".into(),
            path: PathBuf::new(),
            sample_rate,
            channel_count,
            frame_count,
            bits_per_sample: Some(32),
            size_bytes: 0,
            modified: SystemTime::UNIX_EPOCH,
            container_format: "memory".into(),
            codec: "pcm".into(),
            samples: Some(Arc::new(samples)),
        };
        media.id = media.compute_id();
        media.path = PathBuf::from(format!("memory://{}", media.id));
        media.url = media.path.to_string_lossy().into_owned();
        media
    }

    /// Identity fields for hashing.
    pub fn identity_fields(&self) -> MediaIdentityFields<'_> {
        MediaIdentityFields {
            basename: &self.basename,
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            frame_count: self.frame_count,
            bits_per_sample: self.bits_per_sample,
            size_bytes: self.size_bytes,
            container_format: &self.container_format,
            codec: &self.codec,
        }
    }

    /// Compute media id from identity fields.
    pub fn compute_id(&self) -> MediaId {
        compute_media_id(&self.identity_fields())
    }

    /// Ensure basename is set from path and id is computed.
    pub fn finalize_identity(&mut self) {
        if self.basename.is_empty() {
            self.basename = basename_of(&self.path);
        }
        self.id = self.compute_id();
    }

    /// Convert to a persistable descriptor.
    pub fn to_descriptor(&self) -> MediaDescriptor {
        MediaDescriptor {
            id: self.id,
            url: self.url.clone(),
            basename: self.basename.clone(),
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            frame_count: self.frame_count,
            bits_per_sample: self.bits_per_sample,
            size_bytes: self.size_bytes,
            modified: self.modified,
            container_format: self.container_format.clone(),
            codec: self.codec.clone(),
        }
    }

    /// Build from a descriptor and resolved path.
    pub fn from_descriptor(descriptor: MediaDescriptor, path: PathBuf) -> Self {
        Self {
            id: descriptor.id,
            url: descriptor.url,
            basename: descriptor.basename,
            path,
            sample_rate: descriptor.sample_rate,
            channel_count: descriptor.channel_count,
            frame_count: descriptor.frame_count,
            bits_per_sample: descriptor.bits_per_sample,
            size_bytes: descriptor.size_bytes,
            modified: descriptor.modified,
            container_format: descriptor.container_format,
            codec: descriptor.codec,
            samples: None,
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
        if self.basename.is_empty() {
            self.basename = basename_of(&self.path);
        }
        Ok(())
    }
}

/// Pool of media refs keyed by [`MediaId`].
#[derive(Debug, Clone, Default)]
pub struct MediaPool {
    media: HashMap<MediaId, MediaRef>,
}

impl MediaPool {
    /// Empty pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `media` by id: reuse existing if present, otherwise insert.
    ///
    /// Zero ids are computed from identity fields before the lookup so a
    /// shared session store cannot collapse unrelated media onto `[0; 32]`.
    pub fn intern(&mut self, mut media: MediaRef) -> (MediaId, bool) {
        if media.basename.is_empty() && !media.path.as_os_str().is_empty() {
            media.basename = basename_of(&media.path);
        }
        if media.id.0 == [0u8; 32] {
            media.id = media.compute_id();
        }
        let id = media.id;
        if self.media.contains_key(&id) {
            return (id, false);
        }
        self.media.insert(id, media);
        (id, true)
    }

    /// Insert or replace by id.
    pub fn insert(&mut self, mut media: MediaRef) -> MediaId {
        if media.basename.is_empty() && !media.path.as_os_str().is_empty() {
            media.basename = basename_of(&media.path);
        }
        // Zero id (all zeros) means "compute now".
        if media.id.0 == [0u8; 32] {
            media.id = media.compute_id();
        }
        let id = media.id;
        self.media.insert(id, media);
        id
    }

    /// Look up media by id.
    pub fn get(&self, id: MediaId) -> Option<&MediaRef> {
        self.media.get(&id)
    }

    /// Mutable lookup.
    pub fn get_mut(&mut self, id: MediaId) -> Option<&mut MediaRef> {
        self.media.get_mut(&id)
    }

    /// Iterate all media refs.
    pub fn iter(&self) -> impl Iterator<Item = &MediaRef> {
        self.media.values()
    }

    /// First media by hex-sorted id, if any.
    pub fn first(&self) -> Option<&MediaRef> {
        self.media.values().min_by_key(|m| m.id.to_hex())
    }

    /// Whether the pool has no entries.
    pub fn is_empty(&self) -> bool {
        self.media.is_empty()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.media.len()
    }

    /// Consume the pool into id-sorted refs.
    pub fn into_refs(self) -> Vec<MediaRef> {
        let mut refs: Vec<_> = self.media.into_values().collect();
        refs.sort_by_key(|m| m.id.to_hex());
        refs
    }

    /// Descriptors for persistence.
    pub fn descriptors(&self) -> Vec<MediaDescriptor> {
        let mut out: Vec<_> = self.media.values().map(|m| m.to_descriptor()).collect();
        out.sort_by_key(|d| d.id.to_hex());
        out
    }
}

/// Shared media pool + block pager for one session or standalone composition.
pub struct MediaStore {
    pool: MediaPool,
    pager: Arc<Mutex<crate::pager::BlockPager>>,
}

impl MediaStore {
    /// Create a store with an in-memory pager (tests / no decode).
    pub fn in_memory() -> Self {
        Self {
            pool: MediaPool::new(),
            pager: Arc::new(Mutex::new(crate::pager::BlockPager::in_memory())),
        }
    }

    /// Create a store wrapping an existing pager (shared via [`Arc`]).
    pub fn with_pager(pager: crate::pager::BlockPager) -> Self {
        Self {
            pool: MediaPool::new(),
            pager: Arc::new(Mutex::new(pager)),
        }
    }

    /// Replace the block pager with one that spills under `dir`.
    pub fn with_spill_dir(
        &mut self,
        dir: impl AsRef<Path>,
        source: Arc<dyn crate::pager::BlockSource>,
    ) -> anyhow::Result<()> {
        *self.pager.lock().unwrap() =
            crate::pager::BlockPager::new(dir.as_ref().to_path_buf(), source)?;
        Ok(())
    }

    /// Media pool.
    pub fn pool(&self) -> &MediaPool {
        &self.pool
    }

    /// Mutable media pool.
    pub fn pool_mut(&mut self) -> &mut MediaPool {
        &mut self.pool
    }

    /// Lock the block pager.
    pub fn pager(&self) -> MutexGuard<'_, crate::pager::BlockPager> {
        self.pager.lock().unwrap()
    }

    /// Shared block pager handle (same [`Arc`] when stores are cloned).
    pub fn pager_arc(&self) -> Arc<Mutex<crate::pager::BlockPager>> {
        Arc::clone(&self.pager)
    }

    /// Run `f` with the pool and a locked pager (avoids nested composition locks).
    pub fn with_locked_pager<R>(
        &self,
        f: impl FnOnce(&MediaPool, &mut crate::pager::BlockPager) -> R,
    ) -> R {
        let mut pager = self.pager.lock().unwrap();
        f(&self.pool, &mut pager)
    }

    /// Intern an entry into the pool.
    pub fn intern(&mut self, media: MediaRef) -> (MediaId, bool) {
        self.pool.intern(media)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn fields(basename: &str, size: u64) -> MediaIdentityFields<'_> {
        MediaIdentityFields {
            basename,
            sample_rate: 48_000,
            channel_count: 2,
            frame_count: 1000,
            bits_per_sample: Some(24),
            size_bytes: size,
            container_format: "wav",
            codec: "pcm_s24le",
        }
    }

    #[test]
    fn media_id_ignores_mtime_but_mismatch_detects_it() {
        let id_a = compute_media_id(&fields("take.wav", 100));
        let id_b = compute_media_id(&fields("take.wav", 100));
        assert_eq!(id_a, id_b);

        let id_c = compute_media_id(&fields("take.wav", 101));
        assert_ne!(id_a, id_c);

        let recorded = MediaDescriptor {
            id: id_a,
            url: "take.wav".into(),
            basename: "take.wav".into(),
            sample_rate: 48_000,
            channel_count: 2,
            frame_count: 1000,
            bits_per_sample: Some(24),
            size_bytes: 100,
            modified: UNIX_EPOCH,
            container_format: "wav".into(),
            codec: "pcm_s24le".into(),
        };
        let mut probed = recorded.clone();
        probed.modified = UNIX_EPOCH + Duration::from_secs(10);
        assert_eq!(
            descriptor_mismatch(&recorded, &probed),
            Some(DescriptorMismatch::Freshness)
        );
        assert_eq!(recorded.compute_id(), probed.compute_id());

        probed.size_bytes = 101;
        assert_eq!(
            descriptor_mismatch(&recorded, &probed),
            Some(DescriptorMismatch::Identity)
        );
    }

    #[test]
    fn media_id_display_and_parse() {
        let id = compute_media_id(&fields("a.wav", 1));
        let text = id.to_string();
        assert!(text.starts_with("media:"));
        assert_eq!(text.len(), "media:".len() + 64);
        assert_eq!(text.parse::<MediaId>().unwrap(), id);
    }

    #[test]
    fn intern_dedups_by_id() {
        let mut pool = MediaPool::new();
        let a = MediaRef::from_memory_samples(44_100, vec![vec![0.0; 4]]);
        let id = a.id;
        assert!(pool.intern(a).1);
        let b = MediaRef::from_memory_samples(44_100, vec![vec![0.0; 4]]);
        assert_eq!(b.id, id);
        assert!(!pool.intern(b).1);
        assert_eq!(pool.len(), 1);
    }
}
