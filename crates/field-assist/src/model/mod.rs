// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

pub mod document;

pub use document::BufferDocument;
#[allow(unused_imports)] // re-exported for call sites across the binary
pub use field_audio_model::{
    Buffer, BufferSource, ChannelScope, DecodedAudio, PcmBuffer, Region, RegionCollection,
    RegionEndpoint, RegionId, SELECTION_COLLECTION, SamplePosition, nearest_zero_crossing,
    nearest_zero_crossing_default,
};
pub use field_composition as composition;
#[allow(unused_imports)] // re-exported for call sites across the binary
pub use field_composition::{
    AnalysisKind, Clipboard, Composition, CompositionId, DEFAULT_MARKER_TYPES, DescriptorMismatch,
    EditId, EditOp, MARKER_TYPE_BLUE, MARKER_TYPE_PURPLE, MARKER_TYPE_TRANSIENT,
    MARKER_TYPE_YELLOW, Marker, MarkerId, MarkerType, MediaDescriptor, MediaRef, MediaStore,
    default_marker_type, descriptor_mismatch, is_facomp_path, marker_type_color,
};
pub use field_session as session;
#[allow(unused_imports)] // re-exported for call sites across the binary
pub use field_session::{
    DocumentId, Session, SessionDocksUi, SessionUi, SessionWindowUi, is_fasession_path,
};

/// Compatibility shim for `crate::model::file_url`.
pub mod file_url {
    #[allow(unused_imports)]
    pub use field_core::Location;
}

/// Compatibility shim for `crate::model::buffer`.
pub mod buffer {
    #[allow(unused_imports)]
    pub use field_audio_model::{Buffer, BufferSource, ChannelScope, Region, RegionId};
}

/// Compatibility shim for `crate::model::pcm`.
pub mod pcm {
    #[allow(unused_imports)]
    pub use field_audio_model::{DecodedAudio, PcmBuffer};
}

/// Compatibility shim for `crate::model::regions`.
pub mod regions {
    pub use field_audio_model::{RegionCollection, RegionEndpoint, SELECTION_COLLECTION};
}

/// Compatibility shim for `crate::model::selection`.
pub mod selection {
    pub use field_audio_model::SamplePosition;
}

/// Compatibility shim for `crate::model::snap`.
pub mod snap {
    #[allow(unused_imports)]
    pub use field_audio_model::{nearest_zero_crossing, nearest_zero_crossing_default};
}
