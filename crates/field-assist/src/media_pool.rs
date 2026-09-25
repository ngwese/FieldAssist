// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Shared media-pool operations for Lua and the Media dock pane.
//!
//! Pool-only entries are runtime-only: they are not written into `.fasession`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use field_audio_io::probe_file;
use field_composition::media_ref_from_probed;
use field_session::Session;

use crate::model::composition::MediaId;
use crate::model::{Composition, MediaStore};

/// One row of shared-pool media for Lua snapshots and the Media pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPoolRow {
    /// Content-addressed media id.
    pub id: MediaId,
    /// Stored descriptor URL (relative, `file://`, or `memory://`).
    pub url: String,
    /// Resolved filesystem path (display form of [`MediaRef::path`](crate::model::MediaRef)).
    pub path: PathBuf,
    /// Basename used for identity.
    pub basename: String,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channel_count: usize,
    /// Frame count.
    pub frame_count: u64,
    /// Bits per sample when known.
    pub bits_per_sample: Option<u32>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last-modified freshness stamp.
    pub modified: SystemTime,
    /// RFC3339 rendering of [`Self::modified`].
    pub modified_text: String,
    /// Container format label.
    pub container_format: String,
    /// Codec label.
    pub codec: String,
}

impl MediaPoolRow {
    /// Duration in seconds when `sample_rate > 0`.
    #[allow(dead_code)]
    pub fn duration_secs(&self) -> Option<f64> {
        if self.sample_rate == 0 {
            None
        } else {
            Some(self.frame_count as f64 / f64::from(self.sample_rate))
        }
    }

    /// Fingerprint fragment for observe-and-skip UI refresh.
    #[allow(dead_code)]
    pub fn fingerprint(&self) -> (MediaId, PathBuf, SystemTime) {
        (self.id, self.path.clone(), self.modified)
    }
}

/// Probe `path` and intern into `store` without opening a session document.
pub fn add_media(store: &Arc<Mutex<MediaStore>>, path: &Path) -> Result<MediaPoolRow> {
    let probed = probe_file(path).with_context(|| format!("probe {}", path.display()))?;
    let media = media_ref_from_probed(probed);
    let mut locked = store.lock().unwrap();
    let (id, _) = locked.intern(media);
    locked
        .pool()
        .get(id)
        .cloned()
        .map(row_from_ref)
        .context("interned media missing from pool")
}

/// Sorted snapshot of every entry in the shared store.
pub fn list_media(store: &Arc<Mutex<MediaStore>>) -> Vec<MediaPoolRow> {
    let locked = store.lock().unwrap();
    let mut rows: Vec<_> = locked.pool().iter().cloned().map(row_from_ref).collect();
    rows.sort_by(|a, b| a.id.to_hex().cmp(&b.id.to_hex()));
    rows
}

/// Media ids held by open session media documents or composition timelines.
pub fn referenced_media_ids<'a>(
    session: &Session,
    compositions: impl IntoIterator<Item = &'a Composition>,
) -> HashSet<MediaId> {
    let mut ids = HashSet::new();
    for doc in session.documents() {
        if let Some(media_id) = doc.media_id() {
            ids.insert(media_id);
        }
    }
    for composition in compositions {
        ids.extend(composition.used_media_ids());
    }
    ids
}

/// Remove `id` when it is present and not in `referenced`.
pub fn remove_media(
    store: &Arc<Mutex<MediaStore>>,
    id: MediaId,
    referenced: &HashSet<MediaId>,
) -> Result<MediaPoolRow> {
    if referenced.contains(&id) {
        bail!("media still referenced: {id}");
    }
    let mut locked = store.lock().unwrap();
    locked
        .remove(id)
        .map(row_from_ref)
        .with_context(|| format!("unknown media {id}"))
}

