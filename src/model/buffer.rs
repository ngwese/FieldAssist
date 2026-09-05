// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::{path::PathBuf, time::SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::audio::DecodedAudio;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RegionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelScope {
    AllChannels,
    Channels(Vec<usize>),
}

impl ChannelScope {
    pub fn all() -> Self {
        Self::AllChannels
    }

    pub fn single(channel: usize) -> Self {
        Self::Channels(vec![channel])
    }

    pub fn applies_to(&self, channel: usize) -> bool {
        match self {
            Self::AllChannels => true,
            Self::Channels(channels) => channels.contains(&channel),
        }
    }

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

impl Serialize for ChannelScope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::AllChannels => serializer.serialize_str("all"),
            Self::Channels(channels) => channels.serialize(serializer),
        }
    }
}

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

#[derive(Debug, Clone)]
pub struct BufferSource {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size_bytes: u64,
    pub bits_per_sample: Option<u32>,
    pub container_format: String,
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub id: RegionId,
    pub start: usize,
    pub end: usize,
    pub channels: ChannelScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Region {
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

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn contains(&self, sample: usize, channel: usize) -> bool {
        self.channels.applies_to(channel) && sample >= self.start && sample <= self.end
    }

    pub fn span_len(&self) -> usize {
        self.end.saturating_sub(self.start).saturating_add(1)
    }
}

#[derive(Debug)]
pub struct Buffer {
    pub audio: DecodedAudio,
    pub source: Option<BufferSource>,
}

impl Buffer {
    pub fn empty() -> Self {
        Self {
            audio: DecodedAudio {
                sample_rate: 44100,
                channels: vec![],
                peaks: vec![],
            },
            source: None,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.frames() > 0
    }

    pub fn frames(&self) -> usize {
        self.audio.frames()
    }

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
