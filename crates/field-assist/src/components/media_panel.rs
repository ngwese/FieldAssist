// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Bottom-dock table of shared media-pool entries.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex, ActiveTheme as _, IconName, Sizable as _,
};
use gpui_kit::{
    canvas, div, fill, point, prelude::FluentBuilder as _, px, size, uniform_list, App,
    AppContext as _, Bounds, Context, DispatchPhase, DragMoveEvent, ElementId, Empty, Entity,
    EntityId, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window,
};

use crate::media_pool::MediaPoolRow;

const MIN_COLUMN_WIDTH: f32 = 32.;
const ELLIPSIS_WIDTH: f32 = 28.;
const RESIZE_HANDLE_WIDTH: f32 = 5.;
/// Matches `.px_1p5()` on header/row (1.5 × 4px).
const ROW_PAD_X: f32 = 6.;
/// Waveform uses 14px; Media bar is half that.
const H_SCROLLBAR_HEIGHT: f32 = 7.;
const MIN_H_THUMB: f32 = 24.;

/// Identifies a media-pool table column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MediaColumn {
    Row,
    Id,
    Basename,
    Path,
    Url,
    SampleRate,
    Channels,
    Frames,
    BitDepth,
    Size,
    Modified,
    Container,
    Codec,
}

impl MediaColumn {
    const ALL: [Self; 13] = [
        Self::Row,
        Self::Id,
        Self::Basename,
        Self::Path,
        Self::Url,
        Self::SampleRate,
        Self::Channels,
        Self::Frames,
        Self::BitDepth,
        Self::Size,
        Self::Modified,
        Self::Container,
        Self::Codec,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Row => "#",
            Self::Id => "id",
            Self::Basename => "basename",
            Self::Path => "path",
            Self::Url => "url",
            Self::SampleRate => "rate",
            Self::Channels => "ch",
            Self::Frames => "frames",
            Self::BitDepth => "bits",
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Container => "container",
            Self::Codec => "codec",
        }
    }

    fn menu_label(self) -> &'static str {
        match self {
            Self::Row => "Row",
            Self::Id => "Id",
            Self::Basename => "Basename",
            Self::Path => "Path",
            Self::Url => "Url",
            Self::SampleRate => "Sample Rate",
            Self::Channels => "Channels",
            Self::Frames => "Frames",
            Self::BitDepth => "Bit Depth",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Container => "Container",
            Self::Codec => "Codec",
        }
    }

    fn default_width(self) -> Pixels {
        match self {
            Self::Row => px(36.),
            Self::Id => px(120.),
            Self::Basename => px(140.),
            Self::Path => px(280.),
            Self::Url => px(160.),
            Self::SampleRate => px(64.),
            Self::Channels => px(40.),
            Self::Frames => px(80.),
            Self::BitDepth => px(48.),
            Self::Size => px(72.),
            Self::Modified => px(150.),
            Self::Container => px(72.),
            Self::Codec => px(72.),
        }
    }

    fn stable_id(self) -> u64 {
        match self {
            Self::Row => 0,
            Self::Id => 1,
            Self::Basename => 2,
            Self::Path => 3,
            Self::Url => 4,
            Self::SampleRate => 5,
            Self::Channels => 6,
            Self::Frames => 7,
            Self::BitDepth => 8,
            Self::Size => 9,
            Self::Modified => 10,
            Self::Container => 11,
            Self::Codec => 12,
        }
    }
}

fn default_column_order() -> Vec<MediaColumn> {
    MediaColumn::ALL.to_vec()
}

fn default_column_widths() -> HashMap<MediaColumn, Pixels> {
    MediaColumn::ALL
        .into_iter()
        .map(|col| (col, col.default_width()))
        .collect()
}

fn default_hidden_columns() -> HashSet<MediaColumn> {
    HashSet::from([MediaColumn::Id])
}

