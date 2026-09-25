// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! In-memory feature flag registry.

use crate::flags::{Feature, FeatureFlags};

/// Live feature-flag state consulted by hosts when gating UI and commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureRegistry {
    flags: FeatureFlags,
}

impl FeatureRegistry {
    /// Create a registry from a persisted flag snapshot.
    pub fn from_flags(flags: FeatureFlags) -> Self {
        Self { flags }
    }

    /// Whether `feature` is currently enabled.
    pub fn is_enabled(&self, feature: Feature) -> bool {
        self.flags.is_enabled(feature)
    }

    /// Enable or disable `feature`.
    pub fn set(&mut self, feature: Feature, enabled: bool) {
        self.flags.set(feature, enabled);
    }

    /// Borrow the current flag snapshot.
    pub fn flags(&self) -> &FeatureFlags {
        &self.flags
    }

    /// Replace the entire flag snapshot.
    pub fn set_flags(&mut self, flags: FeatureFlags) {
        self.flags = flags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_mirrors_flags() {
        let mut registry = FeatureRegistry::default();
        assert!(!registry.is_enabled(Feature::ContentCredentials));
        registry.set(Feature::ContentCredentials, true);
        assert!(registry.is_enabled(Feature::ContentCredentials));
        assert!(registry.flags().content_credentials);

        let mut flags = FeatureFlags::default();
        flags.analysis_ops = true;
        registry.set_flags(flags.clone());
        assert_eq!(registry.flags(), &flags);
        assert!(registry.is_enabled(Feature::AnalysisOps));
    }
}
