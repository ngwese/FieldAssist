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
    "copyright",
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

/// Stable Vorbis comment keys for BWF `bext` leaves written to FLAC/Ogg.
///
/// Mapping: `source.bwf.<Leaf>` → Vorbis key (uppercase).
pub const BWF_VORBIS_KEYS: &[(&str, &str)] = &[
    ("Description", "DESCRIPTION"),
    ("Originator", "ORIGINATOR"),
    ("OriginatorReference", "ORIGINATOR_REFERENCE"),
    ("OriginationDate", "ORIGINATION_DATE"),
    ("OriginationTime", "ORIGINATION_TIME"),
    ("TimeReference", "TIME_REFERENCE"),
    ("CodingHistory", "CODING_HISTORY"),
];

/// Stable Vorbis comment keys for common iXML leaves written to FLAC/Ogg.
///
/// Other `source.ixml.*` leaves use `IXML_<LEAF>` (uppercase leaf name).
pub const IXML_VORBIS_KEYS: &[(&str, &str)] = &[
    ("PROJECT", "PROJECT"),
    ("SCENE", "SCENE"),
    ("TAKE", "TAKE"),
    ("TAPE", "TAPE"),
    ("NOTE", "NOTE"),
    ("PROJECTNAME", "PROJECT"),
];

/// Tag map passed to encoders (canonical keys → values).
pub type TagMap = BTreeMap<String, String>;

/// Probe a media file and build a `source` / `source.*` variable table.
///
/// Always includes technical fields under `source` (`basename`, `stem`,
/// `parent`, `sample_rate`, …) when `technical` is provided. Container tags
/// fill sub-scopes. `parent` is the parent directory of `path` when known.
pub fn probe_source_variables(
    path: &Path,
    technical: Option<&TechnicalSourceFields<'_>>,
) -> VariableTable {
    let mut table = VariableTable::new();
    if let Some(tech) = technical {
        push_technical(&mut table, tech);
    } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        table.upsert(VariableEntry::new("source", "basename", name));
        push_stem(&mut table, Path::new(name));
    }
    push_parent(&mut table, path);

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
    push_stem(table, Path::new(tech.basename));
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

/// Basename without extension (`source.stem`), matching `field.url` `.stem`.
fn push_stem(table: &mut VariableTable, path: &Path) {
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    table.upsert(VariableEntry::new("source", "stem", stem));
}

/// Parent directory of the media path (`source.parent`).
fn push_parent(table: &mut VariableTable, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    table.upsert(VariableEntry::new("source", "parent", parent));
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
///
/// Includes canonical keys, BWF/iXML → stable Vorbis keys ([`BWF_VORBIS_KEYS`],
/// [`IXML_VORBIS_KEYS`]), and prefers `user.artist` / `user.copyright` when set.
pub fn build_tag_map(
    composed: &VariableTable,
    metadata_templates: &BTreeMap<String, String>,
) -> Result<TagMap, field_variables::InterpolateError> {
    let mut tags = TagMap::new();
    for key in CANONICAL_KEYS {
        if let Some(value) = first_nonempty_by_name(composed, key) {
            tags.insert((*key).to_string(), value.to_string());
        }
    }
    // Also copy common container leaf names into canonical keys when missing.
    map_alias(&mut tags, composed, "TITLE", "title");
    map_alias(&mut tags, composed, "ARTIST", "artist");
    map_alias(&mut tags, composed, "ALBUM", "album");
    map_alias(&mut tags, composed, "COMMENT", "comment");
    map_alias(&mut tags, composed, "COPYRIGHT", "copyright");
    map_alias(&mut tags, composed, "Originator", "originator");
    map_alias(&mut tags, composed, "Description", "description");
    map_alias(&mut tags, composed, "INAM", "title");
    map_alias(&mut tags, composed, "IART", "artist");
    map_alias(&mut tags, composed, "ICMT", "comment");
    map_alias(&mut tags, composed, "ICOP", "copyright");
    map_alias(&mut tags, composed, "PROJECT", "project");
    map_alias(&mut tags, composed, "SCENE", "scene");
    map_alias(&mut tags, composed, "TAKE", "take");

    map_bwf_and_ixml_vorbis(&mut tags, composed);
    prefer_user_artist_copyright(&mut tags, composed);

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
    if let Some(value) = first_nonempty_by_name(composed, from) {
        tags.insert(to.to_string(), value.to_string());
    }
}

fn first_nonempty_by_name<'a>(composed: &'a VariableTable, name: &str) -> Option<&'a str> {
    composed
        .entries()
        .iter()
        .find(|e| e.name == name && !e.value.is_empty())
        .map(|e| e.value.as_str())
}