fn move_column(order: &mut Vec<MediaColumn>, from: MediaColumn, to: MediaColumn) {
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

/// Bottom-dock Media pool table.
pub struct MediaPoolPanel {
    rows: Vec<MediaPoolRow>,
    fingerprint: u64,
    column_order: Vec<MediaColumn>,
    column_widths: HashMap<MediaColumn, Pixels>,
    hidden: HashSet<MediaColumn>,
    list_scroll: UniformListScrollHandle,
    /// Horizontal pan offset in pixels (content scrolled left by this amount).
    h_offset: f32,
    viewport_width: f32,
    scrollbar_origin_x: f32,
    scrollbar_width: f32,
    scrollbar_drag: Option<f32>,
    resize_drag: Option<ResizeDrag>,
    focus_handle: FocusHandle,
}

/// Tracks an in-progress column resize (origin cursor x + starting width).
struct ResizeDrag {
    column: MediaColumn,
    start_x: f32,
    start_width: Pixels,
}

impl MediaPoolPanel {
    /// Create an empty Media pool panel.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            rows: Vec::new(),
            fingerprint: 0,
            column_order: default_column_order(),
            column_widths: default_column_widths(),
            hidden: default_hidden_columns(),
            list_scroll: UniformListScrollHandle::default(),
            h_offset: 0.0,
            viewport_width: 0.0,
            scrollbar_origin_x: 0.0,
            scrollbar_width: 0.0,
            scrollbar_drag: None,
            resize_drag: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Replace rows when the pool fingerprint changes.
    pub fn set_rows(&mut self, rows: Vec<MediaPoolRow>, cx: &mut Context<Self>) {
        let fingerprint = rows_fingerprint(&rows);
        if fingerprint == self.fingerprint {
            return;
        }
        self.fingerprint = fingerprint;
        self.rows = rows;
        cx.notify();
    }

    fn cell_text(row: &MediaPoolRow, row_ix: usize, column: MediaColumn) -> String {
        match column {
            MediaColumn::Row => (row_ix + 1).to_string(),
            MediaColumn::Id => row.id.to_string(),
            MediaColumn::Basename => row.basename.clone(),
            MediaColumn::Path => row.path.display().to_string(),
            MediaColumn::Url => row.url.clone(),
            MediaColumn::SampleRate => row.sample_rate.to_string(),
            MediaColumn::Channels => row.channel_count.to_string(),
            MediaColumn::Frames => row.frame_count.to_string(),
            MediaColumn::BitDepth => row
                .bits_per_sample
                .map(|bits| bits.to_string())
                .unwrap_or_default(),
            MediaColumn::Size => format_size_bytes(row.size_bytes),
            MediaColumn::Modified => row.modified_text.clone(),
            MediaColumn::Container => row.container_format.clone(),
            MediaColumn::Codec => row.codec.clone(),
        }
    }

    fn visible_columns(&self) -> Vec<MediaColumn> {
        self.column_order
            .iter()
            .copied()
            .filter(|col| !self.hidden.contains(col))
            .collect()
    }

    fn column_width(&self, col: MediaColumn) -> Pixels {
        self.column_widths
            .get(&col)
            .copied()
            .unwrap_or_else(|| col.default_width())
    }

    /// Total width of visible columns + resize gutters + row padding.
    fn content_width(&self) -> Pixels {
        let cols: f32 = self
            .visible_columns()
            .into_iter()
            .map(|col| f32::from(self.column_width(col)) + RESIZE_HANDLE_WIDTH)
            .sum();
        px(cols + ROW_PAD_X * 2.)
    }

    fn content_width_f32(&self) -> f32 {
        f32::from(self.content_width())
    }

    fn max_h_offset(&self) -> f32 {
        (self.content_width_f32() - self.viewport_width).max(0.0)
    }

    fn needs_h_scroll(&self) -> bool {
        self.viewport_width > 1.0 && self.content_width_f32() > self.viewport_width + 0.5
    }

    fn clamp_h_offset(&mut self) {
        let max = self.max_h_offset();
        self.h_offset = self.h_offset.clamp(0.0, max);
    }

    fn pan_horizontal(&mut self, dx: f32, cx: &mut Context<Self>) {
        self.h_offset = (self.h_offset + dx).clamp(0.0, self.max_h_offset());
        cx.notify();
    }

    fn remember_viewport_width(&mut self, width: f32, cx: &mut Context<Self>) {
        if width <= 1.0 {
            return;
        }
        if (self.viewport_width - width).abs() <= 0.5 {
            return;
        }
        self.viewport_width = width;
        self.clamp_h_offset();
        cx.notify();
    }

    fn remember_scrollbar(&mut self, bounds: Bounds<Pixels>) {
        self.scrollbar_width = bounds.size.width.as_f32();
        self.scrollbar_origin_x = bounds.origin.x.as_f32();
    }

    fn set_h_offset_from_scrollbar_x(&mut self, x: f32, grab_offset: f32) {
        let track = self.scrollbar_width.max(1.0);
        let content = self.content_width_f32();
        let viewport = self.viewport_width.max(1.0);
        let (thumb_w, _) = h_scroll_geom(content, viewport, self.h_offset, track);
        let max_travel = (track - thumb_w).max(0.0);
        let thumb_x = (x - self.scrollbar_origin_x - grab_offset).clamp(0.0, max_travel);
        let max_offset = self.max_h_offset();
        self.h_offset = if max_travel <= f32::EPSILON {
            0.0
        } else {
            (thumb_x / max_travel) * max_offset
        };
        self.clamp_h_offset();
    }

    fn set_column_width(&mut self, col: MediaColumn, width: Pixels) {
        let clamped = width.max(px(MIN_COLUMN_WIDTH));
        self.column_widths.insert(col, clamped);
        self.clamp_h_offset();
    }

    fn toggle_column_visible(&mut self, col: MediaColumn, cx: &mut Context<Self>) {
        if self.hidden.contains(&col) {
            self.hidden.remove(&col);
        } else {
            self.hidden.insert(col);
        }
        self.clamp_h_offset();
        cx.notify();
    }

    fn move_column_to(&mut self, from: MediaColumn, to: MediaColumn, cx: &mut Context<Self>) {
        move_column(&mut self.column_order, from, to);
        cx.notify();
    }

    fn begin_resize(&mut self, column: MediaColumn, start_x: Pixels, _cx: &mut Context<Self>) {
        self.resize_drag = Some(ResizeDrag {
            column,
            start_x: f32::from(start_x),
            start_width: self.column_width(column),
        });
    }

    fn apply_resize_drag(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.resize_drag.as_ref() else {
            return;
        };
        let column = drag.column;
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
        let content_width = self.content_width();

        h_flex()
            .id("media-pool-header")
            .w(content_width)
            .flex_none()
            .items_center()
            .px_1p5()
            .py_0p5()
            .children(visible.iter().enumerate().flat_map(|(ix, col)| {
                let col = *col;
                let width = self.column_width(col);
                let label = col.label();
                let mut cells = Vec::new();
                cells.push(
                    div()
                        .id(("media-th", ix as u64))
                        .w(width)
                        .flex_none()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .cursor_grab()
                        .on_drag(
                            DragMediaColumn {
                                entity_id,
                                column: col,
                                name: SharedString::from(col.menu_label()),
                            },
                            |drag, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| drag.clone())
                            },
                        )
                        .on_drop(cx.listener(move |this, drag: &DragMediaColumn, _, cx| {
                            if drag.entity_id != cx.entity_id() {
                                return;
                            }
                            this.move_column_to(drag.column, col, cx);
                        }))
                        .child(label)
                        .into_any_element(),
                );
                cells.push(resize_handle(col, cx).into_any_element());
                cells
            }))
    }
}

