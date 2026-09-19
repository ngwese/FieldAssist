// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! FieldAssist adapters that build [`field_ui_components::FileStatus`] from
//! application model types.

use field_ui_components::FileStatus;

use crate::model::composition::Composition;

/// Build status-bar metadata from an open composition.
pub fn file_status_from_composition(composition: &Composition) -> Option<FileStatus> {
    if composition.frames() == 0 {
        return None;
    }
    let pool = composition.pool();
    let media = pool.first();
    Some(FileStatus {
        sample_rate: composition.sample_rate(),
        bits_per_sample: media.and_then(|m| m.bits_per_sample),
        channel_count: composition.channel_count(),
        duration_secs: composition.duration_secs(),
        size_bytes: media.map(|m| m.size_bytes),
    })
}
