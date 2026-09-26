// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::{Cursor, Write};

use anyhow::{bail, Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};

use crate::metadata::TagMap;
use crate::pcm::{clamp_unit, planar_frames, to_signed, to_u8};
use crate::spec::{EncodeSpec, EncoderCaps, PcmFormat};
use crate::FormatEncoder;

/// WAV encoder using `hound`, with optional INFO / bext / iXML tags.
pub struct WavEncoder;

const FORMATS: &[PcmFormat] = &[
    PcmFormat::U8,
    PcmFormat::S16,
    PcmFormat::S24,
    PcmFormat::S32,
    PcmFormat::F32,
];

impl FormatEncoder for WavEncoder {
    fn id(&self) -> &'static str {
        "wav"
    }

    fn label(&self) -> &'static str {
        "WAV"
    }

    fn extension(&self) -> &'static str {
        "wav"
    }

    fn capabilities(&self) -> EncoderCaps {
        EncoderCaps {
            sample_formats: Some(FORMATS),
            min_sample_rate: 1,
            max_sample_rate: u32::MAX,
            min_channels: 1,
            max_channels: u16::MAX,
        }
    }

    fn encode_with_tags(
        &self,
        spec: &EncodeSpec,
        planar: &[Vec<f32>],
        writer: &mut dyn Write,
        tags: &TagMap,
    ) -> Result<()> {
        let format = spec.sample_format.context("WAV requires a sample format")?;
        if !self.supports(spec) {
            bail!(
                "WAV cannot encode {format:?} at {} Hz {} ch",
                spec.sample_rate,
                spec.channel_count
            );
        }
        let (bits_per_sample, sample_format) = match format {
            PcmFormat::U8 => (8, SampleFormat::Int),
            PcmFormat::S16 => (16, SampleFormat::Int),
            PcmFormat::S24 => (24, SampleFormat::Int),
            PcmFormat::S32 => (32, SampleFormat::Int),
            PcmFormat::F32 => (32, SampleFormat::Float),
            other => bail!("WAV does not support {other:?}"),
        };
        let wav_spec = WavSpec {
            channels: spec.channel_count,
            sample_rate: spec.sample_rate,
            bits_per_sample,
            sample_format,
        };
        let mut cursor = Cursor::new(Vec::new());
        let mut wav = WavWriter::new(&mut cursor, wav_spec).context("create WAV writer")?;
        let frames = planar_frames(planar);
        let channels = planar.len();
        for frame in 0..frames {
            for ch in 0..channels {
                let sample = planar[ch][frame];
                match format {
                    PcmFormat::U8 => {
                        let signed = (i16::from(to_u8(sample)) - 128) as i8;
                        wav.write_sample(signed).context("write WAV sample")?;
                    }
                    PcmFormat::S16 => {
                        wav.write_sample(to_signed(sample, 16) as i16)
                            .context("write WAV sample")?;
                    }
                    PcmFormat::S24 | PcmFormat::S32 => {
                        wav.write_sample(to_signed(sample, u32::from(format.bits())))
                            .context("write WAV sample")?;
                    }
                    PcmFormat::F32 => {
                        wav.write_sample(clamp_unit(sample))
                            .context("write WAV sample")?;
                    }
                    _ => unreachable!(),
                }
            }
        }
        wav.finalize().context("finalize WAV")?;
        let mut bytes = cursor.into_inner();
        if !tags.is_empty() {
            append_wav_tags(&mut bytes, tags)?;
        }
        writer.write_all(&bytes).context("write WAV bytes")?;
        Ok(())
    }
}

