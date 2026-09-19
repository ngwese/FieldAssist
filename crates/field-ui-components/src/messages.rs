// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Log / messages panel with a gpui-free [`LogLine`] DTO.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex, ActiveTheme as _, IconName, Sizable as _,
};
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, uniform_list, App, AppContext as _, Context,
    DragMoveEvent, ElementId, Empty, EntityId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, UniformListScrollHandle, Window,
};

const MIN_COLUMN_WIDTH: f32 = 32.;
const ELLIPSIS_WIDTH: f32 = 28.;
const RESIZE_HANDLE_WIDTH: f32 = 5.;

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
    /// Seconds since the messages panel was created (stamped on append).
    pub secs: f64,
    /// Severity.
    pub level: LogLevel,
    /// Topic / subsystem label.
    pub topic: String,
    /// Message body.
    pub text: String,
}

impl LogLine {
    /// Build a log line; [`MessagesPanel::append`] overwrites [`Self::secs`].
    pub fn new(level: LogLevel, topic: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            secs: 0.0,
            level,
            topic: topic.into(),
            text: text.into(),
        }
    }
}

/// Identifies a messages-table column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MessageColumn {
    Time,
    Level,
    Topic,
    Message,
}

impl MessageColumn {
    const ALL: [Self; 4] = [Self::Time, Self::Level, Self::Topic, Self::Message];

    fn label(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Level => "level",
            Self::Topic => "topic",
            Self::Message => "message",
        }
    }

    fn menu_label(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Level => "Level",
            Self::Topic => "Topic",
            Self::Message => "Message",
        }
    }

    fn default_width(self) -> Pixels {
        match self {
            Self::Time => px(64.),
            Self::Level => px(48.),
            Self::Topic => px(72.),
            Self::Message => px(200.),
        }
    }

    fn is_flex(self) -> bool {
        matches!(self, Self::Message)
    }

    fn stable_id(self) -> u64 {
        match self {
            Self::Time => 0,
            Self::Level => 1,
            Self::Topic => 2,
            Self::Message => 3,
        }
    }
}

fn default_column_order() -> Vec<MessageColumn> {
    MessageColumn::ALL.to_vec()
}

fn default_column_widths() -> HashMap<MessageColumn, Pixels> {
    MessageColumn::ALL
        .into_iter()
        .filter(|col| !col.is_flex())
        .map(|col| (col, col.default_width()))
        .collect()
}

fn move_column(order: &mut Vec<MessageColumn>, from: MessageColumn, to: MessageColumn) {
    if from == to {
        return;
    }
    let Some(from_ix) = order.iter().position(|c| *c == from) else {
        return;
    };
    let Some(to_ix) = order.iter().position(|c| *c == to) else {
        return;
    };
    let col = order.remove(from_ix);
    let insert_at = if from_ix < to_ix { to_ix } else { to_ix };
    order.insert(insert_at.min(order.len()), col);
}

fn format_secs(secs: f64) -> String {
    format!("{secs:.3}")
}

/// Bottom-dock messages panel.
pub struct MessagesPanel {
    entries: Vec<LogLine>,
    /// Index of the first log line that still counts toward status-bar alerts.
    acked_until: usize,
    started: Instant,
    column_order: Vec<MessageColumn>,
    column_widths: HashMap<MessageColumn, Pixels>,
    hidden: HashSet<MessageColumn>,
    wrap_messages: bool,
    list_scroll: UniformListScrollHandle,
    wrap_scroll: ScrollHandle,
    resize_drag: Option<ResizeDrag>,
    focus_handle: FocusHandle,
}

/// Tracks an in-progress column resize (origin cursor x + starting width).
struct ResizeDrag {
    column: MessageColumn,
    start_x: f32,
    start_width: Pixels,
}

impl MessagesPanel {
    /// Create an empty messages panel.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            acked_until: 0,
            started: Instant::now(),
            column_order: default_column_order(),
            column_widths: default_column_widths(),
            hidden: HashSet::new(),
            wrap_messages: true,
            list_scroll: UniformListScrollHandle::new(),
            wrap_scroll: ScrollHandle::new(),
            resize_drag: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Append log lines.
    pub fn append(&mut self, entries: Vec<LogLine>, cx: &mut Context<Self>) {
        if entries.is_empty() {
            return;
        }
        let secs = self.started.elapsed().as_secs_f64();
        self.entries.extend(entries.into_iter().map(|mut entry| {
            entry.secs = secs;
            entry
        }));
        self.scroll_to_bottom();
        cx.notify();
    }

