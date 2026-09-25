// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-features
//!
//! Product feature-flag definitions and a small in-memory registry used by
//! FieldAssist hosts to gate experimental functionality.
//!
//! ```
//! use field_features::{Feature, FeatureRegistry};
//!
//! let mut registry = FeatureRegistry::default();
//! assert!(!registry.is_enabled(Feature::AnalysisOps));
//! registry.set(Feature::AnalysisOps, true);
//! assert!(registry.is_enabled(Feature::AnalysisOps));
//! ```

mod flags;
mod registry;

pub use flags::{Feature, FeatureFlags};
pub use registry::FeatureRegistry;