fn insert_nonempty(tags: &mut TagMap, key: impl Into<String>, value: &str) {
    if !value.is_empty() {
        tags.insert(key.into(), value.to_string());
    }
}

/// Map `source.bwf.*` and `source.ixml.*` into stable Vorbis comment keys.
fn map_bwf_and_ixml_vorbis(tags: &mut TagMap, composed: &VariableTable) {
    for (leaf, vorbis_key) in BWF_VORBIS_KEYS {
        if let Some(entry) = composed.get_qualified(&format!("source.bwf.{leaf}")) {
            insert_nonempty(tags, *vorbis_key, &entry.value);
            // Keep lowercase canonical aliases for ItemKey mapping.
            let canonical = match *leaf {
                "Description" => Some("description"),
                "Originator" => Some("originator"),
                "OriginatorReference" => Some("originator_reference"),
                "OriginationDate" => Some("origination_date"),
                "OriginationTime" => Some("origination_time"),
                "TimeReference" => Some("time_reference"),
                "CodingHistory" => Some("coding_history"),
                _ => None,
            };
            if let Some(canon) = canonical {
                if !tags.contains_key(canon) {
                    insert_nonempty(tags, canon, &entry.value);
                }
            }
        }
    }

    let mut known_ixml = std::collections::BTreeSet::new();
    for (leaf, vorbis_key) in IXML_VORBIS_KEYS {
        known_ixml.insert(*leaf);
        if let Some(entry) = composed.get_qualified(&format!("source.ixml.{leaf}")) {
            insert_nonempty(tags, *vorbis_key, &entry.value);
            let canonical = leaf.to_ascii_lowercase();
            if matches!(
                canonical.as_str(),
                "project" | "scene" | "take" | "tape" | "note"
            ) && !tags.contains_key(&canonical)
            {
                insert_nonempty(tags, canonical, &entry.value);
            }
        }
    }

    for entry in composed.entries() {
        if entry.scope != "source.ixml" || entry.value.is_empty() {
            continue;
        }
        if known_ixml.contains(entry.name.as_str()) {
            continue;
        }
        let key = format!("IXML_{}", entry.name.to_ascii_uppercase());
        tags.entry(key).or_insert_with(|| entry.value.clone());
    }
}

