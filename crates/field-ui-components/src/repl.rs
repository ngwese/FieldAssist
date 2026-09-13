// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Script REPL panel with a gpui-free [`ReplOutput`] DTO.

use std::rc::Rc;

use crate::theme::content_foreground;
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex, ActiveTheme as _, Sizable as _,
};
use gpui_kit::{
    div, rems, AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window,
};

/// Evaluation result shown in the transcript.
#[derive(Clone, Debug, Default)]
pub struct ReplOutput {
    /// Printed lines from the evaluation.
    pub prints: Vec<String>,
    /// Optional result string.
    pub result: Option<String>,
    /// Optional error string.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind {
    Input,
    Output,
    Error,
}

#[derive(Clone)]
struct TranscriptLine {
    kind: LineKind,
    text: SharedString,
}

/// Invoked when the user submits a line of code.
pub type ReplEvalHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Bottom-dock script REPL panel.
pub struct ReplPanel {
    title: SharedString,
    input: Entity<InputState>,
    lines: Vec<TranscriptLine>,
    history: Vec<String>,
    history_index: Option<usize>,
    on_eval: Option<ReplEvalHandler>,
    scroll: ScrollHandle,
}

impl ReplPanel {
    /// Create a REPL panel with the given dock tab title.
    pub fn new(
        title: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if !matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false
                    }
                ) {
                    return;
                }
                let code = input.read(cx).value().to_string();
                if code.trim().is_empty() {
                    return;
                }
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
                this.history.retain(|item| item != &code);
                this.history.push(code.clone());
                this.history_index = None;
                let handler = this.on_eval.clone();
                window.defer(cx, move |window, cx| {
                    if let Some(handler) = handler {
                        handler(code, window, cx);
                    }
                });
            },
        )
        .detach();
        Self {
            title: title.into(),
            input,
            lines: Vec::new(),
            history: Vec::new(),
            history_index: None,
            on_eval: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// Set the evaluation handler.
    pub fn set_handler(&mut self, handler: ReplEvalHandler) {
        self.on_eval = Some(handler);
    }

    /// Append an input line and its evaluation output.
    pub fn append_eval(&mut self, code: &str, output: &ReplOutput, cx: &mut Context<Self>) {
        self.lines.push(TranscriptLine {
            kind: LineKind::Input,
            text: code.to_string().into(),
        });
        self.push_output(output);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Append evaluation output without an input line.
    pub fn append_output(&mut self, output: &ReplOutput, cx: &mut Context<Self>) {
        self.push_output(output);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Append a standalone error line.
    pub fn append_error(&mut self, error: &str, cx: &mut Context<Self>) {
        self.lines.push(TranscriptLine {
            kind: LineKind::Error,
            text: error.to_string().into(),
        });
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn push_output(&mut self, output: &ReplOutput) {
        for line in &output.prints {
            self.lines.push(TranscriptLine {
                kind: LineKind::Output,
                text: line.clone().into(),
            });
        }
        if let Some(result) = &output.result {
            self.lines.push(TranscriptLine {
                kind: LineKind::Output,
                text: result.clone().into(),
            });
        }
        if let Some(error) = &output.error {
            self.lines.push(TranscriptLine {
                kind: LineKind::Error,
                text: error.clone().into(),
            });
        }
    }

    fn history_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(index) => index - 1,
        };
        self.history_index = Some(index);
        let value = self.history[index].clone();
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
    }

    fn history_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 >= self.history.len() {
            self.history_index = None;
            self.input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
            return;
        }
        let index = index + 1;
        self.history_index = Some(index);
        let value = self.history[index].clone();
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
    }

    fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.focus_handle(cx).focus(window, cx);
    }
}

impl EventEmitter<PanelEvent> for ReplPanel {}

impl Focusable for ReplPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl BasePanel for ReplPanel {
    fn panel_name(&self) -> &'static str {
        "ReplPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ReplPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.title.clone())
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ReplPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let content = content_foreground(cx);
        let lines = self.lines.clone();
        v_flex()
            .id("repl-panel")
            .key_context("Repl")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .text_sm()
            .line_height(rems(1.25))
            .on_click(cx.listener(|this, _, window, cx| {
                this.focus_input(window, cx);
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "up" => {
                        this.history_prev(window, cx);
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.history_next(window, cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .children(lines.into_iter().enumerate().map(|(ix, line)| {
                let color = match line.kind {
                    LineKind::Input => content,
                    LineKind::Output => muted,
                    LineKind::Error => danger,
                };
                if line.kind == LineKind::Input {
                    prompt_row(("repl-line", ix as u64), muted, color, line.text)
                } else {
                    div()
                        .id(("repl-line", ix as u64))
                        .w_full()
                        .flex_none()
                        .text_color(color)
                        .child(line.text)
                        .into_any_element()
                }
            }))
            .child(prompt_row(
                "repl-prompt",
                muted,
                content,
                Input::new(&self.input)
                    .appearance(false)
                    .bordered(false)
                    .cleanable(false)
                    .small()
                    .px_0()
                    .py_0()
                    .h_auto()
                    .w_full(),
            ))
    }
}

fn prompt_row(
    id: impl Into<ElementId>,
    prompt_color: Hsla,
    body_color: Hsla,
    body: impl IntoElement,
) -> AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .flex_none()
        .items_center()
        .gap_1()
        .child(div().flex_none().text_color(prompt_color).child(">"))
        .child(div().flex_1().min_w_0().text_color(body_color).child(body))
        .into_any_element()
}
