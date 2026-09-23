// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Interactive TTY transport UI for field-play.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Print;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{execute, queue};
use field_audio_playback::TransportState;

/// What to do after a successful save that was started from a confirm flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterSave {
    /// Remain in the interactive loop.
    Stay,
    /// Stop transport and exit.
    Quit,
}

/// Bottom-of-screen focus: keys go to the focused prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    /// Normal transport / marker keys.
    Transport,
    /// Note text entry below the playhead.
    NoteInput {
        buffer: String,
        cursor: usize,
        resume_playing: bool,
    },
    /// Overwrite confirmation for ephemeral first save.
    ConfirmOverwrite { path: PathBuf, after: AfterSave },
    /// Unsaved changes on quit / end-of-play.
    ConfirmQuit,
}

/// Bottom prompt content for redraw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDraw {
    /// Full prompt text (no fake cursor glyph).
    pub text: String,
    /// 0-based terminal column for the real cursor, if editable.
    pub cursor_col: Option<u16>,
}

impl Focus {
    /// Prompt text for the bottom line, if any.
    pub fn prompt_line(&self) -> Option<PromptDraw> {
        match self {
            Focus::Transport => None,
            Focus::NoteInput { buffer, cursor, .. } => {
                let cursor = (*cursor).min(buffer.len());
                let prefix = "Note: ";
                let col = prefix.chars().count() + buffer[..cursor].chars().count();
                Some(PromptDraw {
                    text: format!("{prefix}{buffer}"),
                    cursor_col: Some(col.min(u16::MAX as usize) as u16),
                })
            }
            Focus::ConfirmOverwrite { path, .. } => Some(PromptDraw {
                text: format!("Overwrite {}? [y/n]", path.display()),
                cursor_col: None,
            }),
            Focus::ConfirmQuit => Some(PromptDraw {
                text: "Unsaved changes — quit? [y]es  [n]o  [s]ave".to_string(),
                cursor_col: None,
            }),
        }
    }
}

/// Keyboard → UI action mapping used by the interactive loop.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    /// Request quit (may open unsaved confirm).
    RequestQuit,
    /// Toggle between playing and paused.
    TogglePause,
    /// Seek by a relative offset in seconds (negative = backward).
    SeekRel(f64),
    /// Seek to sample 0.
    SeekStart,
    /// Seek to the last sample.
    SeekEnd,
    /// Add a Blue marker at the playhead.
    AddMarker,
    /// Enter note-input focus (pause).
    StartNote,
    /// Request save.
    Save,
    /// Commit the note buffer.
    CommitNote,
    /// Cancel note input.
    CancelNote,
    /// Affirmative confirm (overwrite y, or unused).
    ConfirmYes,
    /// Negative confirm (overwrite n).
    ConfirmNo,
    /// Quit without saving.
    QuitDiscard,
    /// Cancel quit confirm.
    QuitStay,
    /// Save then quit.
    QuitSave,
}

/// Tracks whether the last redraw included a prompt line under progress.
#[derive(Debug, Default)]
pub struct DisplayState {
    has_prompt_line: bool,
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
    /// Enter raw mode (required for arrow / modifier keys) and hide the cursor.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stderr = io::stderr();
        execute!(stderr, Hide)?;
        Ok(Self { active: true })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let mut stderr = io::stderr();
            let _ = execute!(stderr, Show);
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

/// Map a key under the current focus, mutating note buffer when editing.
pub fn apply_key(focus: &mut Focus, event: KeyEvent) -> Option<UiAction> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    match focus {
        Focus::Transport => transport_action_from_key(event),
        Focus::NoteInput { buffer, cursor, .. } => note_action_from_key(buffer, cursor, event),
        Focus::ConfirmOverwrite { .. } => overwrite_action_from_key(event),
        Focus::ConfirmQuit => quit_confirm_action_from_key(event),
    }
}