    /// Whether long message text wraps within the pane width.
    pub fn wrap_messages(&self) -> bool {
        self.wrap_messages
    }

    /// Set whether long message text wraps within the pane width.
    pub fn set_wrap_messages(&mut self, wrap: bool, cx: &mut Context<Self>) {
        if self.wrap_messages == wrap {
            return;
        }
        self.wrap_messages = wrap;
        self.scroll_to_bottom();
        cx.notify();
    }

    /// Toggle message wrapping; returns the new value.
    pub fn toggle_wrap_messages(&mut self, cx: &mut Context<Self>) -> bool {
        self.set_wrap_messages(!self.wrap_messages, cx);
        self.wrap_messages
    }

    /// Number of unacked error lines (status-bar counter).
    pub fn error_count(&self) -> usize {
        count_level(&self.entries, self.acked_until, LogLevel::Error)
    }

    /// Number of unacked warning lines (status-bar counter).
    pub fn warn_count(&self) -> usize {
        count_level(&self.entries, self.acked_until, LogLevel::Warn)
    }

    /// Reset status-bar error/warn counters without removing log lines.
    pub fn clear_counts(&mut self, cx: &mut Context<Self>) {
        self.acked_until = self.entries.len();
        cx.notify();
    }

    fn scroll_to_bottom(&mut self) {
        if self.wrap_messages {
            self.wrap_scroll.scroll_to_bottom();
        } else {
            self.list_scroll.scroll_to_bottom();
        }
    }

    fn visible_columns(&self) -> Vec<MessageColumn> {
        self.column_order
            .iter()
            .copied()
            .filter(|col| !self.hidden.contains(col))
            .collect()
    }

    fn column_width(&self, col: MessageColumn) -> Pixels {
        self.column_widths
            .get(&col)
            .copied()
            .unwrap_or_else(|| col.default_width())
    }

    fn set_column_width(&mut self, col: MessageColumn, width: Pixels) {
        if col.is_flex() {
            return;
        }
        let clamped = width.max(px(MIN_COLUMN_WIDTH));
        self.column_widths.insert(col, clamped);
    }

    fn toggle_column_visible(&mut self, col: MessageColumn, cx: &mut Context<Self>) {
        if col == MessageColumn::Message {
            return;
        }
        if self.hidden.contains(&col) {
            self.hidden.remove(&col);
        } else {
            // Keep at least one non-message column? Allow hiding all fixed cols.
            self.hidden.insert(col);
        }
        cx.notify();
    }

    fn move_column_to(&mut self, from: MessageColumn, to: MessageColumn, cx: &mut Context<Self>) {
        move_column(&mut self.column_order, from, to);
        cx.notify();
    }

    fn begin_resize(&mut self, column: MessageColumn, start_x: Pixels, _cx: &mut Context<Self>) {
        if column.is_flex() {
            return;
        }
        self.resize_drag = Some(ResizeDrag {
            column,
            start_x: f32::from(start_x),
            start_width: self.column_width(column),
        });
    }

    fn apply_resize_drag(&mut self, column: MessageColumn, x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.resize_drag.as_ref() else {
            return;
        };
        if drag.column != column {
            return;
        }
        let delta = f32::from(x) - drag.start_x;
        let new_width = px(f32::from(drag.start_width) + delta);
        self.set_column_width(column, new_width);
        cx.notify();
    }

    fn end_resize(&mut self, _cx: &mut Context<Self>) {
        self.resize_drag = None;
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let entity_id = cx.entity_id();
        let visible = self.visible_columns();
        let hidden = self.hidden.clone();

        h_flex()
            .id("messages-header")
            .w_full()
            .flex_none()
            .items_center()
            .px_1p5()
            .py_0p5()
            .child(h_flex().flex_1().min_w_0().items_center().children(
                visible.iter().enumerate().flat_map(|(ix, col)| {
                    let col = *col;
                    let width = self.column_width(col);
                    let label = col.label();
                    let mut cells = Vec::new();
                    cells.push(
                        div()
                            .id(("messages-th", ix as u64))
                            .when(col.is_flex(), |el| el.flex_1().min_w_0())
                            .when(!col.is_flex(), |el| el.w(width).flex_none())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(muted)
                            .cursor_grab()
                            .on_drag(
                                DragMessageColumn {
                                    entity_id,
                                    column: col,
                                    name: SharedString::from(col.menu_label()),
                                },
                                |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                },
                            )
                            .on_drop(cx.listener(move |this, drag: &DragMessageColumn, _, cx| {
                                if drag.entity_id != cx.entity_id() {
                                    return;
                                }
                                this.move_column_to(drag.column, col, cx);
                            }))
                            .child(label)
                            .into_any_element(),
                    );
                    if !col.is_flex() {
                        cells.push(resize_handle(entity_id, col, cx).into_any_element());
                    }
                    cells
                }),
            ))
            .child(column_menu_button(hidden, self.wrap_messages, muted, cx))
    }
}

