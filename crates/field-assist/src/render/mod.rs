// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use field_audio_io::{encoder as encoder_by_id, encoders as registry, select_channels};
use field_audio_process::resample_planar;

use crate::model::composition::Composition;
use crate::progress::ProgressHandle;

pub use field_audio_io::{
    format_rate, snap_format, EncodeSpec, FormatEncoder, PcmFormat, RATE_PRESETS,
};

/// Look up built-in encoders.
pub fn encoders() -> &'static [&'static dyn FormatEncoder] {
    registry()
}

/// Look up a built-in encoder by id.
pub fn encoder(id: &str) -> Option<&'static dyn FormatEncoder> {
    encoder_by_id(id)
}

/// Render job parameters for writing a composition to disk.
#[derive(Clone, Debug)]
pub struct RenderJob {
    /// Encoder id (e.g. `"wav"`).
    pub encoder_id: String,
    /// Output format / rate / channels.
    pub spec: EncodeSpec,
    /// Composition channel indices to include.
    pub channel_indices: Vec<usize>,
    /// Destination path.
    pub dest: std::path::PathBuf,
}

/// Render `composition` according to `job`, optionally reporting `progress`.
pub fn render_to_path(
    composition: &Composition,
    job: &RenderJob,
    progress: Option<&ProgressHandle>,
    epoch: u64,
) -> Result<()> {
    let encoder = encoder(&job.encoder_id)
        .with_context(|| format!("unknown encoder `{}`", job.encoder_id))?;
    if job.channel_indices.is_empty() {
        bail!("select at least one channel");
    }
    if !encoder.supports(&job.spec) {
        bail!(
            "{} cannot encode the selected format, rate, or channel count",
            encoder.label()
        );
    }
    if let Some(progress) = progress {
        progress.set_fraction(epoch, 0.05);
    }
    let frames = composition.frames();
    let mut planes = vec![vec![0.0f32; frames as usize]; composition.channel_count()];
    {
        let mut refs: Vec<&mut [f32]> = planes.iter_mut().map(|p| p.as_mut_slice()).collect();
        composition
            .read_planar(0, frames, &mut refs)
            .context("read composition")?;
    }
    if let Some(progress) = progress {
        progress.set_fraction(epoch, 0.4);
    }
    let selected = select_channels(&planes, &job.channel_indices);
    drop(planes);
    let source_rate = composition.sample_rate();
    let planar = if source_rate == job.spec.sample_rate {
        selected
    } else {
        resample_planar(&selected, source_rate, job.spec.sample_rate)?
    };
    if let Some(progress) = progress {
        progress.set_fraction(epoch, 0.7);
    }
    write_encoded(encoder, &job.spec, &planar, &job.dest)?;
    if let Some(progress) = progress {
        progress.set_fraction(epoch, 1.0);
    }
    Ok(())
}

