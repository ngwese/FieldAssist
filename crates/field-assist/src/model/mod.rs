// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

pub mod document;

pub use document::BufferDocument;
#[allow(unused_imports)] // re-exported for call sites across the binary
pub use field_audio_model::{
    nearest_zero_crossing, nearest_zero_crossing_default, Buffer, BufferSource, ChannelScope,
    DecodedAudio, PcmBuffer, Region, RegionCollection, RegionEndpoint, RegionId, SamplePosition,
    SELECTION_COLLECTION,
};
pub use field_composition as composition;
#[allow(unused_imports)] // re-exported for call sites across the binary
pub use field_composition::{
    default_marker_type, is_facomp_path, marker_type_color, Clipboard, Composition, CompositionId,
    EditId, EditOp, Marker, MarkerId, MarkerType, MediaRef, DEFAULT_MARKER_TYPES, MARKER_TYPE_BLUE,
    MARKER_TYPE_PURPLE, MARKER_TYPE_YELLOW,
};
pub use field_session as session;
pub use field_session::{
    is_fasession_path, DocumentId, Session, SessionDocksUi, SessionDocument, SessionId, SessionUi,
    SessionWindowUi,
};

/// Compatibility shim for `crate::model::file_url`.
pub mod file_url {
    #[allow(unused_imports)]
    pub use field_core::{encode_file_url, resolve_file_url};
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
