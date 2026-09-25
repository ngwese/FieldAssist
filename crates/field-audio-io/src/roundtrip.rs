// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Encode → decode coverage for every registered writer.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use field_audio_process::{max_abs_err, sine};
use tempfile::TempDir;

use crate::pcm::{bits_for_integer, clamp_unit, to_signed, to_u8};
use crate::{
    decode, encoder, encoders, probe_file, probe_header, EncodeSpec, FormatEncoder, PcmFormat,
};

const RATE: u32 = 48_000;
const FRAMES: usize = 4_096;
const LONG_FRAMES: usize = 8_192;

fn quant_tol(format: PcmFormat) -> f32 {
    match format {
        PcmFormat::F32 => 1e-6,
        // Symphonia maps U8 via (u - 128) / 128.
        PcmFormat::U8 => 1.0 / 128.0 + 1e-5,
        other => {
            let bits = u32::from(other.bits());
            let max = if bits >= 32 {
                i32::MAX as f32
            } else {
                ((1i32 << (bits - 1)) - 1) as f32
            };
            1.0 / max + 1e-5
        }
    }
}

fn quantize_plane(samples: &[f32], format: PcmFormat) -> Vec<f32> {
    match format {
        PcmFormat::F32 => samples.iter().copied().map(clamp_unit).collect(),
        PcmFormat::U8 => samples
            .iter()
            .copied()
            .map(|s| (f32::from(to_u8(s)) - 128.0) / 128.0)
            .collect(),
        other => {
            let bits = bits_for_integer(other).unwrap_or(u32::from(other.bits()));
            let max = if bits >= 32 {
                i32::MAX as f32
            } else {
                ((1i32 << (bits - 1)) - 1) as f32
            };
            samples
                .iter()
                .copied()
                .map(|s| to_signed(s, bits) as f32 / max)
                .collect()
        }
    }
}

fn planar_sine(frames: usize, channels: usize, rate: u32) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|ch| sine(frames, rate, 440.0 + ch as f32 * 110.0, 0.5))
        .collect()
}

fn encode_to_path(
    enc: &dyn FormatEncoder,
    spec: &EncodeSpec,
    planar: &[Vec<f32>],
    path: &Path,
) -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    enc.encode(spec, planar, &mut bytes)?;
    if bytes.is_empty() {
        anyhow::bail!("encoder produced no bytes");
    }
    let mut file = fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

fn temp_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn assert_pcm_close(got: &[Vec<f32>], expected: &[Vec<f32>], tol: f32, label: &str) {
    assert_eq!(got.len(), expected.len(), "{label}: channel count");
    for (ch, (a, b)) in got.iter().zip(expected.iter()).enumerate() {
        let n = a.len().min(b.len());
        assert!(
            n > 0,
            "{label}: ch{ch} empty (got {}, expected {})",
            a.len(),
            b.len()
        );
        let err = max_abs_err(&a[..n], &b[..n]);
        assert!(
            err <= tol,
            "{label}: ch{ch} max abs err {err} exceeds tol {tol}"
        );
    }
}

fn expected_decode_channels(encoder_id: &str, input_channels: usize) -> usize {
    if encoder_id == "ogg" && input_channels == 1 {
        2
    } else {
        input_channels
    }
}

