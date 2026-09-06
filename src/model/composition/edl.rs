// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::markers::{MarkerType, StoredMarker};
use super::tree::ClipTree;
use crate::model::regions::RegionCollection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EditId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditOp {
    Init,
    Cut { start: u64, len: u64 },
    Copy { start: u64, len: u64 },
    Paste { at: u64, len: u64 },
    Remove { start: u64, len: u64 },
    Delete { start: u64, len: u64 },
    Trim { start: u64, len: u64 },
    Move { from: u64, len: u64, dest: u64 },
    Duplicate { start: u64, len: u64 },
    Roll { at: u64, delta: i64 },
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub id: EditId,
    pub op: EditOp,
    pub snapshot: ClipTree,
}

#[derive(Debug, Clone)]
pub struct Edl {
    edits: Vec<Edit>,
    cursor: usize,
    next_id: u64,
}

impl Edl {
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

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn current(&self) -> &Edit {
        &self.edits[self.cursor]
    }

    pub fn current_id(&self) -> EditId {
        self.current().id
    }

    pub fn snapshot(&self) -> ClipTree {
        self.current().snapshot.clone()
    }

    pub fn len(&self) -> usize {
        self.edits.len()
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn ops_from_first_user(&self) -> Vec<EditOp> {
        self.edits
            .iter()
            .skip(1)
            .map(|edit| edit.op.clone())
            .collect()
    }

    pub fn push(&mut self, op: EditOp, snapshot: ClipTree) -> EditId {
        self.edits.truncate(self.cursor + 1);
        let id = EditId(self.next_id);
        self.next_id += 1;
        self.edits.push(Edit { id, op, snapshot });
        self.cursor = self.edits.len() - 1;
        id
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.edits.len()
    }

    pub fn undo(&mut self) -> Option<ClipTree> {
        if !self.can_undo() {
            return None;
        }
        self.cursor -= 1;
        Some(self.snapshot())
    }

    pub fn redo(&mut self) -> Option<ClipTree> {
        if !self.can_redo() {
            return None;
        }
        self.cursor += 1;
        Some(self.snapshot())
    }

    pub fn jump_to(&mut self, id: EditId) -> Option<ClipTree> {
        let index = self.edits.iter().position(|edit| edit.id == id)?;
        self.cursor = index;
        Some(self.snapshot())
    }

    pub fn jump_to_index(&mut self, index: usize) -> Option<ClipTree> {
        if index >= self.edits.len() {
            return None;
        }
        self.cursor = index;
        Some(self.snapshot())
    }

    pub fn map_snapshots(&mut self, mut map: impl FnMut(&ClipTree) -> ClipTree) {
        for edit in &mut self.edits {
            edit.snapshot = map(&edit.snapshot);
        }
    }

    pub fn replace_init_snapshot(&mut self, snapshot: ClipTree) {
        if let Some(init) = self.edits.first_mut() {
            init.snapshot = snapshot;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InitialState {
    Empty,
    FromMedia { media_id: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub sample_rate: u32,
    pub channel_count: usize,
    pub media: Vec<super::media::MediaRef>,
    pub initial: InitialState,
    pub edits: Vec<EditOp>,
    pub edit_cursor: usize,
    #[serde(default)]
    pub markers: Vec<StoredMarker>,
    #[serde(default)]
    pub marker_types: Vec<MarkerType>,
    #[serde(default)]
    pub collections: Vec<RegionCollection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_layout: Option<String>,
}

pub const FACOMP_KIND: &str = "facomp";
pub const FACOMP_FORMAT_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEnvelope {
    pub kind: String,
    pub format_version: u32,
    #[serde(flatten)]
    pub project: ProjectFile,
}

impl ProjectEnvelope {
    pub fn wrap(project: ProjectFile) -> Self {
        Self {
            kind: FACOMP_KIND.into(),
            format_version: FACOMP_FORMAT_VERSION,
            project,
        }
    }

    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        use anyhow::{bail, Context};

        let envelope: Self = serde_json::from_str(json).context("parse project JSON")?;
        if envelope.kind != FACOMP_KIND {
            bail!("not a snd-review composition (kind {:?})", envelope.kind);
        }
        match envelope.format_version {
            1 | 2 | 3 => Ok(envelope),
            0 => bail!("missing or invalid format_version"),
            n if n > FACOMP_FORMAT_VERSION => {
                bail!("this file requires a newer snd-review (format_version {n})")
            }
            n => bail!("unsupported format_version {n}"),
        }
    }

    pub fn to_json(&self) -> anyhow::Result<String> {
        use anyhow::Context;
        serde_json::to_string_pretty(self).context("serialize project")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::composition::clip::{Clip, ClipId};
    use crate::model::composition::media::MediaId;

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
