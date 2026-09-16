// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-audio-model
//!
//! Core audio editing model types: planar PCM buffers, regions, selections,
//! markers, media pool entries, and a decode-agnostic block pager.
//!
//! ```
//! use field_audio_model::{BlockPager, ChannelScope, PcmBuffer, Region, RegionId};
//!
//! let audio = PcmBuffer::empty(48_000);
//! let region = Region::new(RegionId(1), 0, 0, ChannelScope::all());
//! let _pager = BlockPager::in_memory();
//! assert_eq!(audio.sample_rate, 48_000);
//! assert_eq!(region.span_len(), 1);
//! ```

mod buffer;
mod markers;
mod media;
mod pager;
mod pcm;
mod regions;
mod selection;
mod snap;

pub use buffer::{Buffer, BufferSource, ChannelScope, NewRegion, Region, RegionId};
pub use markers::{
    default_marker_type, marker_type_color, Marker, MarkerId, MarkerList, MarkerType, NewMarker,
    StoredMarker, DEFAULT_MARKER_TYPES, MARKER_TYPE_BLUE, MARKER_TYPE_PURPLE,
    MARKER_TYPE_TRANSIENT, MARKER_TYPE_YELLOW,
};
pub use media::{MediaId, MediaPool, MediaRef};
pub use pager::{
    BlockPager, BlockSource, NullBlockSource, PagerStats, BLOCK_FRAMES, RAM_CACHE_BYTES,
};
pub use pcm::{DecodedAudio, PcmBuffer};
pub use regions::{RegionCollection, RegionEndpoint, SELECTION_COLLECTION};
pub use selection::SamplePosition;
pub use snap::{nearest_zero_crossing, nearest_zero_crossing_default};