fn resize_handle(column: MediaColumn, cx: &mut Context<MediaPoolPanel>) -> impl IntoElement {
    h_flex()
        .id(("media-resize", column.stable_id()))
        .w(px(RESIZE_HANDLE_WIDTH))
        .flex_none()
        .self_stretch()
        .occlude()
        .cursor_col_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                this.begin_resize(column, e.position.x, cx);
                cx.notify();
            }),
        )
}

fn install_resize_listeners(entity: Entity<MediaPoolPanel>, window: &mut Window) {
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            entity.update(cx, |this, cx| {
                if this.resize_drag.is_some() {
                    this.apply_resize_drag(event.position.x, cx);
                }
            });
        }
    });
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            entity.update(cx, |this, cx| {
                if this.resize_drag.is_some() {
                    this.end_resize(cx);
                    cx.notify();
                }
            });
        }
    });
}

fn column_menu_button(
    hidden: HashSet<MediaColumn>,
    muted: gpui_kit::Hsla,
    cx: &mut Context<MediaPoolPanel>,
) -> impl IntoElement {
    let view = cx.entity().clone();
    Button::new("media-columns")
        .ghost()
        .xsmall()
        .w(px(ELLIPSIS_WIDTH))
        .p_0()
        .icon(IconName::Ellipsis)
        .tooltip("Columns")
        .dropdown_menu(move |mut menu: PopupMenu, _, _| {
            for col in MediaColumn::ALL {
                let checked = !hidden.contains(&col);
                let label = col.menu_label();
                let view = view.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().text_xs().text_color(muted).child(label)
                    })
                    .checked(checked)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.toggle_column_visible(col, cx);
                        });
                    }),
                );
            }
            menu
        })
}