fn resize_handle(
    entity_id: EntityId,
    column: MessageColumn,
    cx: &mut Context<MessagesPanel>,
) -> impl IntoElement {
    h_flex()
        .id(("messages-resize", column.stable_id()))
        .w(px(RESIZE_HANDLE_WIDTH))
        .flex_none()
        .h_full()
        .occlude()
        .cursor_col_resize()
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(move |this, e: &gpui_kit::MouseDownEvent, _, cx| {
                this.begin_resize(column, e.position.x, cx);
            }),
        )
        .on_drag_move(
            cx.listener(move |this, e: &DragMoveEvent<ResizeMessageColumn>, _, cx| {
                let drag = e.drag(cx);
                if drag.entity_id != cx.entity_id() || drag.column != column {
                    return;
                }
                this.apply_resize_drag(column, e.event.position.x, cx);
            }),
        )
        .on_drag(
            ResizeMessageColumn { entity_id, column },
            |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            },
        )
        .on_mouse_up(
            gpui_kit::MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.end_resize(cx);
            }),
        )
}

fn column_menu_button(
    hidden: HashSet<MessageColumn>,
    wrap_messages: bool,
    muted: gpui_kit::Hsla,
    cx: &mut Context<MessagesPanel>,
) -> impl IntoElement {
    let view = cx.entity().clone();
    Button::new("messages-columns")
        .ghost()
        .xsmall()
        .w(px(ELLIPSIS_WIDTH))
        .p_0()
        .icon(IconName::Ellipsis)
        .tooltip("Columns")
        .dropdown_menu(move |mut menu: PopupMenu, _, _| {
            for col in MessageColumn::ALL {
                let checked = !hidden.contains(&col);
                let label = col.menu_label();
                let view = view.clone();
                let disabled = col == MessageColumn::Message;
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().text_xs().text_color(muted).child(label)
                    })
                    .checked(checked)
                    .disabled(disabled)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.toggle_column_visible(col, cx);
                        });
                    }),
                );
            }
            let view = view.clone();
            menu = menu.separator();
            menu = menu.item(
                PopupMenuItem::element(move |_, _| {
                    div().text_xs().text_color(muted).child("Wrap Messages")
                })
                .checked(wrap_messages)
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.toggle_wrap_messages(cx);
                    });
                }),
            );
            menu
        })
}

#[derive(Clone)]
struct DragMessageColumn {
    entity_id: EntityId,
    column: MessageColumn,
    name: SharedString,
}

impl Render for DragMessageColumn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .text_xs()
            .bg(cx.theme().background)
            .text_color(cx.theme().muted_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .opacity(0.9)
            .child(self.name.clone())
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ResizeMessageColumn {
    entity_id: EntityId,
    column: MessageColumn,
}

impl Render for ResizeMessageColumn {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn count_level(entries: &[LogLine], acked_until: usize, level: LogLevel) -> usize {
    entries
        .get(acked_until..)
        .unwrap_or(&[])
        .iter()
        .filter(|entry| entry.level == level)
        .count()
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
        let wrap = self.wrap_messages;
        let entries = self.entries.clone();
        let count = entries.len();
        let header = self.render_header(cx);

        // Snapshot layout for row closures.
        let order = self.column_order.clone();
        let widths = self.column_widths.clone();
        let hidden = self.hidden.clone();

        v_flex()
            .id("messages-panel")
            .size_full()
            .text_xs()
            .child(header)
            .child(if wrap {
                let order = order.clone();
                let widths = widths.clone();
                let hidden = hidden.clone();
                v_flex()
                    .id("messages-wrap-rows")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.wrap_scroll)
                    .children(entries.iter().enumerate().map(|(ix, entry)| {
                        render_row_static(
                            ("messages-row", ix as u64),
                            entry,
                            &order,
                            &widths,
                            &hidden,
                            info,
                            warning,
                            danger,
                            muted,
                            foreground,
                            true,
                        )
                    }))
                    .into_any_element()
            } else {
                let order = order.clone();
                let widths = widths.clone();
                let hidden = hidden.clone();
                uniform_list("messages-rows", count, {
                    move |range, _, _cx| {
                        range
                            .map(|ix| {
                                let entry = &entries[ix];
                                render_row_static(
                                    ("messages-row", ix as u64),
                                    entry,
                                    &order,
                                    &widths,
                                    &hidden,
                                    info,
                                    warning,
                                    danger,
                                    muted,
                                    foreground,
                                    false,
                                )
                            })
                            .collect()
                    }
                })
                .track_scroll(&self.list_scroll)
                .flex_1()
                .size_full()
                .into_any_element()
            })
    }
}