/// Prefer `user.artist` / `user.copyright` over other artist/copyright sources.
fn prefer_user_artist_copyright(tags: &mut TagMap, composed: &VariableTable) {
    if let Some(entry) = composed.get_qualified("user.artist") {
        if !entry.value.is_empty() {
            tags.insert("artist".to_string(), entry.value.clone());
            tags.insert("ARTIST".to_string(), entry.value.clone());
        }
    }
    if let Some(entry) = composed.get_qualified("user.copyright") {
        if !entry.value.is_empty() {
            tags.insert("copyright".to_string(), entry.value.clone());
            tags.insert("COPYRIGHT".to_string(), entry.value.clone());
        }
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
        let table = probe_source_variables(Path::new("/recordings/day1/take.wav"), Some(&tech));
        assert_eq!(
            table.get_qualified("source.basename").unwrap().value,
            "take.wav"
        );
        assert_eq!(table.get_qualified("source.stem").unwrap().value, "take");
        assert_eq!(
            table.get_qualified("source.parent").unwrap().value,
            "/recordings/day1"
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

    #[test]
    fn build_tag_map_maps_bwf_and_ixml_to_stable_vorbis_keys() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new(
            "source.bwf",
            "Description",
            "Field take",
        ));
        table.upsert(VariableEntry::new("source.bwf", "Originator", "Greg"));
        table.upsert(VariableEntry::new("source.ixml", "PROJECT", "Show"));
        table.upsert(VariableEntry::new("source.ixml", "MASTER_SPEED", "30/1"));
        let tags = build_tag_map(&table, &BTreeMap::new()).unwrap();
        assert_eq!(
            tags.get("DESCRIPTION").map(String::as_str),
            Some("Field take")
        );
        assert_eq!(tags.get("ORIGINATOR").map(String::as_str), Some("Greg"));
        assert_eq!(tags.get("PROJECT").map(String::as_str), Some("Show"));
        assert_eq!(
            tags.get("IXML_MASTER_SPEED").map(String::as_str),
            Some("30/1")
        );
        assert!(!tags.contains_key("IXML_PROJECT"));
    }

    #[test]
    fn build_tag_map_prefers_user_artist_and_copyright() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new("source", "artist", "FromSource"));
        table.upsert(VariableEntry::new("user", "artist", "FromUser"));
        table.upsert(VariableEntry::new("user", "copyright", "© 2026"));
        let tags = build_tag_map(&table, &BTreeMap::new()).unwrap();
        assert_eq!(tags.get("artist").map(String::as_str), Some("FromUser"));
        assert_eq!(tags.get("ARTIST").map(String::as_str), Some("FromUser"));
        assert_eq!(tags.get("copyright").map(String::as_str), Some("© 2026"));
        assert_eq!(tags.get("COPYRIGHT").map(String::as_str), Some("© 2026"));
    }

    #[test]
    fn build_tag_map_omits_empty_user_artist() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new("user", "artist", ""));
        table.upsert(VariableEntry::new("source", "artist", "KeepMe"));
        let tags = build_tag_map(&table, &BTreeMap::new()).unwrap();
        assert_eq!(tags.get("artist").map(String::as_str), Some("KeepMe"));
    }

    #[test]
    fn flac_encode_writes_mapped_vorbis_comments() {
        use crate::{encoder, EncodeSpec, PcmFormat};

        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new(
            "source.bwf",
            "Description",
            "Catalog note",
        ));
        table.upsert(VariableEntry::new("source.bwf", "Originator", "FA"));
        table.upsert(VariableEntry::new("user", "artist", "Tester"));
        table.upsert(VariableEntry::new("user", "copyright", "MIT"));
        let tags = build_tag_map(&table, &BTreeMap::new()).unwrap();

        let enc = encoder("flac").expect("flac");
        let spec = EncodeSpec {
            sample_rate: 48_000,
            sample_format: Some(PcmFormat::S16),
            channel_count: 1,
        };
        let planar = [vec![0.0f32; 256]];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tagged.flac");
        let mut file = File::create(&path).unwrap();
        enc.encode_with_tags(&spec, &planar, &mut file, &tags)
            .unwrap();
        drop(file);

        use lofty::file::AudioFile;
        use lofty::flac::FlacFile;

        let mut reader = File::open(&path).unwrap();
        let flac = FlacFile::read_from(&mut reader, lofty::config::ParseOptions::new()).unwrap();
        let comments = flac.vorbis_comments().expect("vorbis comments");
        assert_eq!(comments.get("DESCRIPTION"), Some("Catalog note"));
        assert_eq!(comments.get("ARTIST"), Some("Tester"));
        assert_eq!(comments.get("COPYRIGHT"), Some("MIT"));
        assert_eq!(comments.get("ORIGINATOR"), Some("FA"));
    }
}