#[derive(Clone)]
struct DragMediaColumn {
    entity_id: EntityId,
    column: MediaColumn,
    name: SharedString,
}

impl Render for DragMediaColumn {
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

fn rows_fingerprint(rows: &[MediaPoolRow]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    rows.len().hash(&mut hasher);
    for row in rows {
        row.id.to_hex().hash(&mut hasher);
        row.path.hash(&mut hasher);
        system_time_secs(row.modified).hash(&mut hasher);
        row.url.hash(&mut hasher);
        row.size_bytes.hash(&mut hasher);
    }
    hasher.finish()
}

fn system_time_secs(time: SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_size_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn render_row_static(
    id: impl Into<ElementId>,
    row: &MediaPoolRow,
    row_ix: usize,
    order: &[MediaColumn],
    widths: &HashMap<MediaColumn, Pixels>,
    hidden: &HashSet<MediaColumn>,
    content_width: Pixels,
    muted: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    let visible: Vec<_> = order
        .iter()
        .copied()
        .filter(|col| !hidden.contains(col))
        .collect();
    h_flex()
        .id(id)
        .w(content_width)
        .flex_none()
        .items_center()
        .px_1p5()
        .py_0p5()
        .children(visible.iter().flat_map(|col| {
            let col = *col;
            let width = widths
                .get(&col)
                .copied()
                .unwrap_or_else(|| col.default_width());
            let text = MediaPoolPanel::cell_text(row, row_ix, col);
            let tooltip_text = text.clone();
            let mut cells = Vec::new();
            cells.push(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "media-td-{row_ix}-{}",
                        col.stable_id()
                    ))))
                    .w(width)
                    .flex_none()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_xs()
                    .when(col == MediaColumn::Row, |el| el.text_color(muted))
                    .when(!tooltip_text.is_empty(), |el| {
                        el.tooltip(move |window, cx| {
                            Tooltip::new(tooltip_text.clone()).build(window, cx)
                        })
                    })
                    .child(text)
                    .into_any_element(),
            );
            cells.push(
                div()
                    .w(px(RESIZE_HANDLE_WIDTH))
                    .flex_none()
                    .into_any_element(),
            );
            cells
        }))
        .into_any_element()
}

impl EventEmitter<PanelEvent> for MediaPoolPanel {}
impl Focusable for MediaPoolPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl BasePanel for MediaPoolPanel {
    fn panel_name(&self) -> &'static str {
        "MediaPoolPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for MediaPoolPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::dock_titles::BOTTOM_TAB_MEDIA
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MediaPoolPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.clamp_h_offset();
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let track = cx.theme().scrollbar;
        let thumb = cx.theme().scrollbar_thumb;
        let hidden = self.hidden.clone();
        let content_width = self.content_width();
        let content_w = self.content_width_f32();
        let h_offset = self.h_offset;
        let viewport_w = self.viewport_width;
        let show_h_scroll = self.needs_h_scroll();
        let header = self.render_header(cx);
        let menu = column_menu_button(hidden.clone(), muted, cx);
        let order = self.column_order.clone();
        let widths = self.column_widths.clone();
        let rows = self.rows.clone();
        let count = rows.len();
        let entity = cx.entity().clone();
        let entity_id = cx.entity_id();

        let body = if count == 0 {
            div()
                .id("media-pool-empty")
                .size_full()
                .w(content_width)
                .p_2()
                .text_xs()
                .text_color(muted)
                .child("(no media in pool)")
                .into_any_element()
        } else {
            uniform_list("media-pool-rows", count, {
                move |range, _, _cx| {
                    range
                        .map(|ix| {
                            let row = &rows[ix];
                            render_row_static(
                                ElementId::Name(SharedString::from(format!(
                                    "media-row-{}",
                                    row.id.to_hex()
                                ))),
                                row,
                                ix,
                                &order,
                                &widths,
                                &hidden,
                                content_width,
                                muted,
                            )
                        })
                        .collect()
                }
            })
            .track_scroll(&self.list_scroll)
            .size_full()
            .w(content_width)
            .into_any_element()
        };

