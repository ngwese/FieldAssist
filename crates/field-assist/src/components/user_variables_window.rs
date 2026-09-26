// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Singleton "User Variables" window (Media-style table chrome).

use std::collections::{HashMap, HashSet};

use field_ui_components::{content_foreground, VariableRow};
use field_variables::{VariableEntry, VariableTable};
use gpui_kit::component::{
    h_flex,
    input::{IndentInline, Input, InputEvent, InputState, OutdentInline},
    v_flex, ActiveTheme as _, Root, Sizable as _, Theme,
};
use gpui_kit::{
    canvas, div, fill, point, prelude::FluentBuilder as _, px, size, uniform_list, App,
    AppContext as _, Bounds, Context, DispatchPhase, DragMoveEvent, ElementId, Empty, Entity,
    FocusHandle, Focusable, Global, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render,
    ScrollStrategy, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window,
};

use crate::components::variables_filter::{self, VariablesFilter};
use crate::user_variables::{self, UserVariablesFile};

const MIN_COLUMN_WIDTH: f32 = 32.;
const RESIZE_HANDLE_WIDTH: f32 = 5.;
const ROW_PAD_X: f32 = 6.;
const H_SCROLLBAR_HEIGHT: f32 = 7.;
const MIN_H_THUMB: f32 = 24.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TableColumn {
    Row,
    Name,
    Value,
    Description,
}

impl TableColumn {
    const ALL: [Self; 4] = [Self::Row, Self::Name, Self::Value, Self::Description];

    fn label(self) -> &'static str {
        match self {
            Self::Row => "#",
            Self::Name => "name",
            Self::Value => "value",
            Self::Description => "description",
        }
    }

    fn default_width(self) -> Pixels {
        match self {
            Self::Row => px(36.),
            Self::Name => px(160.),
            Self::Value => px(220.),
            Self::Description => px(200.),
        }
    }

    fn stable_id(self) -> u64 {
        match self {
            Self::Row => 0,
            Self::Name => 1,
            Self::Value => 2,
            Self::Description => 3,
        }
    }

    fn edit_column(self) -> Option<EditColumn> {
        match self {
            Self::Row => None,
            Self::Name => Some(EditColumn::Name),
            Self::Value => Some(EditColumn::Value),
            Self::Description => Some(EditColumn::Description),
        }
    }
}

fn default_column_widths() -> HashMap<TableColumn, Pixels> {
    TableColumn::ALL
        .into_iter()
        .map(|col| (col, col.default_width()))
        .collect()
}

