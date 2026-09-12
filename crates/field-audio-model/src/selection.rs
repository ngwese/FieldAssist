// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use crate::buffer::ChannelScope;

/// Sample index plus the channel scope of a caret or click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplePosition {
    /// Sample index along the timeline.
    pub sample: usize,
    /// Channels the position applies to.
    pub channels: ChannelScope,
}
