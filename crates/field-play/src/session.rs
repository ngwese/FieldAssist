// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Open-path / save-target tracking for field-play.

use std::path::{Path, PathBuf};

use field_composition::is_facomp_path;

/// Remembers where the composition was opened and where it should be saved.
#[derive(Debug, Clone)]
pub struct PlaySession {
    opened: PathBuf,
    /// Set when opened as `.facomp`, or after the first successful save.
    save_path: Option<PathBuf>,
    /// Parent directory of the media file when opened ephemerally.
    media_dir: Option<PathBuf>,
}

impl PlaySession {
    /// Build session state from the CLI open path.
    pub fn new(opened: PathBuf) -> Self {
        if is_facomp_path(&opened) {
            Self {
                save_path: Some(opened.clone()),
                media_dir: opened.parent().map(|p| p.to_path_buf()),
                opened,
            }
        } else {
            let media_dir = opened.parent().map(|p| p.to_path_buf());
            Self {
                opened,
                save_path: None,
                media_dir,
            }
        }
    }

    /// Path originally opened.
    pub fn opened(&self) -> &Path {
        &self.opened
    }

    /// Established `.facomp` write path, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn save_path(&self) -> Option<&Path> {
        self.save_path.as_deref()
    }

    /// Resolve the write path and whether a y/n overwrite confirm is required.
    ///
    /// Overwrite confirm is only needed when there is no established `save_path`
    /// yet and the suggested ephemeral path already exists on disk.
    pub fn resolve_save_target(&self, suggested_name: &str) -> Option<SaveTarget> {
        if let Some(path) = &self.save_path {
            return Some(SaveTarget {
                path: path.clone(),
                needs_overwrite_confirm: false,
            });
        }
        let dir = self.media_dir.as_ref()?;
        let path = dir.join(suggested_name);
        let needs_overwrite_confirm = path.exists();
        Some(SaveTarget {
            path,
            needs_overwrite_confirm,
        })
    }

    /// Record a successful save so later saves overwrite without re-prompting.
    pub fn mark_saved(&mut self, path: PathBuf) {
        self.save_path = Some(path);
    }
}

/// Result of [`PlaySession::resolve_save_target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTarget {
    /// Path to write.
    pub path: PathBuf,
    /// True when the user must confirm overwriting an existing file.
    pub needs_overwrite_confirm: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn facomp_open_uses_opened_path_without_confirm() {
        let path = PathBuf::from("/tmp/take.facomp");
        let session = PlaySession::new(path.clone());
        assert_eq!(session.save_path(), Some(path.as_path()));
        let target = session
            .resolve_save_target("ignored.facomp")
            .expect("target");
        assert_eq!(target.path, path);
        assert!(!target.needs_overwrite_confirm);
    }

    #[test]
    fn media_open_suggests_beside_source() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("take.wav");
        fs::write(&wav, b"x").unwrap();
        let session = PlaySession::new(wav);
        let target = session.resolve_save_target("take.facomp").expect("target");
        assert_eq!(target.path, dir.path().join("take.facomp"));
        assert!(!target.needs_overwrite_confirm);
    }

    #[test]
    fn media_open_confirms_when_suggested_exists() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("take.wav");
        let facomp = dir.path().join("take.facomp");
        fs::write(&wav, b"x").unwrap();
        fs::write(&facomp, b"{}").unwrap();
        let session = PlaySession::new(wav);
        let target = session.resolve_save_target("take.facomp").expect("target");
        assert_eq!(target.path, facomp);
        assert!(target.needs_overwrite_confirm);
    }

    #[test]
    fn after_mark_saved_no_confirm_even_if_exists() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("take.wav");
        let facomp = dir.path().join("take.facomp");
        fs::write(&wav, b"x").unwrap();
        fs::write(&facomp, b"{}").unwrap();
        let mut session = PlaySession::new(wav);
        session.mark_saved(facomp.clone());
        let target = session.resolve_save_target("take.facomp").expect("target");
        assert_eq!(target.path, facomp);
        assert!(!target.needs_overwrite_confirm);
    }
}