struct ResizeDrag {
    column: TableColumn,
    start_x: f32,
    start_width: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditColumn {
    Name,
    Value,
    Description,
}

#[derive(Clone)]
struct CellEdit {
    row_ix: usize,
    column: EditColumn,
    input: Entity<InputState>,
}

/// Root view for the User Variables window.
pub struct UserVariablesView {
    rows: Vec<VariableRow>,
    editing: Option<CellEdit>,
    list_scroll: UniformListScrollHandle,
    filter: VariablesFilter,
    column_widths: HashMap<TableColumn, Pixels>,
    resize_drag: Option<ResizeDrag>,
    h_offset: f32,
    viewport_width: f32,
    scrollbar_origin_x: f32,
    scrollbar_width: f32,
    scrollbar_drag: Option<f32>,
    /// Selected absolute row indices.
    selected: HashSet<usize>,
    /// Filtered-list index used as the shift-select anchor.
    selection_anchor: Option<usize>,
    focus_handle: FocusHandle,
}

impl UserVariablesView {
    /// Load current user variables from disk into the editor.
    pub fn new(cx: &mut Context<Self>) -> Self {
        user_variables::ensure_store(cx);
        user_variables::reload_from_disk(cx);
        let rows = rows_from_file(&user_variables::store(cx).file);
        let mut filter = VariablesFilter::default();
        filter.sync_scopes_from_rows(&rows);
        Self {
            rows,
            editing: None,
            list_scroll: UniformListScrollHandle::new(),
            filter,
            column_widths: default_column_widths(),
            resize_drag: None,
            h_offset: 0.0,
            viewport_width: 0.0,
            scrollbar_origin_x: 0.0,
            scrollbar_width: 0.0,
            scrollbar_drag: None,
            selected: HashSet::new(),
            selection_anchor: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Persist the edited rows to `variables.json` and the global store.
    pub fn save_to_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        let table = table_from_rows(&self.rows);
        let file = UserVariablesFile::from_table(&table);
        if let Err(err) = file.save() {
            eprintln!("FieldAssist: failed to save variables.json: {err}");
            return;
        }
        user_variables::store_mut(cx).file = file;
    }

    /// Persist without a live window (best-effort; skips unfinished edit commit).
    pub fn save_to_disk_no_window(&mut self, cx: &mut Context<Self>) {
        let table = table_from_rows(&self.rows);
        let file = UserVariablesFile::from_table(&table);
        if let Err(err) = file.save() {
            eprintln!("FieldAssist: failed to save variables.json: {err}");
            return;
        }
        user_variables::store_mut(cx).file = file;
    }

    fn column_width(&self, col: TableColumn) -> Pixels {
        self.column_widths
            .get(&col)
            .copied()
            .unwrap_or_else(|| col.default_width())
    }

    fn content_width(&self) -> Pixels {
        let cols: f32 = TableColumn::ALL
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

    fn set_column_width(&mut self, col: TableColumn, width: Pixels) {
        let clamped = width.max(px(MIN_COLUMN_WIDTH));
        self.column_widths.insert(col, clamped);
        self.clamp_h_offset();
    }

    fn begin_resize(&mut self, column: TableColumn, start_x: Pixels, _cx: &mut Context<Self>) {
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

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        let name = format!("var{}", self.rows.len() + 1);
        self.rows.push(VariableRow {
            name: name.clone(),
            value: String::new(),
            scope: "user".into(),
            description: None,
        });
        let ix = self.rows.len() - 1;
        self.selected.clear();
        self.selected.insert(ix);
        self.filter.sync_scopes_from_rows(&self.rows);
        if let Some(filtered_ix) = self
            .filter
            .filtered_indices(&self.rows, cx)
            .into_iter()
            .position(|i| i == ix)
        {
            self.selection_anchor = Some(filtered_ix);
            self.list_scroll
                .scroll_to_item(filtered_ix, ScrollStrategy::Center);
        }
        let entity = cx.entity().clone();
        cx.notify();
        window.defer(cx, move |window, cx| {
            entity.update(cx, |this, cx| {
                this.begin_edit(ix, EditColumn::Name, window, cx);
            });
        });
    }

    fn select_row(
        &mut self,
        filtered_ix: usize,
        row_ix: usize,
        toggle: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        if extend {
            let filtered = self.filter.filtered_indices(&self.rows, cx);
            let anchor = self.selection_anchor.unwrap_or(filtered_ix);
            let (lo, hi) = if anchor <= filtered_ix {
                (anchor, filtered_ix)
            } else {
                (filtered_ix, anchor)
            };
            self.selected.clear();
            for vis in lo..=hi {
                if let Some(&ix) = filtered.get(vis) {
                    self.selected.insert(ix);
                }
            }
        } else if toggle {
            if !self.selected.remove(&row_ix) {
                self.selected.insert(row_ix);
            }
            self.selection_anchor = Some(filtered_ix);
        } else {
            self.selected.clear();
            self.selected.insert(row_ix);
            self.selection_anchor = Some(filtered_ix);
        }
        cx.notify();
    }

    fn remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        if self.selected.is_empty() {
            return;
        }
        let mut indices: Vec<usize> = self.selected.iter().copied().collect();
        indices.sort_unstable();
        indices.dedup();
        for ix in indices.into_iter().rev() {
            if ix < self.rows.len() {
                self.rows.remove(ix);
            }
        }
        self.selected.clear();
        self.selection_anchor = None;
        self.editing = None;
        cx.notify();
    }

    fn begin_edit(
        &mut self,
        row_ix: usize,
        column: EditColumn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row_ix >= self.rows.len() {
            return;
        }
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        if row_ix >= self.rows.len() {
            return;
        }
        let seed = match column {
            EditColumn::Name => self.rows[row_ix].name.clone(),
            EditColumn::Value => self.rows[row_ix].value.clone(),
            EditColumn::Description => self.rows[row_ix].description.clone().unwrap_or_default(),
        };
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| {
            state.set_value(seed, window, cx);
            state.select_all(window, cx);
        });
        cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => this.commit_edit(window, cx),
                InputEvent::Blur => {
                    if this
                        .editing
                        .as_ref()
                        .is_some_and(|e| e.input.entity_id() == input.entity_id())
                    {
                        this.commit_edit(window, cx);
                    }
                }
                _ => {}
            },
        )
        .detach();
        self.editing = Some(CellEdit {
            row_ix,
            column,
            input: input.clone(),
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn commit_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        let raw = edit.input.read(cx).value().to_string();
        let text = raw.trim().to_string();
        if edit.row_ix >= self.rows.len() {
            cx.notify();
            return;
        }
        match edit.column {
            EditColumn::Name => {
                if text.is_empty() || text.contains('.') {
                    cx.notify();
                    return;
                }
                if self
                    .rows
                    .iter()
                    .enumerate()
                    .any(|(i, r)| i != edit.row_ix && r.name == text)
                {
                    cx.notify();
                    return;
                }
                self.rows[edit.row_ix].name = text;
            }
            EditColumn::Value => {
                self.rows[edit.row_ix].value = text;
            }
            EditColumn::Description => {
                self.rows[edit.row_ix].description =
                    if text.is_empty() { None } else { Some(text) };
            }
        }
        cx.notify();
    }

    fn move_edit(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.clone() else {
            return;
        };
        let cols = [EditColumn::Name, EditColumn::Value, EditColumn::Description];
        let Some(col_ix) = cols.iter().position(|c| *c == edit.column) else {
            self.commit_edit(window, cx);
            return;
        };
        let visible = self.filter.filtered_indices(&self.rows, cx);
        let Some(vis_ix) = visible.iter().position(|&i| i == edit.row_ix) else {
            self.commit_edit(window, cx);
            return;
        };
        let (next_row, next_col) = if forward {
            if col_ix + 1 < cols.len() {
                (edit.row_ix, cols[col_ix + 1])
            } else if vis_ix + 1 < visible.len() {
                (visible[vis_ix + 1], cols[0])
            } else {
                self.commit_edit(window, cx);
                return;
            }
        } else if col_ix > 0 {
            (edit.row_ix, cols[col_ix - 1])
        } else if vis_ix > 0 {
            (visible[vis_ix - 1], cols[cols.len() - 1])
        } else {
            self.commit_edit(window, cx);
            return;
        };
        self.commit_edit(window, cx);
        if let Some(filtered_ix) = visible.iter().position(|&i| i == next_row) {
            self.list_scroll
                .scroll_to_item(filtered_ix, ScrollStrategy::Center);
        }
        self.begin_edit(next_row, next_col, window, cx);
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let content_width = self.content_width();
        h_flex()
            .id("user-variables-header")
            .w(content_width)
            .flex_none()
            .items_center()
            .px_1p5()
            .py_0p5()
            .children(TableColumn::ALL.iter().enumerate().flat_map(|(ix, col)| {
                let col = *col;
                let width = self.column_width(col);
                let mut cells = Vec::new();
                cells.push(
                    div()
                        .id(("user-var-th", ix as u64))
                        .w(width)
                        .flex_none()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(col.label())
                        .into_any_element(),
                );
                cells.push(resize_handle(col, cx).into_any_element());
                cells
            }))
    }
}

impl Focusable for UserVariablesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UserVariablesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.filter.ensure_search(window, cx, |_, cx| cx.notify());
        self.filter.sync_scopes_from_rows(&self.rows);
        self.clamp_h_offset();
        let content = content_foreground(cx);
        Theme::global_mut(cx).foreground = content;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let selected_bg = cx.theme().list_active;
        let track = cx.theme().scrollbar;
        let thumb = cx.theme().scrollbar_thumb;
        let rows = self.rows.clone();
        let filtered = self.filter.filtered_indices(&rows, cx);
        let count = filtered.len();
        let entity = cx.entity().clone();
        let entity_id = cx.entity_id();
        let editing = self.editing.clone();
        let selected = self.selected.clone();
        let widths = self.column_widths.clone();
        let content_width = self.content_width();
        let content_w = self.content_width_f32();
        let h_offset = self.h_offset;
        let viewport_w = self.viewport_width;
        let body_width = px(content_w.max(viewport_w));
        let show_h_scroll = self.needs_h_scroll();
        let header = self.render_header(cx);
        let filter_bar = variables_filter::filter_bar(
            &self.filter,
            &rows,
            entity.clone(),
            border,
            true,
            |this, scope, cx| {
                this.filter.toggle_scope(&scope);
                cx.notify();
            },
            |this, window, cx| this.add_row(window, cx),
            |this, window, cx| this.remove_selected(window, cx),
        );

