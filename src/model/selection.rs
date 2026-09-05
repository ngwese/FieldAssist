// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use super::buffer::ChannelScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplePosition {
    pub sample: usize,
    pub channels: ChannelScope,
}
