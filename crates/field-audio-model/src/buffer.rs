// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::{path::PathBuf, time::SystemTime};

#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::pcm::PcmBuffer;

/// Stable identifier for a region within a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RegionId(pub u64);

/// Which channels a region applies to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelScope {
    /// Every channel in the buffer.
    AllChannels,
    /// Explicit channel indices.
    Channels(Vec<usize>),
}

impl ChannelScope {
    /// Scope covering all channels.
    pub fn all() -> Self {
        Self::AllChannels
    }

    /// Scope covering a single channel index.
    pub fn single(channel: usize) -> Self {
        Self::Channels(vec![channel])
    }

    /// Whether this scope includes `channel`.
    pub fn applies_to(&self, channel: usize) -> bool {
        match self {
            Self::AllChannels => true,
            Self::Channels(channels) => channels.contains(&channel),
        }
    }

    /// Sort, dedupe, and collapse an empty list to [`AllChannels`](Self::AllChannels).
    pub fn normalize(&mut self) {
        if let Self::Channels(channels) = self {
            channels.sort_unstable();
            channels.dedup();
            if channels.is_empty() {
                *self = Self::AllChannels;
            }
        }
    }
}

#[cfg(feature = "serde")]
impl Serialize for ChannelScope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::AllChannels => serializer.serialize_str("all"),
            Self::Channels(channels) => channels.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for ChannelScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            All(String),
            List(Vec<usize>),
        }
        match Repr::deserialize(deserializer)? {
            Repr::All(text) if text == "all" => Ok(Self::AllChannels),
            Repr::All(text) => Err(serde::de::Error::custom(format!(
                "channels must be \"all\" or an array, got {text:?}"
            ))),
            Repr::List(channels) => {
                let mut scope = if channels.is_empty() {
                    Self::AllChannels
                } else {
                    Self::Channels(channels)
                };
                scope.normalize();
                Ok(scope)
            }
        }
    }
}

/// On-disk provenance for a loaded buffer.
#[derive(Debug, Clone)]
pub struct BufferSource {
    /// Absolute or original filesystem path.
    pub path: PathBuf,
    /// Last modified time from the filesystem.
    pub modified: SystemTime,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Bits per sample when known from the container.
    pub bits_per_sample: Option<u32>,
    /// Container / extension label (e.g. `"wav"`).
    pub container_format: String,
    /// Codec label from the probe/decoder.
    pub codec: String,
}

/// Inclusive sample span with channel scope and optional label.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Region {
    /// Region id within its collection.
    pub id: RegionId,
    /// Inclusive start sample.
    pub start: usize,
    /// Inclusive end sample.
    pub end: usize,
    /// Channels this region covers.
    pub channels: ChannelScope,
    /// Optional display label.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub label: Option<String>,
}

impl Region {
    /// Create a normalized region (swaps endpoints if needed).
    pub fn new(id: RegionId, start: usize, end: usize, channels: ChannelScope) -> Self {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let mut channels = channels;
        channels.normalize();
        Self {
            id,
            start,
            end,
            channels,
            label: None,
        }
    }

    /// Attach a display label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Whether `(sample, channel)` lies inside this region.
    pub fn contains(&self, sample: usize, channel: usize) -> bool {
        self.channels.applies_to(channel) && sample >= self.start && sample <= self.end
    }

    /// Inclusive span length in samples.
    pub fn span_len(&self) -> usize {
        self.end.saturating_sub(self.start).saturating_add(1)
    }
}

/// Loaded audio plus optional file provenance.
#[derive(Debug)]
pub struct Buffer {
    /// Planar PCM and peaks.
    pub audio: PcmBuffer,
    /// Source file metadata when loaded from disk.
    pub source: Option<BufferSource>,
}

impl Buffer {
    /// Empty stereo-rate buffer with no samples.
    pub fn empty() -> Self {
        Self {
            audio: PcmBuffer::empty(44100),
            source: None,
        }
    }

    /// Whether any audio frames are present.
    pub fn is_loaded(&self) -> bool {
        self.frames() > 0
    }

    /// Frame count of the audio.
    pub fn frames(&self) -> usize {
        self.audio.frames()
    }

    /// Full-buffer all-channel region (id `0`).
    pub fn full_region(&self) -> Region {
        let end = self.frames().saturating_sub(1);
        Region {
            id: RegionId(0),
            start: 0,
            end,
            channels: ChannelScope::all(),
            label: None,
        }
    }
}
