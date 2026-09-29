// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Bottom-dock Variables panel (Media-pool table chrome).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use field_ui_components::VariableRow;
use field_variables::{compose, top_level_scope, VariableEntry, VariableTable};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    input::{IndentInline, Input, InputEvent, InputState, OutdentInline},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex, ActiveTheme as _, IconName, Sizable as _,
};
use gpui_kit::{
    canvas, div, fill, point, prelude::FluentBuilder as _, px, size, uniform_list, App,
    AppContext as _, Bounds, Context, DispatchPhase, DragMoveEvent, ElementId, Empty, Entity,
    EntityId, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render,
    ScrollStrategy, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window,
};

use crate::components::variables_filter::{self, VariablesFilter};
use crate::dock_titles::BOTTOM_TAB_VARIABLES;

const MIN_COLUMN_WIDTH: f32 = 32.;
const ELLIPSIS_WIDTH: f32 = 28.;
const RESIZE_HANDLE_WIDTH: f32 = 5.;
const ROW_PAD_X: f32 = 6.;
const H_SCROLLBAR_HEIGHT: f32 = 7.;
const MIN_H_THUMB: f32 = 24.;

/// Snapshot + edit callbacks for the Variables dock.
#[derive(Clone, Debug, Default)]
pub struct VariablesPanelState {
    /// Composed rows for display.
    pub rows: Vec<VariableRow>,
    /// User-scoped table.
    pub user: VariableTable,
    /// Session-scoped table.
    pub session: VariableTable,
    /// Composition-scoped table.
    pub composition: VariableTable,
}

/// Build display rows from a composed table.
pub fn rows_from_composed(composed: &VariableTable) -> Vec<VariableRow> {
    composed
        .entries()
        .iter()
        .map(|e| VariableRow {
            name: e.name.clone(),
            value: e.value.clone(),
            scope: e.scope.clone(),
            description: e.description.clone(),
        })
        .collect()
}

