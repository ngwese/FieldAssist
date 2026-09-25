// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-core
//!
//! Shared primitives used across FieldAssist crates: portable file locations,
//! open-problem reports, shareable progress handles, and prefixed identity types.
//!
//! ```
//! use field_core::{Location, CompositionId, ProgressHandle};
//! use std::path::Path;
//!
//! let url = Location::from_path_relative_to(
//!     Path::new("/sessions/takes/a.wav"),
//!     Path::new("/sessions"),
//! );
//! assert_eq!(url.as_str(), "takes/a.wav");
//! let progress = ProgressHandle::new();
//! let epoch = progress.begin("decode");
//! progress.set_fraction(epoch, 0.5);
//! let _id = CompositionId::new();
//! ```

mod file_url;
mod ids;
mod open_report;
mod progress;

pub use file_url::Location;
pub use ids::{deserialize_prefixed_uuid, serialize_prefixed_uuid, CompositionId};
pub use open_report::{LoadProblem, OpenReport, ProblemCategory};
pub use progress::{ProgressHandle, ProgressState};