        v_flex()
            .id("media-pool-panel")
            .size_full()
            .track_focus(&self.focus_handle)
            .child(
                h_flex()
                    .id("media-pool-header-bar")
                    .w_full()
                    .flex_none()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .id("media-pool-header-clip")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("media-pool-header-pan")
                                    .w(content_width)
                                    .ml(px(-h_offset))
                                    .child(header),
                            ),
                    )
                    .child(
                        h_flex()
                            .id("media-pool-chrome-header")
                            .w(px(ELLIPSIS_WIDTH))
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .child(menu),
                    ),
            )
            .child(
                h_flex()
                    .id("media-pool-main")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .id("media-pool-viewport")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .relative()
                            .on_scroll_wheel(cx.listener(
                                move |this, event: &ScrollWheelEvent, _, cx| {
                                    if !this.needs_h_scroll() {
                                        return;
                                    }
                                    let delta = event.delta.pixel_delta(px(16.));
                                    let dx = delta.x.as_f32();
                                    let dy = delta.y.as_f32();
                                    let horizontal = event.modifiers.shift || dx.abs() > dy.abs();
                                    if !horizontal {
                                        return;
                                    }
                                    let pan = if event.modifiers.shift {
                                        if dx.abs() > dy.abs() {
                                            dx
                                        } else {
                                            dy
                                        }
                                    } else {
                                        dx
                                    };
                                    // Match typical scroll direction: positive delta pans content left.
                                    this.pan_horizontal(-pan, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .child(
                                canvas(
                                    {
                                        let entity = entity.clone();
                                        move |bounds, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.remember_viewport_width(
                                                    bounds.size.width.as_f32(),
                                                    cx,
                                                );
                                            });
                                        }
                                    },
                                    {
                                        let entity = entity.clone();
                                        move |_bounds, _, window, _cx| {
                                            install_resize_listeners(entity.clone(), window);
                                        }
                                    },
                                )
                                .absolute()
                                .size_full(),
                            )
                            .child(
                                div()
                                    .id("media-pool-body")
                                    .absolute()
                                    .top_0()
                                    .left(px(-h_offset))
                                    .h_full()
                                    .w(content_width)
                                    .min_w(content_width)
                                    .child(body),
                            ),
                    )
                    .child(
                        div()
                            .id("media-pool-chrome-body")
                            .flex_none()
                            .h_full()
                            .w(px(ELLIPSIS_WIDTH)),
                    ),
            )
            .when(show_h_scroll, |this| {
                this.child(render_h_scrollbar(
                    entity, entity_id, content_w, viewport_w, h_offset, track, thumb, border, cx,
                ))
            })
    }
}

fn h_scroll_geom(content: f32, viewport: f32, offset: f32, track: f32) -> (f32, f32) {
    if content <= 0.0 || viewport <= 0.0 || track <= 0.0 || content <= viewport {
        return (track, 0.0);
    }
    let ratio = (viewport / content).clamp(0.0, 1.0);
    let thumb_w = (track * ratio).max(MIN_H_THUMB).min(track);
    let max_offset = (content - viewport).max(0.0);
    let thumb_x = if max_offset <= f32::EPSILON {
        0.0
    } else {
        (offset / max_offset) * (track - thumb_w)
    };
    (thumb_w, thumb_x)
}

fn paint_h_scrollbar(
    bounds: Bounds<Pixels>,
    content: f32,
    viewport: f32,
    offset: f32,
    track: gpui_kit::Hsla,
    thumb: gpui_kit::Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(bounds, track));
    let (thumb_w, thumb_x) = h_scroll_geom(content, viewport, offset, bounds.size.width.as_f32());
    window.paint_quad(fill(
        Bounds {
            origin: point(px(bounds.origin.x.as_f32() + thumb_x), bounds.origin.y),
            size: size(px(thumb_w), bounds.size.height),
        },
        thumb,
    ));
}

