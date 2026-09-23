// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Example CLI: open a `.facomp` (or build one from a media file) and play it
//! on the system default output.
//!
//! Loads `init.lua` (user config else embedded) via `field-scripting`, runs
//! `detect_layout` to choose a channel layout / monitor chain when unset, then
//! plays through the composition's monitoring chain (or Direct).
//!
//! When stdin and stderr are TTYs, shows an in-place playhead and accepts
//! keyboard transport, markers, notes, and save.

mod monitor_process;
mod provider;
mod session;
mod term;

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
    output_device_name, resolve_output_device, MonitorProcess, PlaybackEngine, PlaybackShared,
    PlaybackStats, TransportState,
};
use field_composition::{Composition, PagerStats, MARKER_TYPE_BLUE};
use field_scripting::{BackendHandle, HeadlessBackend, HeadlessWorld, HostProfile, ScriptHost};

use monitor_process::MonitorHostProcess;
use provider::CompositionProvider;
use session::PlaySession;
use term::{AfterSave, DisplayState, Focus, RawModeGuard, UiAction};

/// Marker type name for notes entered with `n`.
const NOTE_MARKER_TYPE: &str = "Note";
/// Green RGBA for the Note marker type.
const NOTE_MARKER_COLOR: [f32; 4] = [0.13, 0.77, 0.37, 1.0];

