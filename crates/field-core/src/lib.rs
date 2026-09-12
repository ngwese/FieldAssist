// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-core
//!
//! Shared primitives used across FieldAssist crates: filesystem URL encoding
//! and shareable progress handles for long-running jobs.
//!
//! ```
//! use field_core::{encode_file_url, ProgressHandle};
//! use std::path::Path;
//!
//! let url = encode_file_url(Path::new("takes/a.wav"), Some(Path::new(".")));
//! let progress = ProgressHandle::new();
//! let epoch = progress.begin("decode");
//! progress.set_fraction(epoch, 0.5);
//! ```

mod file_url;
mod progress;

pub use file_url::{encode_file_url, resolve_file_url};
pub use progress::{ProgressHandle, ProgressState};