fn append_wav_tags(wav: &mut Vec<u8>, tags: &TagMap) -> Result<()> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("expected RIFF/WAVE buffer from hound");
    }
    let mut extra = Vec::new();
    if let Some(info) = build_info_list(tags) {
        extra.extend_from_slice(&info);
    }
    if let Some(bext) = build_bext(tags) {
        extra.extend_from_slice(b"bext");
        extra.extend_from_slice(&(bext.len() as u32).to_le_bytes());
        extra.extend_from_slice(&bext);
        if bext.len() % 2 == 1 {
            extra.push(0);
        }
    }
    if let Some(ixml) = build_ixml(tags) {
        extra.extend_from_slice(b"iXML");
        extra.extend_from_slice(&(ixml.len() as u32).to_le_bytes());
        extra.extend_from_slice(&ixml);
        if ixml.len() % 2 == 1 {
            extra.push(0);
        }
    }
    if extra.is_empty() {
        return Ok(());
    }
    wav.extend_from_slice(&extra);
    let riff_size = (wav.len() - 8) as u32;
    wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(())
}

fn build_info_list(tags: &TagMap) -> Option<Vec<u8>> {
    let pairs = [
        ("INAM", tags.get("title")),
        ("IART", tags.get("artist")),
        ("IPRD", tags.get("album")),
        (
            "ICMT",
            tags.get("comment").or_else(|| tags.get("description")),
        ),
        (
            "ICRD",
            tags.get("date").or_else(|| tags.get("origination_date")),
        ),
        ("IGNR", tags.get("genre")),
    ];
    let mut body = Vec::new();
    body.extend_from_slice(b"INFO");
    let mut any = false;
    for (fourcc, value) in pairs {
        let Some(text) = value.filter(|s| !s.is_empty()) else {
            continue;
        };
        any = true;
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        if bytes.len() % 2 == 1 {
            bytes.push(0);
        }
        body.extend_from_slice(fourcc.as_bytes());
        body.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        body.extend_from_slice(&bytes);
    }
    if !any {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

fn build_bext(tags: &TagMap) -> Option<Vec<u8>> {
    let originator = tags.get("originator");
    let description = tags
        .get("description")
        .or_else(|| tags.get("comment"))
        .or_else(|| tags.get("title"));
    let originator_reference = tags.get("originator_reference");
    let date = tags.get("origination_date").or_else(|| tags.get("date"));
    let time = tags.get("origination_time");
    let coding = tags.get("coding_history");
    let time_ref = tags
        .get("time_reference")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if originator.is_none()
        && description.is_none()
        && originator_reference.is_none()
        && date.is_none()
        && time.is_none()
        && coding.is_none()
        && time_ref == 0
    {
        return None;
    }
    let mut buf = vec![0u8; 602];
    put_cstr(
        &mut buf[0..256],
        description.map(String::as_str).unwrap_or(""),
    );
    put_cstr(
        &mut buf[256..288],
        originator.map(String::as_str).unwrap_or(""),
    );
    put_cstr(
        &mut buf[288..320],
        originator_reference.map(String::as_str).unwrap_or(""),
    );
    put_cstr(
        &mut buf[320..330],
        date.map(String::as_str).unwrap_or("0000-00-00"),
    );
    put_cstr(
        &mut buf[330..338],
        time.map(String::as_str).unwrap_or("00:00:00"),
    );
    buf[338..346].copy_from_slice(&time_ref.to_le_bytes());
    // version = 0
    if let Some(history) = coding {
        buf.extend_from_slice(history.as_bytes());
        buf.push(0);
    }
    Some(buf)
}

fn build_ixml(tags: &TagMap) -> Option<Vec<u8>> {
    let fields = [
        ("PROJECT", tags.get("project")),
        ("SCENE", tags.get("scene")),
        ("TAKE", tags.get("take")),
        ("TAPE", tags.get("tape")),
        ("NOTE", tags.get("note").or_else(|| tags.get("comment"))),
    ];
    if fields.iter().all(|(_, v)| v.is_none()) {
        return None;
    }
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<BWFXML>\n");
    for (tag, value) in fields {
        if let Some(v) = value.filter(|s| !s.is_empty()) {
            xml.push_str(&format!("  <{tag}>{}</{tag}>\n", escape_xml(v)));
        }
    }
    xml.push_str("</BWFXML>\n");
    Some(xml.into_bytes())
}

fn put_cstr(dst: &mut [u8], text: &str) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&bytes[..n]);
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
