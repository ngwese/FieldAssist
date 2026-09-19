// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-core
//!
//! Shared primitives used across FieldAssist crates: filesystem URL encoding,
//! shareable progress handles, and prefixed identity types.
//!
//! ```
//! use field_core::{encode_file_url, CompositionId, ProgressHandle};
//! use std::path::Path;
//!
//! let url = encode_file_url(Path::new("takes/a.wav"), Some(Path::new(".")));
//! let progress = ProgressHandle::new();
//! let epoch = progress.begin("decode");
//! progress.set_fraction(epoch, 0.5);
//! let _id = CompositionId::new();
//! ```

mod file_url;
mod ids;
mod progress;

pub use file_url::{encode_file_url, resolve_file_url};
pub use ids::{deserialize_prefixed_uuid, serialize_prefixed_uuid, CompositionId};
pub use progress::{ProgressHandle, ProgressState};
