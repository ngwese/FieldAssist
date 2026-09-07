// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

pub mod buffer;
pub mod composition;
pub mod document;
pub mod file_url;
pub mod regions;
pub mod selection;
pub mod session;
pub mod snap;

pub use buffer::{Buffer, BufferSource, ChannelScope, Region, RegionId};
pub use composition::{
    default_marker_type, is_facomp_path, marker_type_color, Clipboard, Composition, EditId, EditOp,
    Marker, MarkerId, MarkerType, MediaRef, DEFAULT_MARKER_TYPES, MARKER_TYPE_BLUE,
    MARKER_TYPE_PURPLE, MARKER_TYPE_YELLOW,
};
pub use document::BufferDocument;
pub use regions::{RegionCollection, RegionEndpoint, SELECTION_COLLECTION};
pub use selection::SamplePosition;
pub use session::{
    is_fasession_path, DocumentId, Session, SessionDocksUi, SessionDocument, SessionId, SessionUi,
    SessionWindowUi,
};
