// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Source media metadata → `source.*` variables.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};
use field_variables::{VariableEntry, VariableTable};

/// Canonical host keys used when writing export tags.
pub const CANONICAL_KEYS: &[&str] = &[
    "title",
    "artist",
    "album",
    "comment",
    "date",
    "genre",
    "track",
    "originator",
    "originator_reference",
    "description",
    "coding_history",
    "origination_date",
    "origination_time",
    "time_reference",
    "project",
    "scene",
    "take",
    "tape",
    "note",
];

/// Tag map passed to encoders (canonical keys → values).
pub type TagMap = BTreeMap<String, String>;

/// Probe a media file and build a `source` / `source.*` variable table.
///
/// Always includes technical fields under `source` (`basename`, `sample_rate`,
/// …) when `technical` is provided. Container tags fill sub-scopes.
pub fn probe_source_variables(
    path: &Path,
    technical: Option<&TechnicalSourceFields<'_>>,
) -> VariableTable {
    let mut table = VariableTable::new();
    if let Some(tech) = technical {
        push_technical(&mut table, tech);
    } else {
        // Minimal basename from path.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            table.upsert(VariableEntry::new("source", "basename", name));
        }
    }

    if let Err(err) = probe_riff_chunks(path, &mut table) {
        let _ = err; // best-effort
    }
    if let Err(err) = probe_lofty_tags(path, &mut table) {
        let _ = err;
    }
    table
}

/// Technical fields mirrored into `source.*` (no container tags).
#[derive(Clone, Debug)]
pub struct TechnicalSourceFields<'a> {
    /// Basename.
    pub basename: &'a str,
    /// Sample rate Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channel_count: usize,
    /// Frame count.
    pub frame_count: u64,
    /// Bits per sample.
    pub bits_per_sample: Option<u32>,
    /// Container format label.
    pub container_format: &'a str,
    /// Codec label.
    pub codec: &'a str,
}

fn push_technical(table: &mut VariableTable, tech: &TechnicalSourceFields<'_>) {
    table.upsert(VariableEntry::new("source", "basename", tech.basename));
    table.upsert(VariableEntry::new(
        "source",
        "sample_rate",
        tech.sample_rate.to_string(),
    ));
    table.upsert(VariableEntry::new(
        "source",
        "channel_count",
        tech.channel_count.to_string(),
    ));
    table.upsert(VariableEntry::new(
        "source",
        "frame_count",
        tech.frame_count.to_string(),
    ));
    if let Some(bits) = tech.bits_per_sample {
        table.upsert(VariableEntry::new(
            "source",
            "bits_per_sample",
            bits.to_string(),
        ));
    }
    table.upsert(VariableEntry::new(
        "source",
        "container_format",
        tech.container_format,
    ));
    table.upsert(VariableEntry::new("source", "codec", tech.codec));
}

fn probe_lofty_tags(path: &Path, table: &mut VariableTable) -> Result<()> {
    use lofty::prelude::*;
    use lofty::probe::Probe;

    let tagged = Probe::open(path)
        .context("open for tags")?
        .read()
        .context("read tags")?;
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        // Prefer item keys as vorbis-style for FLAC/Ogg; also map common ones.
        for item in tag.items() {
            let name = item_key_name(&item.key());
            let value = item.value().text().unwrap_or("").to_string();
            if value.is_empty() {
                continue;
            }
            // Route by file type when possible.
            let scope = match tagged.file_type() {
                lofty::file::FileType::Mpeg => "source.id3v2",
                lofty::file::FileType::Flac
                | lofty::file::FileType::Vorbis
                | lofty::file::FileType::Opus => "source.vorbis",
                _ => "source.vorbis",
            };
            table.upsert(VariableEntry::new(scope, name, value));
        }
    }
    Ok(())
}