/// Map a transport-mode key event.
pub fn transport_action_from_key(event: KeyEvent) -> Option<UiAction> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    let mods = event.modifiers;
    match event.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(UiAction::RequestQuit),
        KeyCode::Char(' ') => Some(UiAction::TogglePause),
        KeyCode::Char('m') | KeyCode::Char('M') => Some(UiAction::AddMarker),
        KeyCode::Char('n') | KeyCode::Char('N') => Some(UiAction::StartNote),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(UiAction::Save),
        KeyCode::Left => {
            if mods.contains(KeyModifiers::CONTROL) {
                Some(UiAction::SeekStart)
            } else if mods.contains(KeyModifiers::SHIFT) {
                Some(UiAction::SeekRel(-5.0))
            } else {
                Some(UiAction::SeekRel(-1.0))
            }
        }
        KeyCode::Right => {
            if mods.contains(KeyModifiers::CONTROL) {
                Some(UiAction::SeekEnd)
            } else if mods.contains(KeyModifiers::SHIFT) {
                Some(UiAction::SeekRel(5.0))
            } else {
                Some(UiAction::SeekRel(1.0))
            }
        }
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => Some(UiAction::RequestQuit),
        _ => None,
    }
}

fn note_action_from_key(
    buffer: &mut String,
    cursor: &mut usize,
    event: KeyEvent,
) -> Option<UiAction> {
    match event.code {
        KeyCode::Enter => Some(UiAction::CommitNote),
        KeyCode::Esc => Some(UiAction::CancelNote),
        KeyCode::Backspace => {
            if *cursor > 0 {
                let prev = prev_char_boundary(buffer, *cursor);
                buffer.replace_range(prev..*cursor, "");
                *cursor = prev;
            }
            None
        }
        KeyCode::Delete => {
            if *cursor < buffer.len() {
                let next = next_char_boundary(buffer, *cursor);
                buffer.replace_range(*cursor..next, "");
            }
            None
        }
        KeyCode::Left => {
            *cursor = prev_char_boundary(buffer, *cursor);
            None
        }
        KeyCode::Right => {
            *cursor = next_char_boundary(buffer, *cursor);
            None
        }
        KeyCode::Home => {
            *cursor = 0;
            None
        }
        KeyCode::End => {
            *cursor = buffer.len();
            None
        }
        KeyCode::Char(ch) if !event.modifiers.contains(KeyModifiers::CONTROL) => {
            if !ch.is_control() {
                buffer.insert(*cursor, ch);
                *cursor += ch.len_utf8();
            }
            None
        }
        _ => None,
    }
}

fn overwrite_action_from_key(event: KeyEvent) -> Option<UiAction> {
    match event.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(UiAction::ConfirmYes),
        KeyCode::Char('n') | KeyCode::Char('N') => Some(UiAction::ConfirmNo),
        KeyCode::Esc => Some(UiAction::ConfirmNo),
        _ => None,
    }
}

fn quit_confirm_action_from_key(event: KeyEvent) -> Option<UiAction> {
    match event.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(UiAction::QuitDiscard),
        KeyCode::Char('n') | KeyCode::Char('N') => Some(UiAction::QuitStay),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(UiAction::QuitSave),
        KeyCode::Esc => Some(UiAction::QuitStay),
        _ => None,
    }
}

