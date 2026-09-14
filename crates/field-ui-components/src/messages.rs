// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Log / messages panel with a gpui-free [`LogLine`] DTO.

use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _,
};
use gpui_kit::{
    div, rems, uniform_list, App, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    UniformListScrollHandle, Window,
};

const LEVEL_WIDTH: gpui_kit::Rems = rems(4.);
const TOPIC_WIDTH: gpui_kit::Rems = rems(5.5);

/// Severity of a log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Informational message.
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

impl LogLevel {
    /// Short label for the level column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One log line shown in the messages panel.
#[derive(Clone, Debug)]
pub struct LogLine {
    /// Severity.
    pub level: LogLevel,
    /// Topic / subsystem label.
    pub topic: String,
    /// Message body.
    pub text: String,
}

/// Bottom-dock messages panel.
pub struct MessagesPanel {
    entries: Vec<LogLine>,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
}

impl MessagesPanel {
    /// Create an empty messages panel.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// Append log lines.
    pub fn append(&mut self, entries: Vec<LogLine>, cx: &mut Context<Self>) {
        if entries.is_empty() {
            return;
        }
        self.entries.extend(entries);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Number of error lines currently in the panel.
    pub fn error_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.level == LogLevel::Error)
            .count()
    }

    /// Number of warning lines currently in the panel.
    pub fn warn_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.level == LogLevel::Warn)
            .count()
    }
}

impl EventEmitter<PanelEvent> for MessagesPanel {}

impl Focusable for MessagesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for MessagesPanel {
    fn panel_name(&self) -> &'static str {
        "MessagesPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for MessagesPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Messages"
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MessagesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let info = cx.theme().green;
        let warning = cx.theme().warning;
        let danger = cx.theme().danger;
        let entries = self.entries.clone();
        let count = entries.len();
        v_flex()
            .id("messages-panel")
            .size_full()
            .text_xs()
            .child(column_row(
                "messages-header",
                "level",
                "topic",
                "message",
                muted,
                muted,
                muted,
            ))
            .child(
                uniform_list("messages-rows", count, {
                    move |range, _, _cx| {
                        range
                            .map(|ix| {
                                let entry = &entries[ix];
                                let level_color = match entry.level {
                                    LogLevel::Info => info,
                                    LogLevel::Warn => warning,
                                    LogLevel::Error => danger,
                                };
                                column_row(
                                    ("messages-row", ix as u64),
                                    entry.level.as_str(),
                                    &entry.topic,
                                    &entry.text,
                                    level_color,
                                    muted,
                                    foreground,
                                )
                            })
                            .collect()
                    }
                })
                .track_scroll(&self.scroll)
                .flex_1()
                .size_full(),
            )
    }
}

fn column_row(
    id: impl Into<ElementId>,
    level: impl Into<SharedString>,
    topic: impl Into<SharedString>,
    message: impl Into<SharedString>,
    level_color: gpui_kit::Hsla,
    topic_color: gpui_kit::Hsla,
    message_color: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .flex_none()
        .items_center()
        .px_1p5()
        .py_0p5()
        .child(
            div()
                .w(LEVEL_WIDTH)
                .flex_none()
                .text_color(level_color)
                .child(level.into()),
        )
        .child(
            div()
                .w(TOPIC_WIDTH)
                .flex_none()
                .min_w_0()
                .overflow_hidden()
                .text_color(topic_color)
                .child(topic.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(message_color)
                .child(message.into()),
        )
        .into_any_element()
}