/// Compose source + user + session + composition for the Variables view.
pub fn compose_for_view(
    source: &VariableTable,
    user: &VariableTable,
    session: &VariableTable,
    composition: &VariableTable,
) -> VariableTable {
    compose(&[
        source.clone(),
        user.clone(),
        session.clone(),
        composition.clone(),
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VarColumn {
    Row,
    Name,
    Value,
    Scope,
    Description,
}

impl VarColumn {
    /// Columns that may be reordered / shown-hidden (not the fixed `#` column).
    const FLEX: [Self; 4] = [Self::Name, Self::Value, Self::Scope, Self::Description];

    fn label(self) -> &'static str {
        match self {
            Self::Row => "#",
            Self::Name => "name",
            Self::Value => "value",
            Self::Scope => "scope",
            Self::Description => "description",
        }
    }

    fn menu_label(self) -> &'static str {
        match self {
            Self::Row => "Row",
            Self::Name => "Name",
            Self::Value => "Value",
            Self::Scope => "Scope",
            Self::Description => "Description",
        }
    }

    fn default_width(self) -> Pixels {
        match self {
            Self::Row => px(36.),
            Self::Name => px(140.),
            Self::Value => px(220.),
            Self::Scope => px(140.),
            Self::Description => px(200.),
        }
    }

    fn stable_id(self) -> u64 {
        match self {
            Self::Row => 0,
            Self::Name => 1,
            Self::Value => 2,
            Self::Scope => 3,
            Self::Description => 4,
        }
    }
}

fn default_column_order() -> Vec<VarColumn> {
    Vec::from(VarColumn::FLEX)
}

fn default_column_widths() -> HashMap<VarColumn, Pixels> {
    let mut map = HashMap::new();
    map.insert(VarColumn::Row, VarColumn::Row.default_width());
    for col in VarColumn::FLEX {
        map.insert(col, col.default_width());
    }
    map
}

fn default_hidden_columns() -> HashSet<VarColumn> {
    HashSet::from([VarColumn::Description])
}

fn move_column(order: &mut Vec<VarColumn>, from: VarColumn, to: VarColumn) {
    if from == to || from == VarColumn::Row || to == VarColumn::Row {
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

struct ResizeDrag {
    column: VarColumn,
    start_x: f32,
    start_width: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditColumn {
    Name,
    Value,
}

#[derive(Clone)]
struct CellEdit {
    scope: String,
    name: String,
    column: EditColumn,
    input: Entity<InputState>,
}

fn scope_is_editable(scope: &str) -> bool {
    matches!(top_level_scope(scope), "user" | "session" | "composition")
}

/// Bottom-dock Variables panel.
pub struct VariablesPanel {
    title: SharedString,
    state: VariablesPanelState,
    column_order: Vec<VarColumn>,
    column_widths: HashMap<VarColumn, Pixels>,
    hidden: HashSet<VarColumn>,
    list_scroll: UniformListScrollHandle,
    h_offset: f32,
    viewport_width: f32,
    scrollbar_origin_x: f32,
    scrollbar_width: f32,
    scrollbar_drag: Option<f32>,
    resize_drag: Option<ResizeDrag>,
    editing: Option<CellEdit>,
    /// After host refresh, scroll so this `(scope, name)` row is visible.
    pending_reveal: Option<(String, String)>,
    filter: VariablesFilter,
    /// Selected rows keyed by `(scope, name)`.
    selected: HashSet<(String, String)>,
    /// Filtered-list index used as the shift-select anchor.
    selection_anchor: Option<usize>,
    focus_handle: FocusHandle,
    on_change: Option<Rc<dyn Fn(VariablesPanelState, &mut Window, &mut App)>>,
}

impl VariablesPanel {
    /// Create an empty panel.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            title: BOTTOM_TAB_VARIABLES.into(),
            state: VariablesPanelState::default(),
            column_order: default_column_order(),
            column_widths: default_column_widths(),
            hidden: default_hidden_columns(),
            list_scroll: UniformListScrollHandle::new(),
            h_offset: 0.0,
            viewport_width: 0.0,
            scrollbar_origin_x: 0.0,
            scrollbar_width: 0.0,
            scrollbar_drag: None,
            resize_drag: None,
            editing: None,
            pending_reveal: None,
            filter: VariablesFilter::default(),
            selected: HashSet::new(),
            selection_anchor: None,
            focus_handle: cx.focus_handle(),
            on_change: None,
        }
    }

    /// Replace displayed state.
    pub fn set_state(&mut self, state: VariablesPanelState, cx: &mut Context<Self>) {
        self.state = state;
        self.filter.sync_scopes_from_rows(&self.state.rows);
        if let Some((scope, name)) = self.pending_reveal.take() {
            if let Some(filtered_ix) = self
                .filter
                .filtered_indices(&self.state.rows, cx)
                .into_iter()
                .position(|ix| {
                    let r = &self.state.rows[ix];
                    r.scope == scope && r.name == name
                })
            {
                self.list_scroll
                    .scroll_to_item(filtered_ix, ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    /// Host callback when the user edits variables.
    pub fn set_on_change(
        &mut self,
        handler: impl Fn(VariablesPanelState, &mut Window, &mut App) + 'static,
    ) {
        self.on_change = Some(Rc::new(handler));
    }

    fn emit_change(&self, window: &mut Window, cx: &mut Context<Self>) {
        // Defer the host callback so it can refresh this panel without nesting
        // `Entity::update` (GPUI panics on re-entrant updates).
        if let Some(handler) = self.on_change.clone() {
            let state = self.state.clone();
            window.defer(cx, move |window, cx| {
                handler(state, window, cx);
            });
        }
        cx.notify();
    }

    fn upsert_editable(&mut self, name: &str, value: &str, scope: &str) {
        let entry = VariableEntry::new(scope, name, value);
        match scope {
            "user" => {
                self.state.session.remove_in_scope("session", name);
                self.state.composition.remove_in_scope("composition", name);
                self.state.user.upsert(entry);
            }
            "session" => {
                self.state.user.remove_in_scope("user", name);
                self.state.composition.remove_in_scope("composition", name);
                self.state.session.upsert(entry);
            }
            _ => {
                self.state.user.remove_in_scope("user", name);
                self.state.session.remove_in_scope("session", name);
                self.state
                    .composition
                    .upsert(VariableEntry::new("composition", name, value));
            }
        }
    }

    fn add_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        let name = format!("var{}", self.state.composition.len() + 1);
        let scope = "composition".to_string();
        self.upsert_editable(&name, "", &scope);
        self.state.rows.push(VariableRow {
            name: name.clone(),
            value: String::new(),
            scope: scope.clone(),
            description: None,
        });
        self.pending_reveal = Some((scope.clone(), name.clone()));
        if let Some(filtered_ix) = self
            .filter
            .filtered_indices(&self.state.rows, cx)
            .into_iter()
            .position(|ix| {
                let r = &self.state.rows[ix];
                r.scope == scope && r.name == name
            })
        {
            self.list_scroll
                .scroll_to_item(filtered_ix, ScrollStrategy::Center);
        }
        // Defer edit-start until after the dropdown closes and the host
        // refresh runs, so focus isn't stolen by menu teardown.
        let entity = cx.entity().clone();
        self.emit_change(window, cx);
        window.defer(cx, move |window, cx| {
            entity.update(cx, |this, cx| {
                this.begin_edit(&scope, &name, EditColumn::Name, window, cx);
            });
        });
    }

    fn begin_edit(
        &mut self,
        scope: &str,
        name: &str,
        column: EditColumn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !scope_is_editable(scope) {
            return;
        }
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        let Some(row) = self
            .state
            .rows
            .iter()
            .find(|r| r.scope == scope && r.name == name)
        else {
            return;
        };
        let seed = match column {
            EditColumn::Name => row.name.clone(),
            EditColumn::Value => row.value.clone(),
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
                // Only commit when this input is still the active editor. Tab
                // navigation commits then focuses a new input; the old Blur
                // must not take() the new edit.
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
            scope: scope.to_string(),
            name: name.to_string(),
            column,
            input: input.clone(),
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Visible Name/Value cells on editable rows, left-to-right then top-to-bottom.
    fn editable_cell_targets(&self, cx: &App) -> Vec<(String, String, EditColumn)> {
        let mut edit_cols: Vec<EditColumn> = Vec::new();
        for col in &self.column_order {
            if self.hidden.contains(col) {
                continue;
            }
            match col {
                VarColumn::Name => edit_cols.push(EditColumn::Name),
                VarColumn::Value => edit_cols.push(EditColumn::Value),
                _ => {}
            }
        }
        if edit_cols.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for ix in self.filter.filtered_indices(&self.state.rows, cx) {
            let row = &self.state.rows[ix];
            if !scope_is_editable(&row.scope) {
                continue;
            }
            for col in &edit_cols {
                out.push((row.scope.clone(), row.name.clone(), *col));
            }
        }
        out
    }

    /// Commit the current cell and move to the adjacent editable column.
    fn move_edit(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.clone() else {
            return;
        };
        let cells = self.editable_cell_targets(cx);
        let Some(ix) = cells
            .iter()
            .position(|(s, n, c)| *s == edit.scope && *n == edit.name && *c == edit.column)
        else {
            self.commit_edit(window, cx);
            return;
        };
        let next = if forward {
            cells.get(ix + 1).cloned()
        } else {
            ix.checked_sub(1).and_then(|i| cells.get(i).cloned())
        };
        let Some((mut next_scope, mut next_name, next_col)) = next else {
            self.commit_edit(window, cx);
            return;
        };
        let same_row = next_scope == edit.scope && next_name == edit.name;
        let was_name_edit = edit.column == EditColumn::Name;
        self.commit_edit(window, cx);
        // Renaming the Name cell updates the row key before Value opens.
        if was_name_edit && same_row {
            if let Some((scope, name)) = self.pending_reveal.clone() {
                next_scope = scope;
                next_name = name;
            }
        }
        if let Some(filtered_ix) = self
            .filter
            .filtered_indices(&self.state.rows, cx)
            .into_iter()
            .position(|ix| {
                let r = &self.state.rows[ix];
                r.scope == next_scope && r.name == next_name
            })
        {
            self.list_scroll
                .scroll_to_item(filtered_ix, ScrollStrategy::Center);
        }
        self.begin_edit(&next_scope, &next_name, next_col, window, cx);
    }

    fn select_row(
        &mut self,
        filtered_ix: usize,
        scope: &str,
        name: &str,
        toggle: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (scope.to_string(), name.to_string());
        if extend {
            let filtered = self.filter.filtered_indices(&self.state.rows, cx);
            let anchor = self.selection_anchor.unwrap_or(filtered_ix);
            let (lo, hi) = if anchor <= filtered_ix {
                (anchor, filtered_ix)
            } else {
                (filtered_ix, anchor)
            };
            self.selected.clear();
            for vis in lo..=hi {
                if let Some(&ix) = filtered.get(vis) {
                    let row = &self.state.rows[ix];
                    self.selected.insert((row.scope.clone(), row.name.clone()));
                }
            }
        } else if toggle {
            if !self.selected.remove(&key) {
                self.selected.insert(key);
            }
            self.selection_anchor = Some(filtered_ix);
        } else {
            self.selected.clear();
            self.selected.insert(key);
            self.selection_anchor = Some(filtered_ix);
        }
        cx.notify();
    }

    fn remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit(window, cx);
        }
        let keys: Vec<(String, String)> = self.selected.iter().cloned().collect();
        if keys.is_empty() {
            return;
        }
        let mut removed = false;
        for (scope, name) in keys {
            // Dock [-] only deletes composition-scoped rows; user/session
            // overrides are edited elsewhere (User Variables / session).
            if top_level_scope(&scope) != "composition" {
                continue;
            }
            self.remove_editable(&scope, &name);
            self.state
                .rows
                .retain(|r| !(r.scope == scope && r.name == name));
            self.selected.remove(&(scope, name));
            removed = true;
        }
        if removed {
            self.selection_anchor = None;
            self.emit_change(window, cx);
        } else {
            cx.notify();
        }
    }

    fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        let raw = edit.input.read(cx).value().to_string();
        let text = raw.trim().to_string();
        let Some(row_ix) = self
            .state
            .rows
            .iter()
            .position(|r| r.scope == edit.scope && r.name == edit.name)
        else {
            cx.notify();
            return;
        };
        match edit.column {
            EditColumn::Name => {
                if text.is_empty() || text == edit.name || text.contains('.') {
                    cx.notify();
                    return;
                }
                let value = self.state.rows[row_ix].value.clone();
                let description = self.state.rows[row_ix].description.clone();
                self.remove_editable(&edit.scope, &edit.name);
                self.upsert_editable_entry(&edit.scope, &text, &value, description);
                self.state.rows[row_ix].name = text.clone();
                if self
                    .selected
                    .remove(&(edit.scope.clone(), edit.name.clone()))
                {
                    self.selected.insert((edit.scope.clone(), text.clone()));
                }
                self.pending_reveal = Some((edit.scope.clone(), text));
            }
            EditColumn::Value => {
                let name = edit.name.clone();
                let description = self.state.rows[row_ix].description.clone();
                self.upsert_editable_entry(&edit.scope, &name, &text, description);
                self.state.rows[row_ix].value = text;
                self.pending_reveal = Some((edit.scope.clone(), name));
            }
        }
        self.emit_change(window, cx);
    }

    fn remove_editable(&mut self, scope: &str, name: &str) {
        match top_level_scope(scope) {
            "user" => {
                let _ = self.state.user.remove_in_scope(scope, name);
            }
            "session" => {
                let _ = self.state.session.remove_in_scope(scope, name);
            }
            _ => {
                let _ = self.state.composition.remove_in_scope(scope, name);
            }
        }
    }

    fn upsert_editable_entry(
        &mut self,
        scope: &str,
        name: &str,
        value: &str,
        description: Option<String>,
    ) {
        let mut entry = VariableEntry::new(scope, name, value);
        if let Some(description) = description {
            entry = entry.with_description(description);
        }
        match top_level_scope(scope) {
            "user" => {
                self.state.session.remove_in_scope("session", name);
                self.state.composition.remove_in_scope("composition", name);
                entry.scope = scope.to_string();
                self.state.user.upsert(entry);
            }
            "session" => {
                self.state.user.remove_in_scope("user", name);
                self.state.composition.remove_in_scope("composition", name);
                entry.scope = scope.to_string();
                self.state.session.upsert(entry);
            }
            _ => {
                self.state.user.remove_in_scope("user", name);
                self.state.session.remove_in_scope("session", name);
                entry.scope = scope.to_string();
                self.state.composition.upsert(entry);
            }
        }
    }

    fn cell_text(row: &VariableRow, row_ix: usize, column: VarColumn) -> String {
        let raw = match column {
            VarColumn::Row => return (row_ix + 1).to_string(),
            VarColumn::Name => row.name.as_str(),
            VarColumn::Value => row.value.as_str(),
            VarColumn::Scope => row.scope.as_str(),
            VarColumn::Description => row.description.as_deref().unwrap_or(""),
        };
        single_line_display(raw)
    }

    /// Visible columns with `#` pinned first.
    fn visible_columns(&self) -> Vec<VarColumn> {
        let mut cols = vec![VarColumn::Row];
        cols.extend(
            self.column_order
                .iter()
                .copied()
                .filter(|col| !self.hidden.contains(col)),
        );
        cols
    }

    fn column_width(&self, col: VarColumn) -> Pixels {
        self.column_widths
            .get(&col)
            .copied()
            .unwrap_or_else(|| col.default_width())
    }

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

    fn set_column_width(&mut self, col: VarColumn, width: Pixels) {
        let clamped = width.max(px(MIN_COLUMN_WIDTH));
        self.column_widths.insert(col, clamped);
        self.clamp_h_offset();
    }

    fn toggle_column_visible(&mut self, col: VarColumn, cx: &mut Context<Self>) {
        if col == VarColumn::Row {
            return;
        }
        if self.hidden.contains(&col) {
            self.hidden.remove(&col);
        } else {
            self.hidden.insert(col);
        }
        self.clamp_h_offset();
        cx.notify();
    }

    fn move_column_to(&mut self, from: VarColumn, to: VarColumn, cx: &mut Context<Self>) {
        move_column(&mut self.column_order, from, to);
        cx.notify();
    }

    fn begin_resize(&mut self, column: VarColumn, start_x: Pixels, _cx: &mut Context<Self>) {
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
            .id("variables-header")
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
                let header_cell = div()
                    .id(("var-th", ix as u64))
                    .w(width)
                    .flex_none()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(muted);
                let header_cell = if col == VarColumn::Row {
                    header_cell.child(label).into_any_element()
                } else {
                    header_cell
                        .cursor_grab()
                        .on_drag(
                            DragVarColumn {
                                entity_id,
                                column: col,
                                name: SharedString::from(col.menu_label()),
                            },
                            |drag, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| drag.clone())
                            },
                        )
                        .on_drop(cx.listener(move |this, drag: &DragVarColumn, _, cx| {
                            if drag.entity_id != cx.entity_id() {
                                return;
                            }
                            this.move_column_to(drag.column, col, cx);
                        }))
                        .child(label)
                        .into_any_element()
                };
                cells.push(header_cell);
                cells.push(resize_handle(col, cx).into_any_element());
                cells
            }))
    }
}

