// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use field_audio_model::{MarkerType, RegionCollection, StoredMarker};

use super::tree::ClipTree;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// EditId.
pub struct EditId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Edit decision operation recorded in the EDL.
pub enum EditOp {
    /// Initial empty history entry.
    Init,
    /// Cut range to clipboard and remove.
    Cut {
        /// Range start frame.
        start: u64,
        /// Range length in frames.
        len: u64,
    },
    /// Copy range to clipboard.
    Copy {
        /// Range start frame.
        start: u64,
        /// Range length in frames.
        len: u64,
    },
    /// Paste clipboard at position.
    Paste {
        /// Insert position.
        at: u64,
        /// Pasted length in frames.
        len: u64,
    },
    /// Remove range without clipboard.
    Remove {
        /// Range start frame.
        start: u64,
        /// Range length in frames.
        len: u64,
    },
    /// Delete range (may leave silence).
    Delete {
        /// Range start frame.
        start: u64,
        /// Range length in frames.
        len: u64,
    },
    /// Keep only the given range.
    Trim {
        /// Range start frame.
        start: u64,
        /// Range length in frames.
        len: u64,
    },
    /// Move a range to a new destination.
    Move {
        /// Source start frame.
        from: u64,
        /// Range length in frames.
        len: u64,
        /// Destination start frame.
        dest: u64,
    },
    /// Duplicate a range in place.
    Duplicate {
        /// Range start frame.
        start: u64,
        /// Range length in frames.
        len: u64,
    },
    /// Roll a clip boundary by delta frames.
    Roll {
        /// Boundary frame.
        at: u64,
        /// Signed roll distance.
        delta: i64,
    },
}

#[derive(Debug, Clone)]
/// Edit.
pub struct Edit {
    /// id.
    pub id: EditId,
    /// op.
    pub op: EditOp,
    /// snapshot.
    pub snapshot: ClipTree,
}

#[derive(Debug, Clone)]
/// Edl.
pub struct Edl {
    edits: Vec<Edit>,
    cursor: usize,
    next_id: u64,
}

impl Edl {
    /// `new`.
    pub fn new(initial: ClipTree) -> Self {
        Self {
            edits: vec![Edit {
                id: EditId(0),
                op: EditOp::Init,
                snapshot: initial,
            }],
            cursor: 0,
            next_id: 1,
        }
    }

    /// `cursor`.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// `current`.
    pub fn current(&self) -> &Edit {
        &self.edits[self.cursor]
    }

    /// `current_id`.
    pub fn current_id(&self) -> EditId {
        self.current().id
    }

    /// `snapshot`.
    pub fn snapshot(&self) -> ClipTree {
        self.current().snapshot.clone()
    }

    /// `len`.
    pub fn len(&self) -> usize {
        self.edits.len()
    }

    /// `edits`.
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// `ops_from_first_user`.
    pub fn ops_from_first_user(&self) -> Vec<EditOp> {
        self.edits
            .iter()
            .skip(1)
            .map(|edit| edit.op.clone())
            .collect()
    }

    /// `push`.
    pub fn push(&mut self, op: EditOp, snapshot: ClipTree) -> EditId {
        self.edits.truncate(self.cursor + 1);
        let id = EditId(self.next_id);
        self.next_id += 1;
        self.edits.push(Edit { id, op, snapshot });
        self.cursor = self.edits.len() - 1;
        id
    }