fn item_key_name(key: &lofty::tag::ItemKey) -> &'static str {
    use lofty::tag::ItemKey;
    match key {
        ItemKey::TrackTitle => "TITLE",
        ItemKey::AlbumTitle => "ALBUM",
        ItemKey::TrackArtist => "ARTIST",
        ItemKey::AlbumArtist => "ALBUMARTIST",
        ItemKey::Comment => "COMMENT",
        ItemKey::Genre => "GENRE",
        ItemKey::RecordingDate => "DATE",
        ItemKey::Year => "YEAR",
        ItemKey::TrackNumber => "TRACKNUMBER",
        ItemKey::DiscNumber => "DISCNUMBER",
        ItemKey::Composer => "COMPOSER",
        ItemKey::CopyrightMessage => "COPYRIGHT",
        ItemKey::EncodedBy => "ENCODEDBY",
        ItemKey::EncoderSettings => "ENCODERSETTINGS",
        ItemKey::Isrc => "ISRC",
        _ => "Unknown",
    }
}

fn probe_riff_chunks(path: &Path, table: &mut VariableTable) -> Result<()> {
    let mut file = File::open(path).context("open riff")?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).context("read riff header")?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Ok(()); // not WAV
    }
    let file_size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64;
    let end = 8 + file_size;
    let mut pos = 12u64;
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok();
        let mut chunk_hdr = [0u8; 8];
        if file.read_exact(&mut chunk_hdr).is_err() {
            break;
        }
        let id = &chunk_hdr[0..4];
        let size = u32::from_le_bytes(chunk_hdr[4..8].try_into().unwrap()) as u64;
        let data_pos = pos + 8;
        if id == b"LIST" {
            let mut list_type = [0u8; 4];
            file.seek(SeekFrom::Start(data_pos)).ok();
            if file.read_exact(&mut list_type).is_ok() && &list_type == b"INFO" {
                parse_info_list(&mut file, data_pos + 4, size.saturating_sub(4), table)?;
            }
        } else if id == b"bext" {
            let mut buf = vec![0u8; size.min(1024 * 1024) as usize];
            file.seek(SeekFrom::Start(data_pos)).ok();
            if file.read_exact(&mut buf).is_ok() {
                parse_bext(&buf, table);
            }
        } else if id == b"iXML" || id == b"ixml" {
            let mut buf = vec![0u8; size.min(4 * 1024 * 1024) as usize];
            file.seek(SeekFrom::Start(data_pos)).ok();
            if file.read_exact(&mut buf).is_ok() {
                parse_ixml(&buf, table);
            }
        }
        pos = data_pos + size + (size % 2); // word align
    }
    Ok(())
}

fn parse_info_list(
    file: &mut File,
    start: u64,
    size: u64,
    table: &mut VariableTable,
) -> Result<()> {
    let mut pos = start;
    let end = start + size;
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok();
        let mut hdr = [0u8; 8];
        if file.read_exact(&mut hdr).is_err() {
            break;
        }
        let fourcc = String::from_utf8_lossy(&hdr[0..4]).trim().to_string();
        let chunk_size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as u64;
        let mut buf = vec![0u8; chunk_size.min(64 * 1024) as usize];
        if file.read_exact(&mut buf).is_ok() {
            let text = cstr_lossy(&buf);
            if !text.is_empty() && !fourcc.is_empty() {
                table.upsert(VariableEntry::new("source.riff", fourcc, text));
            }
        }
        pos += 8 + chunk_size + (chunk_size % 2);
    }
    Ok(())
}

