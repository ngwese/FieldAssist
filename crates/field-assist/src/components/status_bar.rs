// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! FieldAssist adapters that build [`field_ui_components::FileStatus`] from
//! application model types.

use field_ui_components::FileStatus;

use crate::model::composition::Composition;
use crate::model::Buffer;

/// Build status-bar metadata from an open composition.
pub fn file_status_from_composition(composition: &Composition) -> Option<FileStatus> {
    if composition.frames() == 0 {
        return None;
    }
    let media = composition.pool().first();
    Some(FileStatus {
        sample_rate: composition.sample_rate(),
        bits_per_sample: media.and_then(|m| m.bits_per_sample),
        channel_count: composition.channel_count(),
        duration_secs: composition.duration_secs(),
        size_bytes: media.map(|m| m.size_bytes),
    })
}

/// Build status-bar metadata from an in-memory buffer.
pub fn file_status_from_buffer(buffer: &Buffer) -> Option<FileStatus> {
    if !buffer.is_loaded() {
        return None;
    }
    Some(FileStatus {
        sample_rate: buffer.audio.sample_rate,
        bits_per_sample: buffer
            .source
            .as_ref()
            .and_then(|source| source.bits_per_sample),
        channel_count: buffer.audio.channel_count(),
        duration_secs: buffer.audio.duration_secs(),
        size_bytes: buffer.source.as_ref().map(|source| source.size_bytes),
    })
}