fn row_from_ref(media: crate::model::MediaRef) -> MediaPoolRow {
    MediaPoolRow {
        id: media.id,
        url: media.url.as_str().to_owned(),
        path: media.path,
        basename: media.basename,
        sample_rate: media.sample_rate,
        channel_count: media.channel_count,
        frame_count: media.frame_count,
        bits_per_sample: media.bits_per_sample,
        size_bytes: media.size_bytes,
        modified: media.modified,
        modified_text: format_modified(media.modified),
        container_format: media.container_format,
        codec: media.codec,
    }
}

fn format_modified(time: SystemTime) -> String {
    // Prefer the same RFC3339 form used in .fasession / .facomp descriptors.
    let descriptor = crate::model::MediaDescriptor {
        id: crate::model::composition::MediaId([0u8; 32]),
        url: field_core::Location::from(""),
        basename: String::new(),
        sample_rate: 0,
        channel_count: 0,
        frame_count: 0,
        bits_per_sample: None,
        size_bytes: 0,
        modified: time,
        container_format: String::new(),
        codec: String::new(),
    };
    serde_json::to_value(&descriptor)
        .ok()
        .and_then(|value| {
            value
                .get("modified")
                .and_then(|v| v.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| {
            time.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::composition::MediaId;
    use crate::model::{Composition, CompositionId, DocumentId, MediaDescriptor, MediaRef};
    use field_session::SessionDocument;
    use std::time::UNIX_EPOCH;

    fn memory_media(frames: usize) -> MediaRef {
        MediaRef::from_memory_samples(44_100, vec![vec![0.0; frames]])
    }

    fn descriptor_for(media: &MediaRef) -> MediaDescriptor {
        media.to_descriptor()
    }

    #[test]
    fn list_sorts_by_id_hex() {
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let a = memory_media(4);
        let b = memory_media(8);
        {
            let mut locked = store.lock().unwrap();
            locked.intern(a);
            locked.intern(b);
        }
        let rows = list_media(&store);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].id.to_hex() <= rows[1].id.to_hex());
    }

    #[test]
    fn remove_rejects_session_media_document() {
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let media = memory_media(16);
        let id = media.id;
        store.lock().unwrap().intern(media.clone());

        let mut session = Session::new();
        session.insert(SessionDocument::new_media(
            DocumentId::from_u128(1),
            PathBuf::from("take.wav"),
            CompositionId::from_u128(2),
            descriptor_for(&media),
        ));

        let referenced = referenced_media_ids(&session, std::iter::empty::<&Composition>());
        let err = remove_media(&store, id, &referenced).unwrap_err();
        assert!(err.to_string().contains("still referenced"), "{err:#}");
        assert_eq!(store.lock().unwrap().pool().len(), 1);
    }

    #[test]
    fn remove_rejects_composition_used_media() {
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let media = memory_media(32);
        let composition = Composition::from_media(media.clone()).unwrap();
        let id = *composition.used_media_ids().iter().next().unwrap();
        store.lock().unwrap().intern(media);

        let session = Session::new();
        let referenced = referenced_media_ids(&session, [&composition]);
        let err = remove_media(&store, id, &referenced).unwrap_err();
        assert!(err.to_string().contains("still referenced"), "{err:#}");
    }

    #[test]
    fn remove_allows_unused_interned_media() {
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let used = memory_media(16);
        let unused = memory_media(64);
        let unused_id = unused.id;
        {
            let mut locked = store.lock().unwrap();
            locked.intern(used.clone());
            locked.intern(unused);
        }
        let composition = Composition::from_media(used).unwrap();
        let session = Session::new();
        let referenced = referenced_media_ids(&session, [&composition]);
        let row = remove_media(&store, unused_id, &referenced).unwrap();
        assert_eq!(row.id, unused_id);
        assert_eq!(store.lock().unwrap().pool().len(), 1);
    }

    #[test]
    fn remove_unknown_id_errors() {
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let referenced = HashSet::new();
        let id = MediaId::from_bytes([1u8; 32]);
        let err = remove_media(&store, id, &referenced).unwrap_err();
        assert!(err.to_string().contains("unknown media"), "{err:#}");
        let _ = UNIX_EPOCH;
    }
}