fn parse_bext(buf: &[u8], table: &mut VariableTable) {
    // EBU Tech 3285 layout (minimum 602 bytes for v0).
    if buf.len() < 602 {
        return;
    }
    let description = cstr_lossy(&buf[0..256]);
    let originator = cstr_lossy(&buf[256..288]);
    let originator_reference = cstr_lossy(&buf[288..320]);
    let origination_date = cstr_lossy(&buf[320..330]);
    let origination_time = cstr_lossy(&buf[330..338]);
    let time_reference = u64::from_le_bytes(buf[338..346].try_into().unwrap_or([0; 8]));
    if !description.is_empty() {
        table.upsert(VariableEntry::new("source.bwf", "Description", description));
    }
    if !originator.is_empty() {
        table.upsert(VariableEntry::new("source.bwf", "Originator", originator));
    }
    if !originator_reference.is_empty() {
        table.upsert(VariableEntry::new(
            "source.bwf",
            "OriginatorReference",
            originator_reference,
        ));
    }
    if !origination_date.is_empty() {
        table.upsert(VariableEntry::new(
            "source.bwf",
            "OriginationDate",
            origination_date,
        ));
    }
    if !origination_time.is_empty() {
        table.upsert(VariableEntry::new(
            "source.bwf",
            "OriginationTime",
            origination_time,
        ));
    }
    table.upsert(VariableEntry::new(
        "source.bwf",
        "TimeReference",
        time_reference.to_string(),
    ));
    if buf.len() > 602 {
        let coding = cstr_lossy(&buf[602..]);
        if !coding.is_empty() {
            table.upsert(VariableEntry::new("source.bwf", "CodingHistory", coding));
        }
    }
}

fn parse_ixml(buf: &[u8], table: &mut VariableTable) {
    let text = String::from_utf8_lossy(buf);
    // Flatten known broadcast leaf keys via simple tag scrape. Skip container
    // tags (e.g. SPEED) that wrap nested elements — those are not scalar values.
    for key in [
        "PROJECT",
        "SCENE",
        "TAKE",
        "TAPE",
        "NOTE",
        "CIRCLED",
        "UBITS",
        "TIMESTAMP",
        "FILE_UID",
        "FAMILY_UID",
        "FAMILY_NAME",
        "MASTER_SPEED",
        "CURRENT_SPEED",
        "TIMECODE_FLAG",
        "TIMECODE_RATE",
        "FILE_SAMPLE_RATE",
        "AUDIO_BIT_DEPTH",
        "ORIGINAL_FILENAME",
        "CURRENT_FILENAME",
        "TOTAL_FILES",
        "FILE_SET_INDEX",
        "TRACK_COUNT",
    ] {
        if let Some(value) = extract_xml_leaf(&text, key) {
            table.upsert(VariableEntry::new("source.ixml", key, value));
        }
    }
}

fn extract_xml_leaf(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let value = xml[start..end].trim();
    // Nested markup means this is a container, not a leaf value.
    if value.is_empty() || value.contains('<') {
        None
    } else {
        Some(value.to_string())
    }
}

fn cstr_lossy(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end])
        .trim()
        .trim_end_matches('\0')
        .to_string()
}

/// Build a [`TagMap`] from composed variables + optional metadata templates.
pub fn build_tag_map(
    composed: &VariableTable,
    metadata_templates: &BTreeMap<String, String>,
) -> Result<TagMap, field_variables::InterpolateError> {
    let mut tags = TagMap::new();
    for key in CANONICAL_KEYS {
        if let Some(entry) = composed.get_by_name(key) {
            tags.insert((*key).to_string(), entry.value.clone());
        }
    }
    // Also copy common container leaf names into canonical keys when missing.
    map_alias(&mut tags, composed, "TITLE", "title");
    map_alias(&mut tags, composed, "ARTIST", "artist");
    map_alias(&mut tags, composed, "ALBUM", "album");
    map_alias(&mut tags, composed, "COMMENT", "comment");
    map_alias(&mut tags, composed, "Originator", "originator");
    map_alias(&mut tags, composed, "Description", "description");
    map_alias(&mut tags, composed, "INAM", "title");
    map_alias(&mut tags, composed, "IART", "artist");
    map_alias(&mut tags, composed, "ICMT", "comment");
    map_alias(&mut tags, composed, "PROJECT", "project");
    map_alias(&mut tags, composed, "SCENE", "scene");
    map_alias(&mut tags, composed, "TAKE", "take");

    for (key, template) in metadata_templates {
        let value = field_variables::interpolate_strict(template, composed)?;
        if !value.is_empty() {
            tags.insert(key.clone(), value);
        }
    }
    Ok(tags)
}