#[derive(Parser, Debug)]
#[command(
    name = "field-play",
    about = "Play a .facomp or media file through the default audio device",
    long_about = "Loads a FieldAssist composition and plays it on the system \
default output. A media file is opened as a single-clip composition (no \
session). Startup loads init.lua (user config else embedded) and runs \
detect_layout so the default monitoring chain matches FieldAssist. When the \
composition already defines a monitoring chain, that Faust listen path is \
kept; otherwise Direct is used if detect leaves the chain unset. On an \
interactive terminal, shows a playhead and accepts keyboard transport, \
markers (m), notes (n), and save (s)."
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
    let mut session = PlaySession::new(args.path.clone());
    let composition = open_with_detect(session.opened(), args.config_dir.clone())?;

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
    let duration_secs = composition.read().unwrap().duration_secs();
    let start_sample = args
        .start
        .map(|secs| ((secs * f64::from(sample_rate)).round() as u64).min(frames.saturating_sub(1)))
        .unwrap_or(0);

    {
        let c = composition.read().unwrap();
        eprintln!(
            "Playing {} — {:.2}s · {} Hz · {} ch · {} clips · monitor: {monitor_label}",
            c.display_name(),
            duration_secs,
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

    if term::interactive() {
        run_interactive(
            &engine.shared,
            &composition,
            &mut session,
            sample_rate,
            frames,
            duration_secs,
            args.stats,
            deadline,
        )?;
    } else {
        run_batch(
            &engine.shared,
            &composition,
            sample_rate,
            args.stats,
            deadline,
        );
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

fn run_batch(
    shared: &PlaybackShared,
    composition: &Arc<RwLock<Composition>>,
    sample_rate: u32,
    stats: bool,
    deadline: Option<Instant>,
) {
    let mut last_report = Instant::now();
    while shared.transport() == TransportState::Playing {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            shared.set_transport(TransportState::Stopped);
            break;
        }
        if stats && last_report.elapsed() >= Duration::from_secs(1) {
            print_stats(
                &shared.stats(),
                &composition.read().unwrap().pager_stats(),
                sample_rate,
                shared.position(),
            );
            last_report = Instant::now();
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn run_interactive(
    shared: &PlaybackShared,
    composition: &Arc<RwLock<Composition>>,
    session: &mut PlaySession,
    sample_rate: u32,
    frames: u64,
    duration_secs: f64,
    stats: bool,
    deadline: Option<Instant>,
) -> Result<()> {
    term::print_key_help();
    let _raw = RawModeGuard::enter().context("enable terminal raw mode")?;
    let mut last_report = Instant::now();
    let tick = Duration::from_millis(50);
    let mut focus = Focus::Transport;
    let mut display = DisplayState::default();
    // After discard / save-and-quit, leave even if still dirty.
    let mut force_exit = false;

    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            shared.set_transport(TransportState::Stopped);
        }

        if force_exit {
            break;
        }

        // Natural stop / deadline while idle: prompt if dirty, else leave.
        if shared.transport() == TransportState::Stopped && matches!(focus, Focus::Transport) {
            if composition.read().unwrap().is_modified() {
                focus = Focus::ConfirmQuit;
            } else {
                break;
            }
        }

        let actions = term::poll_actions(&mut focus, tick).context("read terminal keys")?;
        for action in actions {
            handle_ui_action(
                shared,
                composition,
                session,
                &mut focus,
                &mut display,
                &mut force_exit,
                action,
                sample_rate,
                frames,
            )?;
            if force_exit {
                break;
            }
        }
        if force_exit {
            break;
        }

        if stats && last_report.elapsed() >= Duration::from_secs(1) {
            let _ = term::push_event_line(
                &mut display,
                &format_stats(
                    &shared.stats(),
                    &composition.read().unwrap().pager_stats(),
                    sample_rate,
                    shared.position(),
                ),
            );
            last_report = Instant::now();
        }

        let pos_secs = shared.position() as f64 / f64::from(sample_rate);
        let prompt = focus.prompt_line();
        let looping = shared.looping.load(std::sync::atomic::Ordering::SeqCst);
        let _ = term::redraw(
            &mut display,
            pos_secs,
            duration_secs,
            shared.transport(),
            looping,
            prompt.as_ref(),
        );
    }

    let _ = term::finish_display(&mut display);
    Ok(())
}

fn handle_ui_action(
    shared: &PlaybackShared,
    composition: &Arc<RwLock<Composition>>,
    session: &mut PlaySession,
    focus: &mut Focus,
    display: &mut DisplayState,
    force_exit: &mut bool,
    action: UiAction,
    sample_rate: u32,
    frames: u64,
) -> Result<()> {
    match action {
        UiAction::RequestQuit => {
            if composition.read().unwrap().is_modified() {
                *focus = Focus::ConfirmQuit;
            } else {
                shared.set_transport(TransportState::Stopped);
                *force_exit = true;
            }
        }
        UiAction::TogglePause => match shared.transport() {
            TransportState::Playing => shared.set_transport(TransportState::Paused),
            TransportState::Paused => shared.set_transport(TransportState::Playing),
            TransportState::Stopped => {}
        },
        UiAction::SeekRel(secs) => {
            let sample = term::seek_sample_after_rel(shared.position(), secs, sample_rate, frames);
            shared.seek_to(sample);
        }
        UiAction::SeekStart => shared.seek_to(0),
        UiAction::SeekEnd => {
            shared.seek_to(term::clamp_seek_sample(frames as i64, frames));
        }
        UiAction::AddMarker => {
            add_marker_at_playhead(
                shared,
                composition,
                display,
                sample_rate,
                MARKER_TYPE_BLUE,
                None,
            )?;
        }
        UiAction::StartNote => {
            let resume_playing = shared.transport() == TransportState::Playing;
            if resume_playing {
                shared.set_transport(TransportState::Paused);
            }
            *focus = Focus::NoteInput {
                buffer: String::new(),
                cursor: 0,
                resume_playing,
            };
        }
        UiAction::Save => {
            begin_save(
                shared,
                composition,
                session,
                focus,
                display,
                force_exit,
                AfterSave::Stay,
            )?;
        }
        UiAction::ToggleLoop => {
            let next = !shared.looping.load(std::sync::atomic::Ordering::SeqCst);
            shared.set_looping(next);
            let _ = term::push_event_line(display, if next { "Looping on" } else { "Looping off" });
        }
        UiAction::CommitNote => {
            let (text, resume) = match focus {
                Focus::NoteInput {
                    buffer,
                    resume_playing,
                    ..
                } => (buffer.clone(), *resume_playing),
                _ => return Ok(()),
            };
            *focus = Focus::Transport;
            ensure_note_marker_type(composition);
            add_marker_at_playhead(
                shared,
                composition,
                display,
                sample_rate,
                NOTE_MARKER_TYPE,
                Some(text),
            )?;
            if resume {
                shared.set_transport(TransportState::Playing);
            }
        }
        UiAction::CancelNote => {
            let resume = match focus {
                Focus::NoteInput { resume_playing, .. } => *resume_playing,
                _ => false,
            };
            *focus = Focus::Transport;
            if resume {
                shared.set_transport(TransportState::Playing);
            }
        }
        UiAction::ConfirmYes => {
            let after = match focus {
                Focus::ConfirmOverwrite { path, after } => {
                    let path = path.clone();
                    let after = *after;
                    let saved = perform_save(composition, session, display, &path);
                    *focus = Focus::Transport;
                    if saved {
                        Some(after)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if after == Some(AfterSave::Quit) {
                shared.set_transport(TransportState::Stopped);
                *force_exit = true;
            }
        }
        UiAction::ConfirmNo => {
            let _ = term::push_event_line(display, "Save cancelled");
            *focus = Focus::Transport;
        }
        UiAction::QuitDiscard => {
            *focus = Focus::Transport;
            shared.set_transport(TransportState::Stopped);
            *force_exit = true;
        }
        UiAction::QuitStay => {
            *focus = Focus::Transport;
            // At EOF, park in Paused so the user can seek / save / resume.
            if shared.transport() == TransportState::Stopped {
                shared.set_transport(TransportState::Paused);
            }
        }
        UiAction::QuitSave => {
            begin_save(
                shared,
                composition,
                session,
                focus,
                display,
                force_exit,
                AfterSave::Quit,
            )?;
        }
    }
    Ok(())
}

fn begin_save(
    shared: &PlaybackShared,
    composition: &Arc<RwLock<Composition>>,
    session: &mut PlaySession,
    focus: &mut Focus,
    display: &mut DisplayState,
    force_exit: &mut bool,
    after: AfterSave,
) -> Result<()> {
    let suggested = composition.read().unwrap().suggested_facomp_name();
    let Some(target) = session.resolve_save_target(&suggested) else {
        let _ = term::push_event_line(display, "Save failed: no target path");
        *focus = Focus::Transport;
        return Ok(());
    };
    if target.needs_overwrite_confirm {
        *focus = Focus::ConfirmOverwrite {
            path: target.path,
            after,
        };
        return Ok(());
    }
    let saved = perform_save(composition, session, display, &target.path);
    *focus = Focus::Transport;
    if saved && after == AfterSave::Quit {
        shared.set_transport(TransportState::Stopped);
        *force_exit = true;
    }
    Ok(())
}

fn perform_save(
    composition: &Arc<RwLock<Composition>>,
    session: &mut PlaySession,
    display: &mut DisplayState,
    path: &std::path::Path,
) -> bool {
    match composition.write().unwrap().save_to_path(path) {
        Ok(()) => {
            session.mark_saved(path.to_path_buf());
            let _ = term::push_event_line(display, &format!("Saved {}", path.display()));
            true
        }
        Err(err) => {
            let _ = term::push_event_line(display, &format!("Save failed: {err:#}"));
            false
        }
    }
}

fn ensure_note_marker_type(composition: &Arc<RwLock<Composition>>) {
    let mut comp = composition.write().unwrap();
    if comp.marker_type_color(NOTE_MARKER_TYPE).is_none() {
        let _ = comp.add_marker_type(NOTE_MARKER_TYPE, NOTE_MARKER_COLOR);
    }
}

fn add_marker_at_playhead(
    shared: &PlaybackShared,
    composition: &Arc<RwLock<Composition>>,
    display: &mut DisplayState,
    sample_rate: u32,
    marker_type: &str,
    note: Option<String>,
) -> Result<()> {
    let frame = shared.position() as u64;
    let secs = frame as f64 / f64::from(sample_rate.max(1));
    let event = term::format_marker_event(secs, marker_type, note.as_deref());
    let inserted = {
        let mut comp = composition.write().unwrap();
        comp.add_marker(frame, marker_type, note).is_some()
    };
    if inserted {
        let _ = term::push_event_line(display, &event);
    } else {
        let _ = term::push_event_line(
            display,
            &format!("{secs:.2} marker {marker_type} already at frame"),
        );
    }
    Ok(())
}

/// Load path into a headless script world, run init + `detect_layout`, return
/// the shared composition (monitor chain may have been filled by layout defaults).
fn open_with_detect(
    path: &std::path::Path,
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
    eprintln!("{}", format_stats(play, pager, sample_rate, position));
}

fn format_stats(
    play: &PlaybackStats,
    pager: &PagerStats,
    sample_rate: u32,
    position: usize,
) -> String {
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
    format!(
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
    )
}