fn resize_handle(column: VarColumn, cx: &mut Context<VariablesPanel>) -> impl IntoElement {
    h_flex()
        .id(("var-resize", column.stable_id()))
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

fn install_resize_listeners(entity: Entity<VariablesPanel>, window: &mut Window) {
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
    hidden: HashSet<VarColumn>,
    muted: gpui_kit::Hsla,
    cx: &mut Context<VariablesPanel>,
) -> impl IntoElement {
    let view = cx.entity().clone();
    Button::new("variables-menu")
        .ghost()
        .xsmall()
        .w(px(ELLIPSIS_WIDTH))
        .p_0()
        .icon(IconName::Ellipsis)
        .tooltip("Columns")
        .dropdown_menu(move |mut menu: PopupMenu, _, _| {
            for col in VarColumn::FLEX {
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
struct DragVarColumn {
    entity_id: EntityId,
    column: VarColumn,
    name: SharedString,
}

impl Render for DragVarColumn {
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

/// Collapse CR/LF/tabs and runs of whitespace so table cells stay one line.
fn single_line_display(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_row(
    id: impl Into<ElementId>,
    row: &VariableRow,
    row_ix: usize,
    order: &[VarColumn],
    widths: &HashMap<VarColumn, Pixels>,
    hidden: &HashSet<VarColumn>,
    content_width: Pixels,
    muted: gpui_kit::Hsla,
    selected_bg: Option<gpui_kit::Hsla>,
    is_selected: bool,
    editing: Option<&CellEdit>,
    entity: Entity<VariablesPanel>,
) -> gpui_kit::AnyElement {
    let mut visible = vec![VarColumn::Row];
    visible.extend(order.iter().copied().filter(|col| !hidden.contains(col)));
    let editable = scope_is_editable(&row.scope);
    let row_scope = row.scope.clone();
    let row_name = row.name.clone();
    let mut row_el = h_flex()
        .id(id)
        .w(content_width)
        .flex_none()
        .items_center()
        .px_1p5()
        .py_0p5();
    if is_selected {
        if let Some(bg) = selected_bg {
            row_el = row_el.bg(bg);
        }
    }
    row_el
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            let scope = row_scope.clone();
            let name = row_name.clone();
            move |event, _, cx| {
                let toggle = event.modifiers.platform || event.modifiers.control;
                let extend = event.modifiers.shift;
                entity.update(cx, |this, cx| {
                    this.select_row(row_ix, &scope, &name, toggle, extend, cx);
                });
            }
        })
        .children(visible.iter().flat_map(|col| {
            let col = *col;
            let width = widths
                .get(&col)
                .copied()
                .unwrap_or_else(|| col.default_width());
            let text = VariablesPanel::cell_text(row, row_ix, col);
            let tooltip_text = text.clone();
            let edit_col = match col {
                VarColumn::Name => Some(EditColumn::Name),
                VarColumn::Value => Some(EditColumn::Value),
                _ => None,
            };
            let is_editing = editing.is_some_and(|e| {
                e.scope == row_scope
                    && e.name == row_name
                    && edit_col.is_some_and(|c| c == e.column)
            });
            let edit_input = editing.filter(|_| is_editing).map(|e| e.input.clone());
            let mut cells = Vec::new();
            let mut cell = div()
                .id(ElementId::Name(SharedString::from(format!(
                    "var-td-{row_ix}-{}",
                    col.stable_id()
                ))))
                .w(width)
                .flex_none()
                .min_w_0()
                .overflow_hidden()
                .text_xs()
                .when(col == VarColumn::Row, |el| el.text_color(muted));
            if let Some(input) = edit_input {
                let entity_tab = entity.clone();
                let entity_shift_tab = entity.clone();
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
                        let entity = entity_shift_tab;
                        move |_: &OutdentInline, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.move_edit(false, window, cx);
                            });
                            cx.stop_propagation();
                        }
                    })
                    .child(Input::new(&input).xsmall().w_full())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    });
            } else {
                cell = cell
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when(!tooltip_text.is_empty(), |el| {
                        el.tooltip(move |window, cx| {
                            Tooltip::new(tooltip_text.clone()).build(window, cx)
                        })
                    })
                    .child(text);
                if editable {
                    if let Some(column) = edit_col {
                        let entity = entity.clone();
                        let scope = row_scope.clone();
                        let name = row_name.clone();
                        cell = cell.cursor_text().on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.select_row(row_ix, &scope, &name, false, false, cx);
                                this.begin_edit(&scope, &name, column, window, cx);
                            });
                        });
                    }
                }
            }
            cells.push(cell.into_any_element());
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

