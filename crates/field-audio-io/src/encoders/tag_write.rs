// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Post-encode Vorbis-comment tagging via lofty.

use std::io::Write;

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::ogg::tag::VorbisComments;
use lofty::prelude::*;

use crate::metadata::TagMap;

/// Encode into memory, optionally stamp Vorbis/ID3-style tags, then flush.
pub fn encode_with_optional_tags(
    extension: &str,
    tags: &TagMap,
    encode_body: impl FnOnce(&mut Vec<u8>) -> Result<()>,
    writer: &mut dyn Write,
) -> Result<()> {
    let mut bytes = Vec::new();
    encode_body(&mut bytes)?;
    if tags.is_empty() {
        writer.write_all(&bytes).context("write encoded audio")?;
        return Ok(());
    }
    let dir = tempfile::tempdir().context("temp dir for tags")?;
    let path = dir.path().join(format!("export.{extension}"));
    std::fs::write(&path, &bytes).context("stage encoded file")?;
    apply_tags_to_path(&path, extension, tags)?;
    let tagged = std::fs::read(&path).context("read tagged file")?;
    writer.write_all(&tagged).context("write tagged audio")?;
    Ok(())
}

/// Stamp tags onto an already-written audio file at `path`.
pub fn apply_tags_to_path(path: &std::path::Path, _extension: &str, tags: &TagMap) -> Result<()> {
    if tags.is_empty() {
        return Ok(());
    }
    let mut comments = VorbisComments::default();
    insert_vorbis_comments(&mut comments, tags);
    comments
        .save_to_path(path, WriteOptions::default())
        .context("save vorbis tags")?;
    Ok(())
}

fn insert_vorbis_comments(comments: &mut VorbisComments, tags: &TagMap) {
    // Prefer uppercase Vorbis keys when both case variants exist.
    let mut written = std::collections::BTreeSet::new();
    let preferred = [
        ("ARTIST", "artist"),
        ("COPYRIGHT", "copyright"),
        ("DESCRIPTION", "description"),
        ("TITLE", "title"),
        ("ALBUM", "album"),
        ("COMMENT", "comment"),
        ("DATE", "date"),
        ("GENRE", "genre"),
        ("TRACKNUMBER", "track"),
        ("ORIGINATOR", "originator"),
        ("ORIGINATOR_REFERENCE", "originator_reference"),
        ("ORIGINATION_DATE", "origination_date"),
        ("ORIGINATION_TIME", "origination_time"),
        ("TIME_REFERENCE", "time_reference"),
        ("CODING_HISTORY", "coding_history"),
        ("PROJECT", "project"),
        ("SCENE", "scene"),
        ("TAKE", "take"),
        ("TAPE", "tape"),
        ("NOTE", "note"),
    ];
    for (upper, lower) in preferred {
        let value = tags
            .get(upper)
            .or_else(|| tags.get(lower))
            .filter(|s| !s.is_empty());
        if let Some(value) = value {
            comments.insert(upper.to_string(), value.clone());
            written.insert(upper.to_ascii_lowercase());
            written.insert(lower.to_ascii_lowercase());
        }
    }
    for (key, value) in tags {
        if value.is_empty() {
            continue;
        }
        let lower = key.to_ascii_lowercase();
        if written.contains(&lower) {
            continue;
        }
        if preferred.iter().any(|(_, l)| *l == key.as_str()) {
            continue;
        }
        comments.insert(key.clone(), value.clone());
        written.insert(lower);
    }
}
