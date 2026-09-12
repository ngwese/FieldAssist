// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Compiled monitor DSP chain identifiers.

/// Compiled monitor DSP chain. Each variant has a fixed Faust input count
/// and always produces stereo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonitorChain {
    /// Single-channel monitor.
    Mono,
    /// Stereo with width / crossfeed.
    Stereo,
    /// Mid/Side decode.
    Ms,
    /// First-order Ambisonics (AmbiX / ACN-SN3D).
    Foa,
    /// First-order Ambisonics (FuMa).
    FoaFuma,
}

impl MonitorChain {
    /// All supported chains in UI order.
    pub const ALL: [Self; 5] = [Self::Mono, Self::Stereo, Self::Ms, Self::Foa, Self::FoaFuma];

    /// Stable string id (composition / document serialization).
    pub fn id(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::Ms => "ms",
            Self::Foa => "foa",
            Self::FoaFuma => "foa_fuma",
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
            Self::Ms => "M/S",
            Self::Foa => "B-Format (AmbiX)",
            Self::FoaFuma => "B-Format (FuMa)",
        }
    }

    /// Parse a stable id.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mono" => Some(Self::Mono),
            "stereo" => Some(Self::Stereo),
            "ms" => Some(Self::Ms),
            "foa" => Some(Self::Foa),
            "foa_fuma" => Some(Self::FoaFuma),
            _ => None,
        }
    }

    /// Faust input channel count.
    pub fn num_inputs(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo | Self::Ms => 2,
            Self::Foa | Self::FoaFuma => 4,
        }
    }

    /// Output channel count (always stereo).
    pub fn num_outputs(self) -> usize {
        2
    }
}

impl std::fmt::Display for MonitorChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips() {
        for chain in MonitorChain::ALL {
            assert_eq!(MonitorChain::parse(chain.id()), Some(chain));
        }
        assert_eq!(MonitorChain::parse("nope"), None);
    }
}