fn map_alias(tags: &mut TagMap, composed: &VariableTable, from: &str, to: &str) {
    if tags.contains_key(to) {
        return;
    }
    if let Some(entry) = composed.get_by_name(from) {
        tags.insert(to.to_string(), entry.value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn technical_fields_present() {
        let tech = TechnicalSourceFields {
            basename: "take.wav",
            sample_rate: 48000,
            channel_count: 2,
            frame_count: 100,
            bits_per_sample: Some(24),
            container_format: "wav",
            codec: "pcm",
        };
        let table = probe_source_variables(Path::new("take.wav"), Some(&tech));
        assert_eq!(
            table.get_qualified("source.basename").unwrap().value,
            "take.wav"
        );
        assert_eq!(
            table.get_qualified("source.sample_rate").unwrap().value,
            "48000"
        );
    }

    #[test]
    fn ixml_skips_nested_containers_and_keeps_leaves() {
        let xml = br#"<?xml version="1.0"?>
<BWFXML>
  <PROJECT>Show</PROJECT>
  <SPEED>
    <MASTER_SPEED>30/1</MASTER_SPEED>
    <NOTE></NOTE>
  </SPEED>
  <TAKE>002</TAKE>
</BWFXML>
"#;
        let mut table = VariableTable::new();
        parse_ixml(xml, &mut table);
        assert_eq!(
            table.get_qualified("source.ixml.PROJECT").unwrap().value,
            "Show"
        );
        assert_eq!(
            table.get_qualified("source.ixml.TAKE").unwrap().value,
            "002"
        );
        assert_eq!(
            table
                .get_qualified("source.ixml.MASTER_SPEED")
                .unwrap()
                .value,
            "30/1"
        );
        assert!(table.get_qualified("source.ixml.SPEED").is_none());
    }

    #[test]
    fn riff_info_and_bext_round_trip_read() {
        // Minimal WAV with INFO INAM and bext Originator.
        let mut data = Vec::new();
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&0u32.to_le_bytes()); // placeholder size
        data.extend_from_slice(b"WAVE");
        // fmt
        data.extend_from_slice(b"fmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&44100u32.to_le_bytes());
        data.extend_from_slice(&(44100u32 * 2).to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&16u16.to_le_bytes());
        // data (silence 1 sample)
        data.extend_from_slice(b"data");
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&0i16.to_le_bytes());
        // LIST INFO
        let inam = b"Hello\0";
        let info_size = 4 + 8 + inam.len() as u32;
        data.extend_from_slice(b"LIST");
        data.extend_from_slice(&info_size.to_le_bytes());
        data.extend_from_slice(b"INFO");
        data.extend_from_slice(b"INAM");
        data.extend_from_slice(&(inam.len() as u32).to_le_bytes());
        data.extend_from_slice(inam);
        // bext
        let mut bext = vec![0u8; 602];
        bext[256..259].copy_from_slice(b"BBC");
        data.extend_from_slice(b"bext");
        data.extend_from_slice(&(bext.len() as u32).to_le_bytes());
        data.extend_from_slice(&bext);
        let riff_size = (data.len() - 8) as u32;
        data[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.wav");
        File::create(&path).unwrap().write_all(&data).unwrap();
        let mut table = VariableTable::new();
        probe_riff_chunks(&path, &mut table).unwrap();
        assert_eq!(
            table.get_qualified("source.riff.INAM").unwrap().value,
            "Hello"
        );
        assert_eq!(
            table.get_qualified("source.bwf.Originator").unwrap().value,
            "BBC"
        );
    }
}