    /// `can_undo`.
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// `can_redo`.
    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.edits.len()
    }

    /// `undo`.
    pub fn undo(&mut self) -> Option<ClipTree> {
        if !self.can_undo() {
            return None;
        }
        self.cursor -= 1;
        Some(self.snapshot())
    }

    /// `redo`.
    pub fn redo(&mut self) -> Option<ClipTree> {
        if !self.can_redo() {
            return None;
        }
        self.cursor += 1;
        Some(self.snapshot())
    }

    /// `jump_to`.
    pub fn jump_to(&mut self, id: EditId) -> Option<ClipTree> {
        let index = self.edits.iter().position(|edit| edit.id == id)?;
        self.cursor = index;
        Some(self.snapshot())
    }

    /// `jump_to_index`.
    pub fn jump_to_index(&mut self, index: usize) -> Option<ClipTree> {
        if index >= self.edits.len() {
            return None;
        }
        self.cursor = index;
        Some(self.snapshot())
    }

    /// `map_snapshots`.
    pub fn map_snapshots(&mut self, mut map: impl FnMut(&ClipTree) -> ClipTree) {
        for edit in &mut self.edits {
            edit.snapshot = map(&edit.snapshot);
        }
    }

    /// `replace_init_snapshot`.
    pub fn replace_init_snapshot(&mut self, snapshot: ClipTree) {
        if let Some(init) = self.edits.first_mut() {
            init.snapshot = snapshot;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// InitialState.
/// Starting state for a project before user edits.
pub enum InitialState {
    /// Empty composition with no media.
    Empty,
    /// Single full-length clip from a media pool entry.
    FromMedia {
        /// Media pool id.
        media_id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// ProjectFile.
pub struct ProjectFile {
    /// sample_rate.
    pub sample_rate: u32,
    /// channel_count.
    pub channel_count: usize,
    /// media.
    pub media: Vec<super::MediaRef>,
    /// initial.
    pub initial: InitialState,
    /// edits.
    pub edits: Vec<EditOp>,
    /// edit_cursor.
    pub edit_cursor: usize,
    #[serde(default)]
    /// markers.
    pub markers: Vec<StoredMarker>,
    #[serde(default)]
    /// marker_types.
    pub marker_types: Vec<MarkerType>,
    #[serde(default)]
    /// collections.
    pub collections: Vec<RegionCollection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// channel_layout.
    pub channel_layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// monitor_chain.
    pub monitor_chain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// playback_channels.
    pub playback_channels: Option<Vec<usize>>,
}

/// FACOMP_KIND:.
pub const FACOMP_KIND: &str = "facomp";
/// FACOMP_FORMAT_VERSION:.
pub const FACOMP_FORMAT_VERSION: u32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// ProjectEnvelope.
pub struct ProjectEnvelope {
    /// kind.
    pub kind: String,
    /// format_version.
    pub format_version: u32,
    #[serde(flatten)]
    /// project.
    pub project: ProjectFile,
}

impl ProjectEnvelope {
    /// `wrap`.
    pub fn wrap(project: ProjectFile) -> Self {
        Self {
            kind: FACOMP_KIND.into(),
            format_version: FACOMP_FORMAT_VERSION,
            project,
        }
    }

    /// `from_json`.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        use anyhow::{bail, Context};

        let envelope: Self = serde_json::from_str(json).context("parse project JSON")?;
        if envelope.kind != FACOMP_KIND {
            bail!("not a FieldAssist composition (kind {:?})", envelope.kind);
        }
        match envelope.format_version {
            1 | 2 | 3 | 4 | 5 => Ok(envelope),
            0 => bail!("missing or invalid format_version"),
            n if n > FACOMP_FORMAT_VERSION => {
                bail!("this file requires a newer FieldAssist (format_version {n})")
            }
            n => bail!("unsupported format_version {n}"),
        }
    }

    /// `to_json`.
    pub fn to_json(&self) -> anyhow::Result<String> {
        use anyhow::Context;
        serde_json::to_string_pretty(self).context("serialize project")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Clip, ClipId};
    use field_audio_model::MediaId;

    #[test]
    fn undo_redo_and_jump() {
        let mut edl = Edl::new(ClipTree::empty());
        let a = ClipTree::from_clip(Clip::silence(ClipId(1), 10));
        let b = ClipTree::from_clip(Clip::from_media(ClipId(2), MediaId(1), 0, 4));
        let id_a = edl.push(EditOp::Delete { start: 0, len: 1 }, a);
        let id_b = edl.push(EditOp::Paste { at: 0, len: 4 }, b.clone());
        assert!(edl.can_undo());
        edl.undo();
        assert_eq!(edl.current_id(), id_a);
        edl.redo();
        assert_eq!(edl.current_id(), id_b);
        edl.jump_to(id_a);
        assert_eq!(edl.snapshot().frames(), 10);
    }
}
