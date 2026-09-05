// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::{bail, Context, Result};
use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use rusty_vorbis::VorbisEncoder;

use crate::render::pcm::interleave_f32;
use crate::render::spec::{EncodeSpec, EncoderCaps};
use crate::render::FormatEncoder;

pub struct VorbisEncoderImpl;

impl FormatEncoder for VorbisEncoderImpl {
    fn id(&self) -> &'static str {
        "ogg"
    }

    fn label(&self) -> &'static str {
        "Ogg Vorbis"
    }

    fn extension(&self) -> &'static str {
        "ogg"
    }

    fn capabilities(&self) -> EncoderCaps {
        EncoderCaps {
            sample_formats: None,
            min_sample_rate: 1,
            max_sample_rate: u32::MAX,
            min_channels: 1,
            max_channels: 255,
        }
    }

    fn encode(&self, spec: &EncodeSpec, planar: &[Vec<f32>], writer: &mut dyn Write) -> Result<()> {
        if !self.supports(spec) {
            bail!(
                "Ogg Vorbis cannot encode {} Hz {} ch",
                spec.sample_rate,
                spec.channel_count
            );
        }
        // rusty_vorbis ships a stereo-coupled setup header. Mono is encoded as
        // dual-mono so the ident/setup channel counts stay consistent.
        let mut planes = planar.to_vec();
        if planes.len() == 1 {
            planes.push(planes[0].clone());
        }
        let channels = u16::try_from(planes.len()).context("Vorbis channel count")?;
        let pcm = interleave_f32(&planes);
        let mut encoder = VorbisEncoder::default();
        encoder
            .push_pcm_f32(&pcm, channels, spec.sample_rate)
            .map_err(|err| anyhow::anyhow!("Vorbis encode: {err}"))?;
        encoder.finish();

        let mut packets = Vec::new();
        loop {
            match encoder.next_packet() {
                Ok(packet) => packets.push(packet),
                Err(rusty_vorbis::Error::Eof) => break,
                Err(rusty_vorbis::Error::Again) => continue,
                Err(err) => bail!("Vorbis packet: {err}"),
            }
        }
        if packets.len() < 3 {
            bail!("Vorbis encoder produced no header packets");
        }
        let last = packets.len() - 1;
        let mut ogg = PacketWriter::new(writer);
        let serial = 1u32;
        for (index, packet) in packets.into_iter().enumerate() {
            let end = if index < 3 && index != last {
                PacketWriteEndInfo::EndPage
            } else if index == last {
                PacketWriteEndInfo::EndStream
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            ogg.write_packet(packet.data, serial, end, packet.pts.max(0) as u64)
                .context("write Ogg packet")?;
        }
        Ok(())
    }
}
