// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Example CLI: open a `.facomp` (or build one from a media file) and play it
//! on the system default output.
//!
//! Loads `init.lua` (user config else embedded) via `field-scripting`, runs
//! `detect_layout` to choose a channel layout / monitor chain when unset, then
//! plays through the composition's monitoring chain (or Direct).

mod monitor_process;
mod provider;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use field_audio_monitor::{MonitorChain, MonitorHost};
use field_audio_playback::{
    output_device_name, resolve_output_device, MonitorProcess, PlaybackEngine, PlaybackStats,
    TransportState,
};
use field_composition::{Composition, PagerStats};
use field_scripting::{BackendHandle, HeadlessBackend, HeadlessWorld, HostProfile, ScriptHost};

use monitor_process::MonitorHostProcess;
use provider::CompositionProvider;

#[derive(Parser, Debug)]
#[command(
    name = "field-play",
    about = "Play a .facomp or media file through the default audio device",
    long_about = "Loads a FieldAssist composition and plays it on the system \
default output. A media file is opened as a single-clip composition (no \
session). Startup loads init.lua (user config else embedded) and runs \
detect_layout so the default monitoring chain matches FieldAssist. When the \
composition already defines a monitoring chain, that Faust listen path is \
kept; otherwise Direct is used if detect leaves the chain unset."
)]
struct Args {
    /// Path to a `.facomp` composition or media file.
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

    /// Config directory for `init.lua` (default: FieldAssist config dir).
    #[arg(long)]
    config_dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let composition = open_with_detect(&args.path, args.config_dir.clone())?;

    let chain = composition
        .read()
        .unwrap()
        .monitor_chain()
        .and_then(MonitorChain::parse);
    let playback_channels = composition
        .read()
        .unwrap()
        .playback_channels()
        .map(|ch| ch.to_vec());
    let monitor_label = chain
        .map(|c| c.label().to_string())
        .unwrap_or_else(|| "Direct".to_string());
    let sample_rate = composition.read().unwrap().sample_rate().max(1);
    let frames = composition.read().unwrap().frames();
    let start_sample = args
        .start
        .map(|secs| ((secs * f64::from(sample_rate)).round() as u64).min(frames.saturating_sub(1)))
        .unwrap_or(0);

    {
        let c = composition.read().unwrap();
        eprintln!(
            "Playing {} — {:.2}s · {} Hz · {} ch · {} clips · monitor: {monitor_label}",
            c.display_name(),
            c.duration_secs(),
            sample_rate,
            c.channel_count(),
            c.clip_count(),
        );
        if let Some(layout) = c.channel_layout() {
            eprintln!("Channel layout: {layout}");
        }
    }
    if start_sample > 0 {
        eprintln!(
            "Start at {:.2}s (sample {start_sample})",
            start_sample as f64 / f64::from(sample_rate)
        );
    }
    if let Some(channels) = playback_channels.as_deref() {
        eprintln!("Playback channel subset: {channels:?}");
    }

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

/// Load path into a headless script world, run init + `detect_layout`, return
/// the shared composition (monitor chain may have been filled by layout defaults).
fn open_with_detect(
    path: &PathBuf,
    config_dir: Option<PathBuf>,
) -> Result<Arc<RwLock<Composition>>> {
    let config_dir = config_dir.or_else(field_scripting::user_config_dir);
    let world = Rc::new(RefCell::new(HeadlessWorld::new()));
    let backend: BackendHandle =
        Rc::new(RefCell::new(HeadlessBackend::from_world_rc(world.clone())));
    let mut host = ScriptHost::with_backend(
        HostProfile {
            name: "field-play",
            config_dir,
        },
        backend,
    )
    .map_err(|err| anyhow::anyhow!("create script host: {err}"))?;

    host.load_init()
        .map_err(|err| anyhow::anyhow!("load init.lua: {err}"))?;
    flush_script_output(&host);

    let id = world
        .borrow_mut()
        .open_path(path)
        .with_context(|| format!("open {}", path.display()))?;
    host.fire_detect_layout(id);
    flush_script_output(&host);

    let composition = world
        .borrow()
        .docs
        .get(&id)
        .map(|doc| doc.composition.clone())
        .context("document missing after open")?;
    Ok(composition)
}

fn flush_script_output(host: &ScriptHost) {
    for line in host.take_prints() {
        eprintln!("{line}");
    }
    for (subject, body) in host.take_alerts() {
        eprintln!("alert: {subject}: {body}");
    }
    for entry in host.take_logs() {
        eprintln!(
            "{} [{}] {}",
            entry.level.as_str(),
            entry.topic,
            entry.message
        );
    }
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
    let headroom_pct = if play.last_budget_ns == 0 {
        0.0
    } else {
        100.0 * play.last_headroom_ns as f64 / play.last_budget_ns as f64
    };
    eprintln!(
        "pos={:.2}s  cb={} slow={} underrun={} err={}  max_cb={:.2}ms avg_cb={:.2}ms  \
         budget={:.2}ms last_cb={:.2}ms headroom={:.2}ms ({:.0}%) min_head={:.2}ms frames={}  \
         reads={} max_read={:.2}ms avg_read={:.2}ms  out_frames≤{}  depth={}/{}  \
         pager ram={} spill={} decode={} (max_at={:.2}s avg_dec={:.2}ms total_dec={:.1}ms spill_w={})",
        position as f64 / f64::from(sample_rate.max(1)),
        play.callbacks,
        play.slow_callbacks,
        play.underruns,
        play.stream_errors,
        play.max_callback_ns as f64 / 1e6,
        avg_cb,
        play.last_budget_ns as f64 / 1e6,
        play.last_callback_ns as f64 / 1e6,
        play.last_headroom_ns as f64 / 1e6,
        headroom_pct,
        play.min_headroom_ns as f64 / 1e6,
        play.last_out_frames,
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