/// Drain pending key events into UI actions (mutates note focus buffer).
pub fn poll_actions(focus: &mut Focus, timeout: Duration) -> io::Result<Vec<UiAction>> {
    let mut actions = Vec::new();
    if !event::poll(timeout)? {
        return Ok(actions);
    }
    loop {
        match event::read()? {
            Event::Key(key) => {
                if let Some(action) = apply_key(focus, key) {
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

/// Format and write progress (+ optional prompt) to stderr.
pub fn redraw(
    display: &mut DisplayState,
    pos_secs: f64,
    dur_secs: f64,
    state: TransportState,
    prompt: Option<&PromptDraw>,
) -> io::Result<()> {
    let mut stderr = io::stderr();
    if display.has_prompt_line {
        queue!(stderr, MoveUp(1))?;
    }
    let progress = format_progress_line(pos_secs, dur_secs, state);
    queue!(
        stderr,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(&progress)
    )?;
    match prompt {
        Some(prompt) => {
            queue!(
                stderr,
                Print("\r\n"),
                Clear(ClearType::CurrentLine),
                Print(&prompt.text)
            )?;
            if let Some(col) = prompt.cursor_col {
                queue!(stderr, Show, MoveToColumn(col))?;
            } else {
                queue!(stderr, Hide)?;
            }
            display.has_prompt_line = true;
        }
        None => {
            if display.has_prompt_line {
                // Clear the stale prompt line, then return to the progress row.
                queue!(
                    stderr,
                    Print("\r\n"),
                    Clear(ClearType::CurrentLine),
                    MoveUp(1)
                )?;
            }
            queue!(stderr, Hide)?;
            display.has_prompt_line = false;
        }
    }
    stderr.flush()
}

/// Commit an event line above the playhead, then leave display ready for redraw.
pub fn push_event_line(display: &mut DisplayState, line: &str) -> io::Result<()> {
    let mut stderr = io::stderr();
    if display.has_prompt_line {
        queue!(stderr, MoveUp(1))?;
    }
    queue!(
        stderr,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(line),
        Print("\r\n")
    )?;
    if display.has_prompt_line {
        // Cursor is on the old prompt line; clear it so the next redraw is clean.
        queue!(stderr, Clear(ClearType::CurrentLine))?;
    }
    display.has_prompt_line = false;
    stderr.flush()
}

/// Finish the progress/prompt region with a newline so later output is clean.
pub fn finish_display(display: &mut DisplayState) -> io::Result<()> {
    let mut stderr = io::stderr();
    if display.has_prompt_line {
        queue!(stderr, MoveUp(1))?;
        display.has_prompt_line = false;
    }
    execute!(
        stderr,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print("\r\n")
    )?;
    Ok(())
}

/// Print a one-line key hint before the first progress redraw.
pub fn print_key_help() {
    eprintln!("Keys: space pause  ←/→ seek  m marker  n note  s save  q quit");
}

/// Format a marker / note event line.
pub fn format_marker_event(secs: f64, marker_type: &str, note: Option<&str>) -> String {
    match note {
        Some(text) if !text.is_empty() => {
            format!("{secs:.2} marker {marker_type}: {text}")
        }
        _ => format!("{secs:.2} marker {marker_type}"),
    }
}

/// Pure formatter used by [`redraw`] and unit tests.
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

fn prev_char_boundary(s: &str, index: usize) -> usize {
    if index == 0 {
        return 0;
    }
    let mut i = index - 1;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn maps_quit_space_and_edit_keys() {
        assert_eq!(
            transport_action_from_key(press(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(UiAction::RequestQuit)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(UiAction::TogglePause)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Char('m'), KeyModifiers::NONE)),
            Some(UiAction::AddMarker)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Char('n'), KeyModifiers::NONE)),
            Some(UiAction::StartNote)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Char('s'), KeyModifiers::NONE)),
            Some(UiAction::Save)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(UiAction::RequestQuit)
        );
    }

    #[test]
    fn maps_arrows_with_modifiers() {
        assert_eq!(
            transport_action_from_key(press(KeyCode::Left, KeyModifiers::NONE)),
            Some(UiAction::SeekRel(-1.0))
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Right, KeyModifiers::NONE)),
            Some(UiAction::SeekRel(1.0))
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Left, KeyModifiers::SHIFT)),
            Some(UiAction::SeekRel(-5.0))
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Right, KeyModifiers::SHIFT)),
            Some(UiAction::SeekRel(5.0))
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Left, KeyModifiers::CONTROL)),
            Some(UiAction::SeekStart)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(UiAction::SeekEnd)
        );
    }

    #[test]
    fn ctrl_takes_precedence_over_shift() {
        let both = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(
            transport_action_from_key(press(KeyCode::Left, both)),
            Some(UiAction::SeekStart)
        );
        assert_eq!(
            transport_action_from_key(press(KeyCode::Right, both)),
            Some(UiAction::SeekEnd)
        );
    }

    #[test]
    fn note_prompt_positions_cursor_without_pipe_glyph() {
        let focus = Focus::NoteInput {
            buffer: "hi".into(),
            cursor: 1,
            resume_playing: true,
        };
        let prompt = focus.prompt_line().expect("prompt");
        assert_eq!(prompt.text, "Note: hi");
        assert!(!prompt.text.contains('|'));
        // "Note: " is 6 columns; cursor after first char of "hi" → column 7.
        assert_eq!(prompt.cursor_col, Some(7));
    }

    #[test]
    fn note_focus_edits_buffer_and_commits() {
        let mut focus = Focus::NoteInput {
            buffer: String::new(),
            cursor: 0,
            resume_playing: true,
        };
        assert_eq!(
            apply_key(&mut focus, press(KeyCode::Char('h'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            apply_key(&mut focus, press(KeyCode::Char('i'), KeyModifiers::NONE)),
            None
        );
        match &focus {
            Focus::NoteInput { buffer, cursor, .. } => {
                assert_eq!(buffer, "hi");
                assert_eq!(*cursor, 2);
            }
            _ => panic!("expected note focus"),
        }
        assert_eq!(
            apply_key(&mut focus, press(KeyCode::Enter, KeyModifiers::NONE)),
            Some(UiAction::CommitNote)
        );
        assert_eq!(
            apply_key(&mut focus, press(KeyCode::Esc, KeyModifiers::NONE)),
            Some(UiAction::CancelNote)
        );
    }

    #[test]
    fn note_focus_ignores_transport_keys() {
        let mut focus = Focus::NoteInput {
            buffer: String::new(),
            cursor: 0,
            resume_playing: false,
        };
        // 'm' inserts as text, does not AddMarker
        assert_eq!(
            apply_key(&mut focus, press(KeyCode::Char('m'), KeyModifiers::NONE)),
            None
        );
        match &focus {
            Focus::NoteInput { buffer, .. } => assert_eq!(buffer, "m"),
            _ => panic!("expected note focus"),
        }
    }

    #[test]
    fn confirm_keys_map() {
        let mut overwrite = Focus::ConfirmOverwrite {
            path: PathBuf::from("x.facomp"),
            after: AfterSave::Stay,
        };
        assert_eq!(
            apply_key(
                &mut overwrite,
                press(KeyCode::Char('y'), KeyModifiers::NONE)
            ),
            Some(UiAction::ConfirmYes)
        );
        assert_eq!(
            apply_key(
                &mut overwrite,
                press(KeyCode::Char('n'), KeyModifiers::NONE)
            ),
            Some(UiAction::ConfirmNo)
        );
        let mut quit = Focus::ConfirmQuit;
        assert_eq!(
            apply_key(&mut quit, press(KeyCode::Char('y'), KeyModifiers::NONE)),
            Some(UiAction::QuitDiscard)
        );
        assert_eq!(
            apply_key(&mut quit, press(KeyCode::Char('s'), KeyModifiers::NONE)),
            Some(UiAction::QuitSave)
        );
        assert_eq!(
            apply_key(&mut quit, press(KeyCode::Char('n'), KeyModifiers::NONE)),
            Some(UiAction::QuitStay)
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
    fn marker_event_formats() {
        assert_eq!(
            format_marker_event(12.34, "Blue", None),
            "12.34 marker Blue"
        );
        assert_eq!(
            format_marker_event(18.5, "Note", Some("verse start")),
            "18.50 marker Note: verse start"
        );
    }

    #[test]
    fn seek_helpers_clamp() {
        assert_eq!(clamp_seek_sample(-10, 1000), 0);
        assert_eq!(clamp_seek_sample(9999, 1000), 999);
        assert_eq!(seek_sample_after_rel(0, -1.0, 44_100, 1000), 0);
        assert_eq!(seek_sample_after_rel(0, 1.0, 44_100, 100_000), 44_100);
    }
}