fn h_scroll_geom(content: f32, viewport: f32, offset: f32, track: f32) -> (f32, f32) {
    if content <= viewport || track <= 0.0 {
        return (track, 0.0);
    }
    let thumb_w = ((viewport / content) * track).max(MIN_H_THUMB).min(track);
    let max_travel = (track - thumb_w).max(0.0);
    let max_offset = (content - viewport).max(0.0);
    let thumb_x = if max_offset <= f32::EPSILON {
        0.0
    } else {
        (offset / max_offset) * max_travel
    };
    (thumb_w, thumb_x)
}

impl EventEmitter<PanelEvent> for VariablesPanel {}
impl Focusable for VariablesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl BasePanel for VariablesPanel {
    fn panel_name(&self) -> &'static str {
        "VariablesPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for VariablesPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for VariablesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.filter.ensure_search(window, cx, |_, cx| cx.notify());
        self.filter.sync_scopes_from_rows(&self.state.rows);
        self.clamp_h_offset();
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let selected_bg = cx.theme().list_active;
        let track = cx.theme().scrollbar;
        let thumb = cx.theme().scrollbar_thumb;
        let hidden = self.hidden.clone();
        let content_width = self.content_width();
        let content_w = self.content_width_f32();
        let h_offset = self.h_offset;
        let viewport_w = self.viewport_width;
        // Fill the viewport when columns are narrower so wheel/scroll over the
        // open space to the right of the last column still drives the list.
        let body_width = px(content_w.max(viewport_w));
        let show_h_scroll = self.needs_h_scroll();
        let header = self.render_header(cx);
        let menu = column_menu_button(hidden.clone(), muted, cx);
        let order = self.column_order.clone();
        let widths = self.column_widths.clone();
        let rows = self.state.rows.clone();
        let filtered = self.filter.filtered_indices(&rows, cx);
        let count = filtered.len();
        let entity = cx.entity().clone();
        let entity_id = cx.entity_id();
        let editing = self.editing.clone();
        let selected = self.selected.clone();
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
            |this, window, cx| this.add_variable(window, cx),
            |this, window, cx| this.remove_selected(window, cx),
        );

        let body = if count == 0 {
            div()
                .id("variables-empty")
                .size_full()
                .w(body_width)
                .p_2()
                .text_xs()
                .text_color(muted)
                .child(if rows.is_empty() {
                    "(no variables)"
                } else {
                    "(no matching variables)"
                })
                .into_any_element()
        } else {
            uniform_list("variables-rows", count, {
                let entity = entity.clone();
                let filtered = filtered.clone();
                let selected = selected.clone();
                move |range, _, _cx| {
                    range
                        .map(|vis_ix| {
                            let ix = filtered[vis_ix];
                            let row = &rows[ix];
                            let is_selected =
                                selected.contains(&(row.scope.clone(), row.name.clone()));
                            render_row(
                                ElementId::Name(SharedString::from(format!(
                                    "var-row-{vis_ix}-{}",
                                    row.name
                                ))),
                                row,
                                vis_ix,
                                &order,
                                &widths,
                                &hidden,
                                content_width,
                                muted,
                                Some(selected_bg),
                                is_selected,
                                editing.as_ref(),
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
            .id("variables-panel")
            .size_full()
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
            .child(
                h_flex()
                    .id("variables-filter-row")
                    .w_full()
                    .flex_none()
                    .border_b_1()
                    .border_color(border)
                    .child(filter_bar),
            )
            .child(
                h_flex()
                    .id("variables-header-bar")
                    .w_full()
                    .flex_none()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .id("variables-header-clip")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("variables-header-pan")
                                    .w(content_width)
                                    .ml(px(-h_offset))
                                    .child(header),
                            ),
                    )
                    .child(
                        h_flex()
                            .id("variables-chrome-header")
                            .w(px(ELLIPSIS_WIDTH))
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .child(menu),
                    ),
            )
            .child(
                h_flex()
                    .id("variables-main")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .id("variables-viewport")
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
                                    .id("variables-body")
                                    .absolute()
                                    .top_0()
                                    .left(px(-h_offset))
                                    .h_full()
                                    .w(body_width)
                                    .min_w(body_width)
                                    .child(body),
                            ),
                    )
                    .child(
                        div()
                            .id("variables-chrome-body")
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
    entity: Entity<VariablesPanel>,
    entity_id: EntityId,
    content: f32,
    viewport: f32,
    offset: f32,
    track: gpui_kit::Hsla,
    thumb: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    cx: &mut Context<VariablesPanel>,
) -> impl IntoElement {
    div()
        .id("variables-h-scroll")
        .w_full()
        .h(px(H_SCROLLBAR_HEIGHT))
        .flex_none()
        .border_t_1()
        .border_color(border)
        .child(
            div()
                .id("variables-h-scroll-track")
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
    fn single_line_display_collapses_crlf() {
        let raw = "sSPEED=030.000-ND\r\nsTAKE=002\r\nsNOTE=\r\n";
        assert_eq!(
            single_line_display(raw),
            "sSPEED=030.000-ND sTAKE=002 sNOTE="
        );
    }
}