fn write_encoded(
    encoder: &dyn FormatEncoder,
    spec: &EncodeSpec,
    planar: &[Vec<f32>],
    dest: &Path,
) -> Result<()> {
    let file = std::fs::File::create(dest).with_context(|| format!("create {}", dest.display()))?;
    let mut writer = BufWriter::new(file);
    encoder.encode(spec, planar, &mut writer)?;
    writer.flush().context("flush render output")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::model::composition::{Composition, MediaId, MediaRef};
    use field_audio_io::{planar_frames, EncoderCaps};

    fn sine(frames: usize, channels: usize, rate: u32) -> MediaRef {
        let samples = (0..channels)
            .map(|ch| {
                (0..frames)
                    .map(|i| {
                        let t = i as f32 / rate as f32;
                        (t * (440.0 + ch as f32 * 110.0) * std::f32::consts::TAU).sin() * 0.5
                    })
                    .collect()
            })
            .collect();
        MediaRef::from_memory(MediaId(0), rate, samples)
    }

    fn decode_path(path: &Path) -> crate::audio::DecodedAudio {
        crate::audio::decode(path).expect("decode rendered file")
    }

    #[test]
    fn registry_lists_wav_flac_ogg() {
        let ids: Vec<_> = encoders().iter().map(|e| e.id()).collect();
        assert_eq!(ids, vec!["wav", "flac", "ogg"]);
    }

    #[test]
    fn wav_caps_allow_s16_f32_reject_f64() {
        let wav = encoder("wav").unwrap();
        let mut spec = EncodeSpec {
            sample_rate: 44100,
            sample_format: Some(PcmFormat::S16),
            channel_count: 2,
        };
        assert!(wav.supports(&spec));
        spec.sample_format = Some(PcmFormat::F32);
        assert!(wav.supports(&spec));
        spec.sample_format = Some(PcmFormat::F64);
        assert!(!wav.supports(&spec));
    }

    #[test]
    fn flac_caps_allow_s24_reject_f32_and_nine_channels() {
        let flac = encoder("flac").unwrap();
        let mut spec = EncodeSpec {
            sample_rate: 48000,
            sample_format: Some(PcmFormat::S24),
            channel_count: 2,
        };
        assert!(flac.supports(&spec));
        spec.sample_format = Some(PcmFormat::F32);
        assert!(!flac.supports(&spec));
        spec.sample_format = Some(PcmFormat::S16);
        spec.channel_count = 9;
        assert!(!flac.supports(&spec));
    }

    #[test]
    fn vorbis_treats_pcm_format_as_na_and_rejects_zero_hz() {
        let ogg = encoder("ogg").unwrap();
        assert!(!ogg.capabilities().stores_sample_format());
        let mut spec = EncodeSpec {
            sample_rate: 44100,
            sample_format: Some(PcmFormat::F64),
            channel_count: 2,
        };
        assert!(ogg.supports(&spec));
        spec.sample_rate = 0;
        assert!(!ogg.supports(&spec));
    }

    #[test]
    fn wav_encode_round_trips_s16_s24_f32() {
        let comp = Composition::from_media(sine(256, 1, 44100)).unwrap();
        let dir = std::env::temp_dir();
        for format in [PcmFormat::S16, PcmFormat::S24, PcmFormat::F32] {
            let dest = dir.join(format!("snd-render-{format:?}.wav"));
            render_to_path(
                &comp,
                &RenderJob {
                    encoder_id: "wav".into(),
                    spec: EncodeSpec {
                        sample_rate: 44100,
                        sample_format: Some(format),
                        channel_count: 1,
                    },
                    channel_indices: vec![0],
                    dest: dest.clone(),
                },
                None,
                0,
            )
            .unwrap();
            let decoded = decode_path(&dest);
            assert_eq!(decoded.sample_rate, 44100);
            assert_eq!(decoded.channel_count(), 1);
            assert!(decoded.frames() >= 200);
            let _ = std::fs::remove_file(dest);
        }
    }

    #[test]
    fn wav_channel_subset_is_mono() {
        let comp = Composition::from_media(sine(128, 2, 44100)).unwrap();
        let dest = std::env::temp_dir().join("snd-render-subset.wav");
        render_to_path(
            &comp,
            &RenderJob {
                encoder_id: "wav".into(),
                spec: EncodeSpec {
                    sample_rate: 44100,
                    sample_format: Some(PcmFormat::S16),
                    channel_count: 1,
                },
                channel_indices: vec![1],
                dest: dest.clone(),
            },
            None,
            0,
        )
        .unwrap();
        let decoded = decode_path(&dest);
        assert_eq!(decoded.channel_count(), 1);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn flac_s16_decodes() {
        let comp = Composition::from_media(sine(4096, 1, 44100)).unwrap();
        let dest = std::env::temp_dir().join("snd-render.flac");
        render_to_path(
            &comp,
            &RenderJob {
                encoder_id: "flac".into(),
                spec: EncodeSpec {
                    sample_rate: 44100,
                    sample_format: Some(PcmFormat::S16),
                    channel_count: 1,
                },
                channel_indices: vec![0],
                dest: dest.clone(),
            },
            None,
            0,
        )
        .unwrap();
        let decoded = decode_path(&dest);
        assert_eq!(decoded.sample_rate, 44100);
        assert!(decoded.frames() > 0);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn vorbis_short_buffer_decodes() {
        let comp = Composition::from_media(sine(8192, 2, 44100)).unwrap();
        let dest = std::env::temp_dir().join("snd-render.ogg");
        render_to_path(
            &comp,
            &RenderJob {
                encoder_id: "ogg".into(),
                spec: EncodeSpec {
                    sample_rate: 44100,
                    sample_format: None,
                    channel_count: 2,
                },
                channel_indices: vec![0, 1],
                dest: dest.clone(),
            },
            None,
            0,
        )
        .unwrap();
        let decoded = decode_path(&dest);
        assert_eq!(decoded.sample_rate, 44100);
        assert_eq!(decoded.channel_count(), 2);
        assert!(decoded.frames() > 0);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn custom_encoder_can_stand_in_without_app() {
        struct Dummy;

        impl FormatEncoder for Dummy {
            fn id(&self) -> &'static str {
                "dummy"
            }
            fn label(&self) -> &'static str {
                "Dummy"
            }
            fn extension(&self) -> &'static str {
                "bin"
            }
            fn capabilities(&self) -> EncoderCaps {
                EncoderCaps {
                    sample_formats: Some(&[PcmFormat::S16]),
                    min_sample_rate: 1,
                    max_sample_rate: u32::MAX,
                    min_channels: 1,
                    max_channels: 2,
                }
            }
            fn encode(
                &self,
                spec: &EncodeSpec,
                planar: &[Vec<f32>],
                writer: &mut dyn Write,
            ) -> Result<()> {
                writer.write_all(&(spec.sample_rate.to_le_bytes()))?;
                writer.write_all(&(planar.len() as u16).to_le_bytes())?;
                Ok(())
            }
        }

        let dummy = Dummy;
        let spec = EncodeSpec {
            sample_rate: 48000,
            sample_format: Some(PcmFormat::S16),
            channel_count: 1,
        };
        assert!(dummy.supports(&spec));
        let mut out = Vec::new();
        dummy.encode(&spec, &[vec![0.0; 4]], &mut out).unwrap();
        assert_eq!(&out[..4], 48000u32.to_le_bytes());
        assert!(!encoders().iter().any(|encoder| encoder.id() == "dummy"));
    }

    #[test]
    fn resample_helper_is_used_when_rates_differ() {
        let input = vec![vec![0.0f32; 4800]];
        let out = resample_planar(&input, 48000, 44100).unwrap();
        let expected = (4800.0 * 44100.0 / 48000.0) as usize;
        let delta = (out[0].len() as i64 - expected as i64).unsigned_abs() as usize;
        assert!(delta <= expected / 10 + 64);
        assert_eq!(planar_frames(&out), out[0].len());
    }

    #[test]
    fn wav_f32_same_rate_render_is_sample_identity() {
        use field_audio_process::{max_abs_err, sine};
        let rate = 48_000u32;
        let frames = 2048usize;
        let samples = sine(frames, rate, 440.0, 0.5);
        let media = MediaRef::from_memory(MediaId(0), rate, vec![samples.clone()]);
        let comp = Composition::from_media(media).unwrap();
        let dest = std::env::temp_dir().join("fa-identity-render.wav");
        render_to_path(
            &comp,
            &RenderJob {
                encoder_id: "wav".into(),
                spec: EncodeSpec {
                    sample_rate: rate,
                    sample_format: Some(PcmFormat::F32),
                    channel_count: 1,
                },
                channel_indices: vec![0],
                dest: dest.clone(),
            },
            None,
            0,
        )
        .unwrap();
        let decoded = decode_path(&dest);
        let _ = std::fs::remove_file(&dest);
        assert_eq!(decoded.sample_rate, rate);
        let got = &decoded.channels[0];
        let n = frames.min(got.len());
        let err = max_abs_err(&got[..n], &samples[..n]);
        assert!(err < 1e-6, "same-rate f32 render err {err}");
    }

    #[test]
    fn wav_f32_stereo_left_impulse_stays_isolated() {
        let mut left = vec![0.0f32; 128];
        left[0] = 1.0;
        let right = vec![0.0f32; 128];
        let media = MediaRef::from_memory(MediaId(0), 48_000, vec![left, right]);
        let comp = Composition::from_media(media).unwrap();
        let dest = std::env::temp_dir().join("fa-identity-stereo.wav");
        render_to_path(
            &comp,
            &RenderJob {
                encoder_id: "wav".into(),
                spec: EncodeSpec {
                    sample_rate: 48_000,
                    sample_format: Some(PcmFormat::F32),
                    channel_count: 2,
                },
                channel_indices: vec![0, 1],
                dest: dest.clone(),
            },
            None,
            0,
        )
        .unwrap();
        let decoded = decode_path(&dest);
        let _ = std::fs::remove_file(&dest);
        assert!((decoded.channels[0][0] - 1.0).abs() < 1e-6);
        assert!(decoded.channels[1][0].abs() < 1e-6);
    }

    #[test]
    fn decode_wav_f32_round_trips_synthetic_sine() {
        use field_audio_io::{encoder, EncodeSpec, PcmFormat};
        use field_audio_process::{max_abs_err, sine};
        let rate = 44_100u32;
        let samples = sine(1024, rate, 1000.0, 0.25);
        let dest = std::env::temp_dir().join("fa-decode-identity.wav");
        {
            let enc = encoder("wav").expect("wav");
            let file = std::fs::File::create(&dest).unwrap();
            let mut writer = std::io::BufWriter::new(file);
            enc.encode(
                &EncodeSpec {
                    sample_rate: rate,
                    sample_format: Some(PcmFormat::F32),
                    channel_count: 1,
                },
                &[samples.clone()],
                &mut writer,
            )
            .unwrap();
            writer.flush().unwrap();
        }
        let decoded = crate::audio::decode(&dest).unwrap();
        let _ = std::fs::remove_file(&dest);
        let n = samples.len().min(decoded.channels[0].len());
        let err = max_abs_err(&decoded.channels[0][..n], &samples[..n]);
        assert!(err < 1e-6, "decode round-trip err {err}");
    }
}