        let body = if count == 0 {
            div()
                .id("user-variables-empty")
                .size_full()
                .w(body_width)
                .p_3()
                .text_xs()
                .text_color(muted)
                .child(if rows.is_empty() {
                    "(no user variables)"
                } else {
                    "(no matching variables)"
                })
                .into_any_element()
        } else {
            uniform_list("user-variables-rows", count, {
                let entity = entity.clone();
                let filtered = filtered.clone();
                let selected = selected.clone();
                let widths = widths.clone();
                move |range, _, _cx| {
                    range
                        .map(|vis_ix| {
                            let ix = filtered[vis_ix];
                            render_row(
                                vis_ix,
                                ix,
                                &rows[ix],
                                &widths,
                                content_width,
                                selected.contains(&ix),
                                selected_bg,
                                editing.as_ref(),
                                muted,
                                entity.clone(),
                            )
                        })
                        .collect()
                }
            })
            .track_scroll(&self.list_scroll)
            .size_full()
            .w(body_width)
            .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(content)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.editing.is_some() {
                    this.move_edit(true, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &OutdentInline, window, cx| {
                if this.editing.is_some() {
                    this.move_edit(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(crate::components::window_chrome::window_title_bar(
                "User Variables",
            ))
            .child(
                h_flex()
                    .id("user-variables-filter-row")
                    .w_full()
                    .flex_none()
                    .border_b_1()
                    .border_color(border)
                    .child(filter_bar),
            )
            .child(
                div()
                    .id("user-variables-header-clip")
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .id("user-variables-header-pan")
                            .w(content_width)
                            .ml(px(-h_offset))
                            .child(header),
                    ),
            )
            .child(
                div()
                    .id("user-variables-viewport")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .relative()
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
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
                        this.pan_horizontal(-pan, cx);
                        cx.stop_propagation();
                    }))
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
                            .id("user-variables-body")
                            .absolute()
                            .top_0()
                            .left(px(-h_offset))
                            .h_full()
                            .w(body_width)
                            .min_w(body_width)
                            .child(body),
                    ),
            )
            .when(show_h_scroll, |this| {
                this.child(render_h_scrollbar(
                    entity, entity_id, content_w, viewport_w, h_offset, track, thumb, border, cx,
                ))
            })
    }
}

