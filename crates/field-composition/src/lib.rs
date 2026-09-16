// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-composition
//!
//! Non-destructive composition model: clip trees, edit decision lists (EDL),
//! `.facomp` project I/O, and peak overview jobs. Decoding goes through
//! [`field_audio_model::BlockSource`] (typically
//! [`field_audio_io::SymphoniaBlockSource`]). Peak folding uses
//! [`field_audio_process::PEAK_BLOCK`].
//!
//! ```
//! use field_composition::{is_facomp_path, Composition};
//! use std::path::Path;
//!
//! assert!(is_facomp_path(Path::new("take.facomp")));
//! let composition = Composition::new(48_000, 2);
//! assert_eq!(composition.sample_rate(), 48_000);
//! assert!(composition.is_empty());
//! ```

mod analysis_store;
mod clip;
mod composition;
mod edit_ranges;
mod edl;
mod tree;

pub use analysis_store::{
    analysis_dirty_seeds, expand_frame_ranges, hop_align_frame_ranges, merge_frame_ranges,
    subtract_frame_range, AnalysisStreams, FloatStreamSeries, FrameRange, SpectralStreamSeries,
};
pub use clip::{Clip, ClipCache, ClipId, ClipMarker, ClipMarkerId, ClipSource, ClipSpan};
pub use composition::{
    is_facomp_path, normalize_facomp_file_name, AnalysisBlockOutcome, Clipboard, Composition,
    FramesIter, PeakBlockOutcome,
};
pub use edit_ranges::{
    analysis_inverse_op, map_inclusive_through_inverse, map_inclusive_through_op,
    map_point_if_kept, map_point_if_kept_inverse, map_point_through_inverse, map_point_through_op,
};
pub use edl::{
    CompositionId, Edit, EditId, EditOp, Edl, InitialState, ProjectEnvelope, ProjectFile,
    FACOMP_FORMAT_VERSION, FACOMP_KIND,
};
pub use field_audio_model::{
    default_marker_type, marker_type_color, BlockPager, BlockSource, Marker, MarkerId, MarkerList,
    MarkerType, MediaId, MediaPool, MediaRef, NullBlockSource, PagerStats, StoredMarker,
    BLOCK_FRAMES, DEFAULT_MARKER_TYPES, MARKER_TYPE_BLUE, MARKER_TYPE_PURPLE,
    MARKER_TYPE_TRANSIENT, MARKER_TYPE_YELLOW,
};
pub use field_audio_process::AnalysisKind;
pub use tree::ClipTree;
