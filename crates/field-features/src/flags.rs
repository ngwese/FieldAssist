// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Feature identifiers and persisted flag snapshots.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Known product feature flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// C2PA / Content Credentials signing and related tooling.
    ContentCredentials,
    /// Offline analysis ops (Analyze menu and `analyze.*` commands).
    AnalysisOps,
}

impl Feature {
    /// Stable string id used in settings JSON and diagnostics.
    pub fn id(self) -> &'static str {
        match self {
            Self::ContentCredentials => "content_credentials",
            Self::AnalysisOps => "analysis_ops",
        }
    }

    /// Shipping default for this flag (always off today).
    pub fn default_enabled(self) -> bool {
        false
    }

    /// All registered features in a stable order.
    pub fn all() -> &'static [Feature] {
        &[Self::ContentCredentials, Self::AnalysisOps]
    }
}

/// Persisted snapshot of experimental feature flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FeatureFlags {
    /// Enable Content Credentials / C2PA tooling when implemented.
    pub content_credentials: bool,
    /// Enable the Analyze menu and `analyze.*` commands.
    pub analysis_ops: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            content_credentials: Feature::ContentCredentials.default_enabled(),
            analysis_ops: Feature::AnalysisOps.default_enabled(),
        }
    }
}

impl FeatureFlags {
    /// Whether `feature` is enabled in this snapshot.
    pub fn is_enabled(&self, feature: Feature) -> bool {
        match feature {
            Feature::ContentCredentials => self.content_credentials,
            Feature::AnalysisOps => self.analysis_ops,
        }
    }

    /// Set whether `feature` is enabled.
    pub fn set(&mut self, feature: Feature, enabled: bool) {
        match feature {
            Feature::ContentCredentials => self.content_credentials = enabled,
            Feature::AnalysisOps => self.analysis_ops = enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off() {
        let flags = FeatureFlags::default();
        for feature in Feature::all() {
            assert!(!flags.is_enabled(*feature));
            assert!(!feature.default_enabled());
        }
    }

    #[test]
    fn set_and_is_enabled() {
        let mut flags = FeatureFlags::default();
        flags.set(Feature::AnalysisOps, true);
        assert!(flags.is_enabled(Feature::AnalysisOps));
        assert!(!flags.is_enabled(Feature::ContentCredentials));
    }

    #[test]
    fn ids_are_stable() {
        assert_eq!(Feature::ContentCredentials.id(), "content_credentials");
        assert_eq!(Feature::AnalysisOps.id(), "analysis_ops");
    }

    #[test]
    fn round_trip_json() {
        let mut flags = FeatureFlags::default();
        flags.content_credentials = true;
        let text = serde_json::to_string(&flags).unwrap();
        let back: FeatureFlags = serde_json::from_str(&text).unwrap();
        assert_eq!(back, flags);
    }
}