fn resize_handle(column: TableColumn, cx: &mut Context<UserVariablesView>) -> impl IntoElement {
    h_flex()
        .id(("user-var-resize", column.stable_id()))
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

fn install_resize_listeners(entity: Entity<UserVariablesView>, window: &mut Window) {
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

fn h_scroll_geom(content: f32, viewport: f32, offset: f32, track: f32) -> (f32, f32) {
    if content <= viewport || track <= 0.0 {
        return (track, 0.0);
    }
    let thumb_w = ((viewport / content) * track).max(MIN_H_THUMB).min(track);
    let max_offset = (content - viewport).max(0.0);
    let thumb_x = if max_offset <= f32::EPSILON {
        0.0
    } else {
        (offset / max_offset) * (track - thumb_w).max(0.0)
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
    entity: Entity<UserVariablesView>,
    entity_id: gpui_kit::EntityId,
    content: f32,
    viewport: f32,
    offset: f32,
    track: gpui_kit::Hsla,
    thumb: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    cx: &mut Context<UserVariablesView>,
) -> impl IntoElement {
    div()
        .id("user-variables-h-scroll")
        .w_full()
        .h(px(H_SCROLLBAR_HEIGHT))
        .flex_none()
        .border_t_1()
        .border_color(border)
        .child(
            div()
                .id("user-variables-h-scroll-track")
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
    entity_id: gpui_kit::EntityId,
}

impl Render for DragHScroll {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn render_row(
    display_ix: usize,
    row_ix: usize,
    row: &VariableRow,
    widths: &HashMap<TableColumn, Pixels>,
    content_width: Pixels,
    is_selected: bool,
    selected_bg: gpui_kit::Hsla,
    editing: Option<&CellEdit>,
    muted: gpui_kit::Hsla,
    entity: Entity<UserVariablesView>,
) -> gpui_kit::AnyElement {
    let name = row.name.clone();
    let mut row_el = h_flex()
        .id(SharedString::from(format!("user-var-row-{row_ix}-{name}")))
        .w(content_width)
        .flex_none()
        .items_center()
        .px_1p5()
        .py_0p5();
    if is_selected {
        row_el = row_el.bg(selected_bg);
    }
    row_el
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            move |event, _, cx| {
                let toggle = event.modifiers.platform || event.modifiers.control;
                let extend = event.modifiers.shift;
                entity.update(cx, |this, cx| {
                    this.select_row(display_ix, row_ix, toggle, extend, cx);
                });
            }
        })
        .children(TableColumn::ALL.iter().flat_map(|col| {
            let col = *col;
            let width = widths
                .get(&col)
                .copied()
                .unwrap_or_else(|| col.default_width());
            let mut cells = Vec::new();
            let text = match col {
                TableColumn::Row => (display_ix + 1).to_string(),
                TableColumn::Name => single_line_display(&row.name),
                TableColumn::Value => single_line_display(&row.value),
                TableColumn::Description => {
                    single_line_display(row.description.as_deref().unwrap_or(""))
                }
            };
            if let Some(edit_col) = col.edit_column() {
                cells.push(edit_cell(
                    display_ix,
                    row_ix,
                    edit_col,
                    &text,
                    width,
                    editing,
                    entity.clone(),
                ));
            } else {
                cells.push(
                    div()
                        .w(width)
                        .flex_none()
                        .min_w_0()
                        .overflow_hidden()
                        .text_xs()
                        .text_color(muted)
                        .child(text)
                        .into_any_element(),
                );
            }
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

fn edit_cell(
    display_ix: usize,
    row_ix: usize,
    column: EditColumn,
    text: &str,
    width: Pixels,
    editing: Option<&CellEdit>,
    entity: Entity<UserVariablesView>,
) -> gpui_kit::AnyElement {
    let is_editing = editing.is_some_and(|e| e.row_ix == row_ix && e.column == column);
    let col = match column {
        EditColumn::Name => "name",
        EditColumn::Value => "value",
        EditColumn::Description => "description",
    };
    let mut cell = div()
        .id(ElementId::Name(SharedString::from(format!(
            "user-var-cell-{row_ix}-{col}"
        ))))
        .w(width)
        .flex_none()
        .min_w_0()
        .overflow_hidden()
        .text_xs();
    if is_editing {
        if let Some(edit) = editing {
            let entity_tab = entity.clone();
            let entity_shift = entity.clone();
            cell = cell
                .on_action({
                    let entity = entity_tab;
                    move |_: &IndentInline, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.move_edit(true, window, cx);
                        });
                        cx.stop_propagation();
                    }
                })
                .on_action({
                    let entity = entity_shift;
                    move |_: &OutdentInline, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.move_edit(false, window, cx);
                        });
                        cx.stop_propagation();
                    }
                })
                .child(Input::new(&edit.input).xsmall().w_full())
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                });
        }
    } else {
        let display = text.to_string();
        cell = cell
            .whitespace_nowrap()
            .text_ellipsis()
            .cursor_text()
            .child(display)
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    this.select_row(display_ix, row_ix, false, false, cx);
                    this.begin_edit(row_ix, column, window, cx);
                });
            });
    }
    cell.into_any_element()
}