fn render_row_static(
    id: impl Into<ElementId>,
    entry: &LogLine,
    order: &[MessageColumn],
    widths: &HashMap<MessageColumn, Pixels>,
    hidden: &HashSet<MessageColumn>,
    info: gpui_kit::Hsla,
    warning: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    wrap: bool,
) -> gpui_kit::AnyElement {
    let visible: Vec<_> = order
        .iter()
        .copied()
        .filter(|col| !hidden.contains(col))
        .collect();
    let level_color = match entry.level {
        LogLevel::Info => info,
        LogLevel::Warn => warning,
        LogLevel::Error => danger,
    };
    let row = if wrap {
        h_flex().items_start()
    } else {
        h_flex().items_center()
    };
    row.id(id)
        .w_full()
        .flex_none()
        .px_1p5()
        .py_0p5()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .when(wrap, |el| el.items_start())
                .when(!wrap, |el| el.items_center())
                .children(visible.iter().flat_map(|col| {
                    let col = *col;
                    let width = widths
                        .get(&col)
                        .copied()
                        .unwrap_or_else(|| col.default_width());
                    let mut cells = Vec::new();
                    let (text, color): (SharedString, _) = match col {
                        MessageColumn::Time => (format_secs(entry.secs).into(), muted),
                        MessageColumn::Level => (entry.level.as_str().into(), level_color),
                        MessageColumn::Topic => (entry.topic.clone().into(), muted),
                        MessageColumn::Message => (entry.text.clone().into(), foreground),
                    };
                    let is_message = matches!(col, MessageColumn::Message);
                    cells.push(
                        div()
                            .when(col.is_flex(), |el| el.flex_1().min_w_0())
                            .when(!col.is_flex(), |el| el.w(width).flex_none())
                            .when(wrap && is_message, |el| el.min_w_0())
                            .when(!(wrap && is_message), |el| {
                                el.min_w_0().overflow_hidden().whitespace_nowrap()
                            })
                            .text_color(color)
                            .child(text)
                            .into_any_element(),
                    );
                    if !col.is_flex() {
                        cells.push(
                            div()
                                .w(px(RESIZE_HANDLE_WIDTH))
                                .flex_none()
                                .into_any_element(),
                        );
                    }
                    cells
                })),
        )
        .child(div().w(px(ELLIPSIS_WIDTH)).flex_none())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: LogLevel) -> LogLine {
        LogLine::new(level, "t", "m")
    }

    #[test]
    fn counts_unacked_errors_and_warns() {
        let entries = vec![
            line(LogLevel::Info),
            line(LogLevel::Warn),
            line(LogLevel::Error),
            line(LogLevel::Error),
        ];
        assert_eq!(count_level(&entries, 0, LogLevel::Error), 2);
        assert_eq!(count_level(&entries, 0, LogLevel::Warn), 1);
        assert_eq!(count_level(&entries, 3, LogLevel::Error), 1);
        assert_eq!(count_level(&entries, 3, LogLevel::Warn), 0);
        assert_eq!(count_level(&entries, 4, LogLevel::Error), 0);
        assert_eq!(count_level(&entries, 4, LogLevel::Warn), 0);
    }

    #[test]
    fn formats_seconds_to_three_decimals() {
        assert_eq!(format_secs(0.0), "0.000");
        assert_eq!(format_secs(12.3456), "12.346");
        assert_eq!(format_secs(1.2), "1.200");
    }

    #[test]
    fn move_column_reorders() {
        let mut order = default_column_order();
        move_column(&mut order, MessageColumn::Time, MessageColumn::Topic);
        assert_eq!(
            order,
            vec![
                MessageColumn::Level,
                MessageColumn::Topic,
                MessageColumn::Time,
                MessageColumn::Message,
            ]
        );
        move_column(&mut order, MessageColumn::Message, MessageColumn::Level);
        assert_eq!(
            order,
            vec![
                MessageColumn::Message,
                MessageColumn::Level,
                MessageColumn::Topic,
                MessageColumn::Time,
            ]
        );
    }

    #[test]
    fn move_column_noop_when_same() {
        let mut order = default_column_order();
        move_column(&mut order, MessageColumn::Level, MessageColumn::Level);
        assert_eq!(order, default_column_order());
    }
}