#[test]
fn every_writable_format_decodes() {
    let dir = TempDir::new().expect("tempdir");
    for enc in encoders().iter().copied() {
        let caps = enc.capabilities();
        let formats: Vec<Option<PcmFormat>> = match caps.sample_formats {
            Some(list) => list.iter().copied().map(Some).collect(),
            None => vec![None],
        };
        for &channels in &[1usize, 2usize] {
            if (channels as u16) > caps.max_channels || (channels as u16) < caps.min_channels {
                continue;
            }
            for format in &formats {
                let frames = if enc.id() == "flac" {
                    LONG_FRAMES
                } else {
                    FRAMES
                };
                let planar = planar_sine(frames, channels, RATE);
                let spec = EncodeSpec {
                    sample_rate: RATE,
                    sample_format: *format,
                    channel_count: channels as u16,
                };
                assert!(
                    enc.supports(&spec),
                    "{} should support {:?}",
                    enc.id(),
                    spec
                );
                let name = format!(
                    "matrix-{}-{}ch-{}.{}",
                    enc.id(),
                    channels,
                    format
                        .map(|f| f.label().to_string())
                        .unwrap_or_else(|| "na".into()),
                    enc.extension()
                );
                let path = temp_path(&dir, &name);
                encode_to_path(enc, &spec, &planar, &path).unwrap_or_else(|err| {
                    panic!("encode {} {:?}: {err}", enc.id(), format);
                });
                assert!(
                    fs::metadata(&path).unwrap().len() > 0,
                    "{} wrote empty file",
                    enc.id()
                );

                let decoded = decode(&path).unwrap_or_else(|err| {
                    panic!("decode {} {:?}: {err}", enc.id(), format);
                });
                assert_eq!(decoded.sample_rate, RATE, "{} rate", enc.id());
                let expect_ch = expected_decode_channels(enc.id(), channels);
                assert_eq!(decoded.channel_count(), expect_ch, "{} channels", enc.id());

                if enc.id() == "ogg" {
                    assert!(
                        decoded.frames() > frames / 2,
                        "ogg frames {} too short vs {frames}",
                        decoded.frames()
                    );
                } else {
                    assert_eq!(decoded.frames(), frames, "{} frame count", enc.id());
                    let fmt = format.expect("lossless encoder stores format");
                    let expected: Vec<Vec<f32>> =
                        planar.iter().map(|ch| quantize_plane(ch, fmt)).collect();
                    // Mono ogg expands; lossless keeps channel count.
                    assert_pcm_close(
                        &decoded.channels,
                        &expected,
                        quant_tol(fmt),
                        &format!("{} {:?}", enc.id(), fmt),
                    );
                }

                if enc.id() != "ogg" {
                    let probed = probe_file(&path).expect("probe_file");
                    assert_eq!(probed.sample_rate, RATE);
                    assert_eq!(probed.channel_count, channels);
                    assert_eq!(probed.frame_count, frames as u64);
                    if enc.id() == "flac" {
                        let header = probe_header(&path).expect("flac probe_header");
                        assert_eq!(header.frame_count, frames as u64);
                        assert_eq!(
                            header.bits_per_sample,
                            Some(u32::from(format.unwrap().bits()))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn flac_rejects_unsupported_format_and_empty_buffer() {
    let flac = encoder("flac").expect("flac");
    let planar = planar_sine(256, 1, RATE);
    let bad = EncodeSpec {
        sample_rate: RATE,
        sample_format: Some(PcmFormat::F32),
        channel_count: 1,
    };
    let err = flac
        .encode(&bad, &planar, &mut Vec::new())
        .expect_err("F32 FLAC must fail");
    assert!(
        err.to_string().contains("cannot encode"),
        "unexpected error: {err}"
    );

    let empty_spec = EncodeSpec {
        sample_rate: RATE,
        sample_format: Some(PcmFormat::S16),
        channel_count: 1,
    };
    let err = flac
        .encode(&empty_spec, &[Vec::new()], &mut Vec::new())
        .expect_err("empty FLAC must fail");
    assert!(err.to_string().contains("empty"), "unexpected error: {err}");
}

#[test]
fn flac_short_buffer_below_min_block_fails() {
    let flac = encoder("flac").expect("flac");
    let frames = 20usize;
    let planar = planar_sine(frames, 1, RATE);
    let spec = EncodeSpec {
        sample_rate: RATE,
        sample_format: Some(PcmFormat::S16),
        channel_count: 1,
    };
    let err = flac
        .encode(&spec, &planar, &mut Vec::new())
        .expect_err("short FLAC must fail");
    assert!(
        err.to_string().contains("fewer than"),
        "unexpected error: {err}"
    );
}

#[test]
fn flac_non_block_multiple_length_decodes() {
    // Regression: lengths that leave a short final flacenc frame used to
    // produce files Symphonia rejected (silent open in the app).
    let dir = TempDir::new().expect("tempdir");
    let flac = encoder("flac").expect("flac");
    for frames in [4097usize, 4200, 8193, 66_150] {
        let planar = planar_sine(frames, 2, 44_100);
        let spec = EncodeSpec {
            sample_rate: 44_100,
            sample_format: Some(PcmFormat::S16),
            channel_count: 2,
        };
        let path = temp_path(&dir, &format!("odd-{frames}.flac"));
        encode_to_path(flac, &spec, &planar, &path)
            .unwrap_or_else(|err| panic!("encode {frames}: {err}"));
        let decoded = decode(&path).unwrap_or_else(|err| panic!("decode {frames}: {err}"));
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.channel_count(), 2);
        assert_eq!(decoded.frames(), frames, "frames for length {frames}");
        let expected: Vec<Vec<f32>> = planar
            .iter()
            .map(|ch| quantize_plane(ch, PcmFormat::S16))
            .collect();
        assert_pcm_close(
            &decoded.channels,
            &expected,
            quant_tol(PcmFormat::S16),
            &format!("odd-length {frames}"),
        );
        let probed = probe_header(&path).expect("probe");
        assert_eq!(probed.frame_count, frames as u64);
    }
}

#[test]
fn flac_min_block_buffer_decodes() {
    let dir = TempDir::new().expect("tempdir");
    let flac = encoder("flac").expect("flac");
    let frames = 32usize; // flacenc::constant::MIN_BLOCK_SIZE
    let planar = planar_sine(frames, 1, RATE);
    let spec = EncodeSpec {
        sample_rate: RATE,
        sample_format: Some(PcmFormat::S16),
        channel_count: 1,
    };
    let path = temp_path(&dir, "min-block.flac");
    encode_to_path(flac, &spec, &planar, &path).expect("encode min-block flac");
    let decoded = decode(&path).expect("decode min-block flac");
    assert_eq!(decoded.sample_rate, RATE);
    assert_eq!(decoded.frames(), frames);
    let expected = quantize_plane(&planar[0], PcmFormat::S16);
    assert_pcm_close(
        &decoded.channels,
        &[expected],
        quant_tol(PcmFormat::S16),
        "min-block flac",
    );
}

fn wav_flac_wav_chain(format: PcmFormat, channels: usize) {
    let dir = TempDir::new().expect("tempdir");
    let wav = encoder("wav").expect("wav");
    let flac = encoder("flac").expect("flac");
    let planar = planar_sine(FRAMES, channels, RATE);
    let spec = EncodeSpec {
        sample_rate: RATE,
        sample_format: Some(format),
        channel_count: channels as u16,
    };

    let wav1 = temp_path(&dir, &format!("chain-a-{}.wav", format.label()));
    encode_to_path(wav, &spec, &planar, &wav1).expect("wav encode 1");
    let pcm_a = decode(&wav1).expect("decode wav a");

    let flac_path = temp_path(&dir, &format!("chain-{}.flac", format.label()));
    encode_to_path(flac, &spec, &planar, &flac_path).expect("flac encode");
    let pcm_flac = decode(&flac_path).expect("decode flac");

    let wav2 = temp_path(&dir, &format!("chain-b-{}.wav", format.label()));
    encode_to_path(wav, &spec, &pcm_flac.channels, &wav2).expect("wav encode 2");
    let pcm_b = decode(&wav2).expect("decode wav b");

    let tol = quant_tol(format);
    assert_eq!(pcm_a.sample_rate, RATE);
    assert_eq!(pcm_flac.sample_rate, RATE);
    assert_eq!(pcm_b.sample_rate, RATE);
    assert_eq!(pcm_a.channel_count(), channels);
    assert_eq!(pcm_flac.channel_count(), channels);
    assert_eq!(pcm_b.channel_count(), channels);
    assert_eq!(pcm_a.frames(), FRAMES);
    assert_eq!(pcm_flac.frames(), FRAMES);
    assert_eq!(pcm_b.frames(), FRAMES);

    assert_pcm_close(
        &pcm_a.channels,
        &pcm_flac.channels,
        tol,
        &format!("wav≈flac {:?}", format),
    );
    assert_pcm_close(
        &pcm_flac.channels,
        &pcm_b.channels,
        1e-6,
        &format!("flac≈wav2 {:?}", format),
    );

    // File-chain: WAV → decode → FLAC → decode → WAV.
    let flac_from_wav = temp_path(&dir, &format!("from-wav-{}.flac", format.label()));
    encode_to_path(flac, &spec, &pcm_a.channels, &flac_from_wav).expect("flac from wav pcm");
    let pcm_flac2 = decode(&flac_from_wav).expect("decode flac from wav");
    let wav3 = temp_path(&dir, &format!("chain-c-{}.wav", format.label()));
    encode_to_path(wav, &spec, &pcm_flac2.channels, &wav3).expect("wav encode 3");
    let pcm_c = decode(&wav3).expect("decode wav c");
    assert_pcm_close(
        &pcm_a.channels,
        &pcm_flac2.channels,
        tol,
        &format!("file-chain wav→flac {:?}", format),
    );
    assert_pcm_close(
        &pcm_flac2.channels,
        &pcm_c.channels,
        1e-6,
        &format!("file-chain flac→wav {:?}", format),
    );
}

#[test]
fn flac_s8_round_trips_mono_and_stereo() {
    // WAV has no S8 writer; cover FLAC S8 encode→decode directly.
    let dir = TempDir::new().expect("tempdir");
    let flac = encoder("flac").expect("flac");
    for channels in [1usize, 2usize] {
        let planar = planar_sine(FRAMES, channels, RATE);
        let spec = EncodeSpec {
            sample_rate: RATE,
            sample_format: Some(PcmFormat::S8),
            channel_count: channels as u16,
        };
        let path = temp_path(&dir, &format!("s8-{channels}ch.flac"));
        encode_to_path(flac, &spec, &planar, &path).expect("flac s8 encode");
        let decoded = decode(&path).expect("flac s8 decode");
        assert_eq!(decoded.frames(), FRAMES);
        let expected: Vec<Vec<f32>> = planar
            .iter()
            .map(|ch| quantize_plane(ch, PcmFormat::S8))
            .collect();
        assert_pcm_close(
            &decoded.channels,
            &expected,
            quant_tol(PcmFormat::S8),
            &format!("flac s8 {channels}ch"),
        );
    }
}

#[test]
fn wav_flac_wav_s16_mono_and_stereo() {
    wav_flac_wav_chain(PcmFormat::S16, 1);
    wav_flac_wav_chain(PcmFormat::S16, 2);
}

#[test]
fn wav_flac_wav_s24_mono_and_stereo() {
    wav_flac_wav_chain(PcmFormat::S24, 1);
    wav_flac_wav_chain(PcmFormat::S24, 2);
}
