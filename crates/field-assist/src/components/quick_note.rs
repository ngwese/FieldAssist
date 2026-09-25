// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Floating quick-note entry over the waveform (`n` key).

use gpui_kit::component::{
    input::{InputEvent, InputState},
    IconNamed,
};
use gpui_kit::{AppContext as _, Context, Entity, Focusable as _, SharedString, Window};

use crate::app::AppView;

/// Marker type name used by quick notes (matches field-play).
pub const NOTE_MARKER_TYPE: &str = "Note";
/// Default green for the Note marker type (matches field-play).
pub const NOTE_MARKER_COLOR: [f32; 4] = [0.13, 0.77, 0.37, 1.0];

/// Lucide `message-square` via [`crate::assets::EXTRA_ICONS`].
pub struct MessageSquareIcon;

impl IconNamed for MessageSquareIcon {
    fn path(self) -> SharedString {
        "icons/message-square.svg".into()
    }
}

/// In-progress quick note session owned by [`AppView`].
pub struct QuickNoteSession {
    /// Text field under edit.
    pub input: Entity<InputState>,
    /// Sample frame captured when the session opened (hover or caret).
    pub sample: usize,
    /// Resume playback after commit/cancel when we paused for the prompt.
    pub resume_playing: bool,
}

impl QuickNoteSession {
    /// Create an empty focused note field at `sample`.
    pub fn begin(
        sample: usize,
        resume_playing: bool,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| {
            state.set_placeholder("Enter a note...", window, cx);
        });
        cx.subscribe_in(
            &input,
            window,
            |this, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => this.commit_quick_note(window, cx),
                InputEvent::Blur => {
                    // Esc/X/`take` clear the session first; Blur then no-ops.
                    // Clicking away also dismisses without creating a note.
                    this.cancel_quick_note(window, cx);
                }
                _ => {}
            },
        )
        .detach();
        input.read(cx).focus_handle(cx).focus(window, cx);
        Self {
            input,
            sample,
            resume_playing,
        }
    }

    /// Current field text.
    pub fn text(&self, cx: &gpui_kit::App) -> String {
        self.input.read(cx).value().to_string()
    }
}
