// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Transport state machine (UI side).

/// Playback transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// Not playing; position may be mid-buffer.
    Stopped,
    /// Actively rendering audio.
    Playing,
    /// Paused; position retained.
    Paused,
}

impl TransportState {
    /// Decode from the atomic wire format.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Playing,
            2 => Self::Paused,
            _ => Self::Stopped,
        }
    }

    /// Encode for atomics shared with the audio callback.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Stopped => 0,
            Self::Playing => 1,
            Self::Paused => 2,
        }
    }
}

/// UI-side transport controller mirroring engine state.
pub struct Transport {
    state: TransportState,
}

impl Transport {
    /// Create a stopped transport.
    pub fn new() -> Self {
        Self {
            state: TransportState::Stopped,
        }
    }

    /// Current state.
    pub fn state(&self) -> TransportState {
        self.state
    }

    /// Set state.
    pub fn set_state(&mut self, state: TransportState) {
        self.state = state;
    }

    /// Whether currently playing.
    pub fn is_playing(&self) -> bool {
        self.state == TransportState::Playing
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}
