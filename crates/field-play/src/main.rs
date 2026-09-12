// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Example CLI: open a `.facomp` and play it on the system default output.
//!
//! Uses the composition's stored monitoring chain when set; otherwise plays
//! direct (channel map only, no Faust DSP).

mod monitor_process;
mod provider;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use field_audio_monitor::{MonitorChain, MonitorHost};
use field_audio_playback::{
    output_device_name, resolve_output_device, MonitorProcess, PlaybackEngine, PlaybackStats,
    TransportState,
};
use field_composition::{is_facomp_path, Composition, PagerStats};

use monitor_process::MonitorHostProcess;
use provider::CompositionProvider;

#[derive(Parser, Debug)]
#[command(
    name = "field-play",
    about = "Play a .facomp through the default audio device",
    long_about = "Loads a FieldAssist composition and plays it on the system \
default output. When the composition defines a monitoring chain, that Faust \
listen path is used; otherwise audio is sent direct."
)]
struct Args {
    /// Path to a `.facomp` composition file.
    path: PathBuf,

    /// Print playback / pager counters every second and at exit.
    #[arg(long)]
    stats: bool,

    /// Stop after this many seconds (default: play to end).
    #[arg(long)]
    seconds: Option<f64>,

    /// Seek to this timeline position in seconds before playing.
    #[arg(long)]
    start: Option<f64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !is_facomp_path(&args.path) {
        bail!("{} is not a .facomp file", args.path.display());
    }

    let (composition, warnings) = Composition::load_facomp(&args.path)
        .with_context(|| format!("load {}", args.path.display()))?;
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }

    let chain = composition.monitor_chain().and_then(MonitorChain::parse);
    let playback_channels = composition.playback_channels().map(|ch| ch.to_vec());
    let monitor_label = chain
        .map(|c| c.label().to_string())
        .unwrap_or_else(|| "Direct".to_string());
    let sample_rate = composition.sample_rate().max(1);
    let frames = composition.frames();
    let start_sample = args
        .start
        .map(|secs| ((secs * f64::from(sample_rate)).round() as u64).min(frames.saturating_sub(1)))
        .unwrap_or(0);

    eprintln!(
        "Playing {} — {:.2}s · {} Hz · {} ch · {} clips · monitor: {monitor_label}",
        composition.display_name(),
        composition.duration_secs(),
        sample_rate,
        composition.channel_count(),
        composition.clip_count(),
    );
    if start_sample > 0 {
        eprintln!(
            "Start at {:.2}s (sample {start_sample})",
            start_sample as f64 / f64::from(sample_rate)
        );
    }
    if let Some(channels) = playback_channels.as_deref() {
        eprintln!("Playback channel subset: {channels:?}");
    }

    let composition = Arc::new(RwLock::new(composition));
    let provider = Arc::new(CompositionProvider::new(composition.clone()));

    let device = resolve_output_device(None).context("resolve default output device")?;
    eprintln!("Output device: {}", output_device_name(&device));

    let engine = PlaybackEngine::open(&device, provider).context("open playback engine")?;
    let host = Arc::new(MonitorHost::new(engine.shared.output_rate()));
    engine.shared.set_monitor_process(Some(
        Arc::new(MonitorHostProcess::new(host.clone())) as Arc<dyn MonitorProcess>
    ));
    host.set_config(chain, playback_channels, engine.shared.output_rate());

    composition.read().unwrap().reset_pager_stats();
    engine.shared.reset_stats();

    engine.shared.set_position(start_sample as usize);
    engine.shared.set_looping(false);
    engine.shared.bump_epoch();
    engine.shared.set_transport(TransportState::Playing);

    let deadline = args
        .seconds
        .map(|secs| Instant::now() + Duration::from_secs_f64(secs.max(0.0)));
    let mut last_report = Instant::now();

    while engine.shared.transport() == TransportState::Playing {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            engine.shared.set_transport(TransportState::Stopped);
            break;
        }
        if args.stats && last_report.elapsed() >= Duration::from_secs(1) {
            print_stats(
                &engine.shared.stats(),
                &composition.read().unwrap().pager_stats(),
                sample_rate,
                engine.shared.position(),
            );
            last_report = Instant::now();
        }
        thread::sleep(Duration::from_millis(50));
    }

    if args.stats {
        eprintln!("--- final ---");
        print_stats(
            &engine.shared.stats(),
            &composition.read().unwrap().pager_stats(),
            sample_rate,
            engine.shared.position(),
        );
    }

    eprintln!("Done.");
    Ok(())
}

fn print_stats(play: &PlaybackStats, pager: &PagerStats, sample_rate: u32, position: usize) {
    let avg_cb = if play.callbacks == 0 {
        0.0
    } else {
        play.total_callback_ns as f64 / play.callbacks as f64 / 1e6
    };
    let avg_read = if play.provider_reads == 0 {
        0.0
    } else {
        play.total_provider_read_ns as f64 / play.provider_reads as f64 / 1e6
    };
    let avg_decode = if pager.decodes == 0 {
        0.0
    } else {
        pager.decode_ns as f64 / pager.decodes as f64 / 1e6
    };
    eprintln!(
        "pos={:.2}s  cb={} slow={} underrun={} err={}  max_cb={:.2}ms avg_cb={:.2}ms  \
         reads={} max_read={:.2}ms avg_read={:.2}ms  out_frames≤{}  depth={}/{}  \
         pager ram={} spill={} decode={} (max_at={:.2}s avg_dec={:.2}ms total_dec={:.1}ms spill_w={})",
        position as f64 / f64::from(sample_rate.max(1)),
        play.callbacks,
        play.slow_callbacks,
        play.underruns,
        play.stream_errors,
        play.max_callback_ns as f64 / 1e6,
        avg_cb,
        play.provider_reads,
        play.max_provider_read_ns as f64 / 1e6,
        avg_read,
        play.max_out_frames,
        play.prefetch_depth_frames,
        play.max_prefetch_depth_frames,
        pager.ram_hits,
        pager.spill_hits,
        pager.decodes,
        play.slowest_at_sample as f64 / f64::from(sample_rate.max(1)),
        avg_decode,
        pager.decode_ns as f64 / 1e6,
        pager.spill_writes,
    );
}
