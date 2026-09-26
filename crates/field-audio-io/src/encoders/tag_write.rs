// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Post-encode Vorbis-comment tagging via lofty.

use std::io::Write;

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

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
    apply_vorbis_tags(&path, tags)?;
    let tagged = std::fs::read(&path).context("read tagged file")?;
    writer.write_all(&tagged).context("write tagged audio")?;
    Ok(())
}

fn apply_vorbis_tags(path: &std::path::Path, tags: &TagMap) -> Result<()> {
    let mut tagged = Probe::open(path)
        .context("open for tag write")?
        .read()
        .context("read for tag write")?;
    let tag_type = tagged
        .primary_tag()
        .map(|t| t.tag_type())
        .unwrap_or(TagType::VorbisComments);
    let mut tag = Tag::new(tag_type);
    let mapping = [
        ("title", ItemKey::TrackTitle),
        ("artist", ItemKey::TrackArtist),
        ("album", ItemKey::AlbumTitle),
        ("comment", ItemKey::Comment),
        ("description", ItemKey::Comment),
        ("date", ItemKey::RecordingDate),
        ("genre", ItemKey::Genre),
        ("track", ItemKey::TrackNumber),
    ];
    for (key, item) in mapping {
        if let Some(value) = tags.get(key).filter(|s| !s.is_empty()) {
            tag.insert_text(item, value.clone());
        }
    }
    // Extra keys as unknown vorbis comments when supported.
    for (key, value) in tags {
        if mapping.iter().any(|(k, _)| *k == key.as_str()) {
            continue;
        }
        if value.is_empty() {
            continue;
        }
        if let Some(item_key) = ItemKey::from_key(tag_type, key) {
            tag.insert_text(item_key, value.clone());
        }
    }
    tagged.insert_tag(tag);
    tagged
        .save_to_path(path, WriteOptions::default())
        .context("save tags")?;
    Ok(())
}
