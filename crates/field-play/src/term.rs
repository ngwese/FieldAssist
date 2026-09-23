// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Interactive TTY transport UI for field-play.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crossterm::cursor::MoveToColumn;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Print;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{execute, queue};
use field_audio_playback::TransportState;

/// Keyboard → transport mapping used by the interactive loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportAction {
    /// Stop playback and exit.
    Quit,
    /// Toggle between playing and paused.
    TogglePause,
    /// Seek by a relative offset in seconds (negative = backward).
    SeekRel(f64),
    /// Seek to sample 0.
    SeekStart,
    /// Seek to the last sample.
    SeekEnd,
}

/// True when both stdin and stderr are interactive terminals.
pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// Enables crossterm raw mode for the process lifetime of this guard.
pub struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    /// Enter raw mode (required for arrow / modifier keys).
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self { active: true })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

/// Map a key event to zero or one transport action.
pub fn action_from_key(event: KeyEvent) -> Option<TransportAction> {
    // Ignore key-release / repeat on platforms that emit them.
    if event.kind != KeyEventKind::Press {
        return None;
    }
    let mods = event.modifiers;
    match event.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(TransportAction::Quit),
        KeyCode::Char(' ') => Some(TransportAction::TogglePause),
        KeyCode::Left => {
            if mods.contains(KeyModifiers::CONTROL) {
                Some(TransportAction::SeekStart)
            } else if mods.contains(KeyModifiers::SHIFT) {
                Some(TransportAction::SeekRel(-5.0))
            } else {
                Some(TransportAction::SeekRel(-1.0))
            }
        }
        KeyCode::Right => {
            if mods.contains(KeyModifiers::CONTROL) {
                Some(TransportAction::SeekEnd)
            } else if mods.contains(KeyModifiers::SHIFT) {
                Some(TransportAction::SeekRel(5.0))
            } else {
                Some(TransportAction::SeekRel(1.0))
            }
        }
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => Some(TransportAction::Quit),
        _ => None,
    }
}

/// Drain pending key events (non-blocking) into transport actions.
pub fn poll_actions(timeout: Duration) -> io::Result<Vec<TransportAction>> {
    let mut actions = Vec::new();
    if !event::poll(timeout)? {
        return Ok(actions);
    }
    loop {
        match event::read()? {
            Event::Key(key) => {
                if let Some(action) = action_from_key(key) {
                    actions.push(action);
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
        if !event::poll(Duration::ZERO)? {
            break;
        }
    }
    Ok(actions)
}

/// Format and write the in-place progress line to stderr.
pub fn draw_progress(pos_secs: f64, dur_secs: f64, state: TransportState) -> io::Result<()> {
    let mut stderr = io::stderr();
    let line = format_progress_line(pos_secs, dur_secs, state);
    queue!(
        stderr,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(&line)
    )?;
    stderr.flush()
}

/// Finish the progress row with a newline so later output is clean.
pub fn finish_progress_line() -> io::Result<()> {
    let mut stderr = io::stderr();
    execute!(stderr, Print("\r\n"))?;
    Ok(())
}

/// Print a one-line key hint before the first progress redraw.
pub fn print_key_help() {
    eprintln!("Keys: space pause  ←/→ ±1s  Shift+←/→ ±5s  Ctrl+←/→ start/end  q quit");
}

/// Pure formatter used by [`draw_progress`] and unit tests.
pub fn format_progress_line(pos_secs: f64, dur_secs: f64, state: TransportState) -> String {
    const BAR_WIDTH: usize = 48;
    let dur = dur_secs.max(0.0);
    let pos = pos_secs.clamp(0.0, dur.max(0.0));
    let frac = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((frac * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    let mut bar = String::with_capacity(BAR_WIDTH);
    for i in 0..BAR_WIDTH {
        if i + 1 == filled && filled < BAR_WIDTH {
            bar.push('>');
        } else if i < filled {
            bar.push('=');
        } else {
            bar.push(' ');
        }
    }
    let state_label = match state {
        TransportState::Playing => "Playing",
        TransportState::Paused => "Paused",
        TransportState::Stopped => "Stopped",
    };
    format!("[{bar}] {pos:6.2} / {dur:6.2}s  {state_label}")
}

/// Clamp a seek target in samples to a valid playhead index.
pub fn clamp_seek_sample(sample: i64, frames: u64) -> usize {
    if frames == 0 {
        return 0;
    }
    let max = frames.saturating_sub(1) as i64;
    sample.clamp(0, max) as usize
}

/// Convert a relative seek in seconds to a new sample index.
pub fn seek_sample_after_rel(
    position: usize,
    rel_secs: f64,
    sample_rate: u32,
    frames: u64,
) -> usize {
    let delta = (rel_secs * f64::from(sample_rate.max(1))).round() as i64;
    clamp_seek_sample(position as i64 + delta, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn maps_quit_and_space() {
        assert_eq!(
            action_from_key(press(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(TransportAction::Quit)
        );
        assert_eq!(
            action_from_key(press(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(TransportAction::TogglePause)
        );
        assert_eq!(
            action_from_key(press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(TransportAction::Quit)
        );
    }

    #[test]
    fn maps_arrows_with_modifiers() {
        assert_eq!(
            action_from_key(press(KeyCode::Left, KeyModifiers::NONE)),
            Some(TransportAction::SeekRel(-1.0))
        );
        assert_eq!(
            action_from_key(press(KeyCode::Right, KeyModifiers::NONE)),
            Some(TransportAction::SeekRel(1.0))
        );
        assert_eq!(
            action_from_key(press(KeyCode::Left, KeyModifiers::SHIFT)),
            Some(TransportAction::SeekRel(-5.0))
        );
        assert_eq!(
            action_from_key(press(KeyCode::Right, KeyModifiers::SHIFT)),
            Some(TransportAction::SeekRel(5.0))
        );
        assert_eq!(
            action_from_key(press(KeyCode::Left, KeyModifiers::CONTROL)),
            Some(TransportAction::SeekStart)
        );
        assert_eq!(
            action_from_key(press(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(TransportAction::SeekEnd)
        );
    }

    #[test]
    fn ctrl_takes_precedence_over_shift() {
        let both = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(
            action_from_key(press(KeyCode::Left, both)),
            Some(TransportAction::SeekStart)
        );
        assert_eq!(
            action_from_key(press(KeyCode::Right, both)),
            Some(TransportAction::SeekEnd)
        );
    }

    #[test]
    fn progress_line_contains_times_and_state() {
        let line = format_progress_line(12.34, 54.0, TransportState::Playing);
        assert!(line.contains("12.34"));
        assert!(line.contains("54.00"));
        assert!(line.contains("Playing"));
        assert!(line.starts_with('['));
        assert!(!line.contains("space pause"));
        assert!(line.contains(&"=".repeat(10)));
    }

    #[test]
    fn seek_helpers_clamp() {
        assert_eq!(clamp_seek_sample(-10, 1000), 0);
        assert_eq!(clamp_seek_sample(9999, 1000), 999);
        assert_eq!(seek_sample_after_rel(0, -1.0, 44_100, 1000), 0);
        assert_eq!(seek_sample_after_rel(0, 1.0, 44_100, 100_000), 44_100);
    }
}
