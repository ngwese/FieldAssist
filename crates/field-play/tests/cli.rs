// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! CLI checks for media open + detect_layout (decoder installed, not silent).

use std::f32::consts::TAU;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn empty_config_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp config dir")
}

fn write_stereo_sine(path: &Path, frames: u32, sample_rate: u32) {
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let block_align = channels * bits_per_sample / 8;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_len = frames * u32::from(block_align);
    let mut out = std::fs::File::create(path).unwrap();
    out.write_all(b"RIFF").unwrap();
    out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    out.write_all(b"WAVE").unwrap();
    out.write_all(b"fmt ").unwrap();
    out.write_all(&16u32.to_le_bytes()).unwrap();
    out.write_all(&1u16.to_le_bytes()).unwrap();
    out.write_all(&channels.to_le_bytes()).unwrap();
    out.write_all(&sample_rate.to_le_bytes()).unwrap();
    out.write_all(&byte_rate.to_le_bytes()).unwrap();
    out.write_all(&block_align.to_le_bytes()).unwrap();
    out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
    out.write_all(b"data").unwrap();
    out.write_all(&data_len.to_le_bytes()).unwrap();
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        let sample = (0.5 * (TAU * 440.0 * t).sin() * i16::MAX as f32) as i16;
        for _ in 0..channels {
            out.write_all(&sample.to_le_bytes()).unwrap();
        }
    }
}

#[test]
fn plays_media_file_with_detect_layout_and_decoder() {
    let config = empty_config_dir();
    let media = tempfile::tempdir().unwrap();
    let wav = media.path().join("stereo_sine.wav");
    // ~0.25 s at 44.1 kHz — enough for open + a short play.
    write_stereo_sine(&wav, 11_025, 44_100);

    let bin = env!("CARGO_BIN_EXE_field-play");
    let output = Command::new(bin)
        .args([
            "--config-dir",
            config.path().to_str().unwrap(),
            "--seconds",
            "0.15",
            "--stats",
            wav.to_str().unwrap(),
        ])
        .output()
        .expect("spawn field-play");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        output.status.success(),
        "field-play failed: status={:?}\n{combined}",
        output.status
    );
    assert!(
        !combined.contains("read_interleaved failed"),
        "decoder missing:\n{combined}"
    );
    assert!(
        combined.contains("monitor: Stereo"),
        "expected detect_layout stereo chain:\n{combined}"
    );
    assert!(
        combined.contains("decode=") && !combined.contains("decode=0 ("),
        "expected pager decode activity:\n{combined}"
    );
}