fn single_line_display(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rows_from_file(file: &UserVariablesFile) -> Vec<VariableRow> {
    file.variables
        .iter()
        .map(|v| VariableRow {
            name: v.name.clone(),
            value: v.value.clone(),
            scope: "user".into(),
            description: v.description.clone(),
        })
        .collect()
}

fn table_from_rows(rows: &[VariableRow]) -> VariableTable {
    let mut table = VariableTable::new();
    for row in rows {
        if row.name.trim().is_empty() {
            continue;
        }
        let mut entry = VariableEntry::new("user", &row.name, &row.value);
        if let Some(desc) = &row.description {
            entry = entry.with_description(desc.clone());
        }
        table.upsert(entry);
    }
    table
}

#[derive(Clone)]
struct UserVariablesWindow {
    handle: gpui_kit::AnyWindowHandle,
    view: Entity<UserVariablesView>,
}

impl Global for UserVariablesWindow {}

fn window_is_open(cx: &App, handle: gpui_kit::AnyWindowHandle) -> bool {
    cx.windows().iter().any(|window| *window == handle)
}

/// Open or focus the singleton User Variables window.
pub fn open_user_variables_window(cx: &mut App) {
    user_variables::ensure_store(cx);

    if let Some(UserVariablesWindow { handle, .. }) =
        cx.try_global::<UserVariablesWindow>().cloned()
    {
        if window_is_open(cx, handle) {
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
            return;
        }
        let _ = cx.remove_global::<UserVariablesWindow>();
    }

    use gpui_kit::{point, px};

    let view = cx.new(UserVariablesView::new);
    let view_for_window = view.clone();
    let (origin, size) = (
        point(px(180.), px(140.)),
        crate::components::window_chrome::window_size(720., 480.),
    );
    let options =
        crate::components::window_chrome::themed_window_options("User Variables", origin, size);

    match cx.open_window(options, move |window, cx| {
        window.focus(&view_for_window.focus_handle(cx), cx);
        cx.new(|cx| Root::new(view_for_window.clone(), window, cx).bg(cx.theme().background))
    }) {
        Ok(handle) => {
            cx.set_global(UserVariablesWindow {
                handle: handle.into(),
                view,
            });
        }
        Err(err) => {
            eprintln!("failed to open User Variables window: {err}");
        }
    }
}

/// Persist edits and clear the singleton when the window closes.
pub fn on_user_variables_window_closed(cx: &mut App, id: gpui_kit::WindowId) {
    let Some(UserVariablesWindow { handle, view }) =
        cx.try_global::<UserVariablesWindow>().cloned()
    else {
        return;
    };
    if handle.window_id() != id {
        return;
    }
    let _ = cx.remove_global::<UserVariablesWindow>();
    let saved = handle.update(cx, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.save_to_disk(window, cx);
        });
    });
    if saved.is_err() {
        view.update(cx, |this, cx| {
            this.save_to_disk_no_window(cx);
        });
    }
    propagate_user_variables(cx);
    crate::app::apply_muted_chrome(cx);
}

fn propagate_user_variables(cx: &mut App) {
    let table = user_variables::store(cx).file.to_table();
    if let Some((app_view, _)) = crate::app::living_editor_window(cx) {
        app_view.update(cx, |this, cx| {
            this.apply_user_variables(table, cx);
        });
    }
}