fn render_h_scrollbar(
    entity: Entity<MediaPoolPanel>,
    entity_id: EntityId,
    content: f32,
    viewport: f32,
    offset: f32,
    track: gpui_kit::Hsla,
    thumb: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    cx: &mut Context<MediaPoolPanel>,
) -> impl IntoElement {
    div()
        .id("media-pool-h-scroll")
        .w_full()
        .h(px(H_SCROLLBAR_HEIGHT))
        .flex_none()
        .border_t_1()
        .border_color(border)
        .child(
            div()
                .id("media-pool-h-scroll-track")
                .size_full()
                .child(
                    canvas(
                        {
                            let entity = entity.clone();
                            move |bounds, _, cx| {
                                entity.update(cx, |this, _cx| {
                                    this.remember_scrollbar(bounds);
                                });
                                bounds
                            }
                        },
                        move |bounds, _, window, _cx| {
                            paint_h_scrollbar(
                                bounds, content, viewport, offset, track, thumb, window,
                            );
                        },
                    )
                    .size_full(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        let x = event.position.x.as_f32();
                        let track_w = this.scrollbar_width.max(1.0);
                        let (thumb_w, thumb_x) = h_scroll_geom(
                            this.content_width_f32(),
                            this.viewport_width,
                            this.h_offset,
                            track_w,
                        );
                        let local = x - this.scrollbar_origin_x;
                        let grab_offset = if local >= thumb_x && local <= thumb_x + thumb_w {
                            local - thumb_x
                        } else {
                            thumb_w * 0.5
                        };
                        this.scrollbar_drag = Some(grab_offset);
                        this.set_h_offset_from_scrollbar_x(x, grab_offset);
                        cx.notify();
                    }),
                )
                .on_drag(DragHScroll { entity_id }, |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                })
                .on_drag_move(
                    cx.listener(move |this, e: &DragMoveEvent<DragHScroll>, _, cx| {
                        let drag = e.drag(cx);
                        if drag.entity_id != cx.entity_id() {
                            return;
                        }
                        let Some(grab_offset) = this.scrollbar_drag else {
                            return;
                        };
                        this.set_h_offset_from_scrollbar_x(
                            e.event.position.x.as_f32(),
                            grab_offset,
                        );
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.scrollbar_drag = None;
                        cx.notify();
                    }),
                ),
        )
}

#[derive(Clone, PartialEq, Eq)]
struct DragHScroll {
    entity_id: EntityId,
}

impl Render for DragHScroll {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_column_reorders() {
        let mut order = default_column_order();
        move_column(&mut order, MediaColumn::Basename, MediaColumn::Url);
        assert_eq!(
            order,
            vec![
                MediaColumn::Row,
                MediaColumn::Id,
                MediaColumn::Path,
                MediaColumn::Url,
                MediaColumn::Basename,
                MediaColumn::SampleRate,
                MediaColumn::Channels,
                MediaColumn::Frames,
                MediaColumn::BitDepth,
                MediaColumn::Size,
                MediaColumn::Modified,
                MediaColumn::Container,
                MediaColumn::Codec,
            ]
        );
    }

    #[test]
    fn id_hidden_by_default() {
        let hidden = default_hidden_columns();
        assert!(hidden.contains(&MediaColumn::Id));
        assert!(!hidden.contains(&MediaColumn::Row));
        assert!(!hidden.contains(&MediaColumn::Path));
    }

    #[test]
    fn move_column_noop_when_same() {
        let mut order = default_column_order();
        move_column(&mut order, MediaColumn::Row, MediaColumn::Row);
        assert_eq!(order, default_column_order());
    }

    #[test]
    fn h_scroll_geom_scales_thumb() {
        let (thumb_w, thumb_x) = h_scroll_geom(1000.0, 250.0, 0.0, 400.0);
        assert!((thumb_w - 100.0).abs() < 0.01);
        assert!((thumb_x - 0.0).abs() < 0.01);

        let (thumb_w, thumb_x) = h_scroll_geom(1000.0, 250.0, 750.0, 400.0);
        assert!((thumb_w - 100.0).abs() < 0.01);
        assert!((thumb_x - 300.0).abs() < 0.01);
    }

    #[test]
    fn h_scroll_geom_full_when_content_fits() {
        let (thumb_w, thumb_x) = h_scroll_geom(200.0, 400.0, 0.0, 300.0);
        assert!((thumb_w - 300.0).abs() < 0.01);
        assert!((thumb_x - 0.0).abs() < 0.01);
    }
}
