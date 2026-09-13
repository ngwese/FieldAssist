// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Markers list panel driven by a host [`MarkersData`] provider.

use std::rc::Rc;

use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _, StyledExt as _,
};
use gpui_kit::{
    actions, div, px, uniform_list, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, Rgba,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

#[allow(missing_docs)]
mod marker_actions {
    use super::*;
    actions!(markers_panel, [DeleteSelectedMarker]);
}
/// Deletes the selected or caret-highlighted marker.
pub use marker_actions::DeleteSelectedMarker;

const CONTEXT: &str = "Markers";

/// One marker row for the list panel.
#[derive(Clone, Debug)]
pub struct MarkerRow {
    /// Stable marker id.
    pub id: u64,
    /// Sample frame.
    pub frame: u64,
    /// Marker kind / type label.
    pub kind: String,
    /// Optional note text.
    pub note: String,
    /// RGBA swatch color.
    pub color: [f32; 4],
    /// Formatted time / sample stamp.
    pub stamp: String,
    /// Whether the caret highlights this marker.
    pub caret_highlight: bool,
}

/// Host-provided marker list data.
pub trait MarkersData {
    /// Cheap change detector for list refresh.
    fn fingerprint(&self) -> u64;
    /// Snapshot of marker rows for rendering.
    fn snapshot(&self) -> Vec<MarkerRow>;
}

/// Invoked when the user selects a marker.
pub type MarkerSelectHandler = Rc<dyn Fn(u64, u64, &mut Window, &mut App)>;

/// Invoked when the user deletes a marker.
pub type MarkerDeleteHandler = Rc<dyn Fn(u64, &mut Window, &mut App)>;

/// Detail-dock panel listing markers.
pub struct MarkersPanel<D: MarkersData + 'static> {
    title: SharedString,
    document: Option<Entity<D>>,
    on_select: Option<MarkerSelectHandler>,
    on_delete: Option<MarkerDeleteHandler>,
    selected: Option<u64>,
    last_fingerprint: Option<u64>,
    focus_handle: FocusHandle,
    _document_observe: Option<Subscription>,
}

impl<D: MarkersData + 'static> MarkersPanel<D> {
    /// Create a panel with the given dock tab title.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("delete", DeleteSelectedMarker, Some(CONTEXT)),
            KeyBinding::new("backspace", DeleteSelectedMarker, Some(CONTEXT)),
        ]);
        Self {
            title: title.into(),
            document: None,
            on_select: None,
            on_delete: None,
            selected: None,
            last_fingerprint: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
        }
    }

    /// Bind a data source and select/delete callbacks.
    pub fn set_target(
        &mut self,
        document: Entity<D>,
        on_select: MarkerSelectHandler,
        on_delete: MarkerDeleteHandler,
        cx: &mut Context<Self>,
    ) {
        self.document = Some(document);
        self.on_select = Some(on_select);
        self.on_delete = Some(on_delete);
        self.selected = None;
        self.last_fingerprint = self.document.as_ref().map(|doc| doc.read(cx).fingerprint());
        if let Some(document) = &self.document {
            self._document_observe = Some(cx.observe(document, |this, _, cx| {
                let next = this.document.as_ref().map(|doc| doc.read(cx).fingerprint());
                if this.last_fingerprint == next {
                    return;
                }
                this.last_fingerprint = next;
                cx.notify();
            }));
        }
        cx.notify();
    }

    /// Clear the bound document and selection.
    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.document = None;
        self.on_select = None;
        self.on_delete = None;
        self.selected = None;
        self.last_fingerprint = None;
        self._document_observe = None;
        cx.notify();
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let id = self.selected.or_else(|| {
            document
                .read(cx)
                .snapshot()
                .into_iter()
                .find(|row| row.caret_highlight)
                .map(|row| row.id)
        });
        let Some(id) = id else {
            return;
        };
        if let Some(on_delete) = self.on_delete.clone() {
            on_delete(id, window, cx);
        }
        self.selected = None;
        cx.notify();
    }
}

impl<D: MarkersData + 'static> EventEmitter<PanelEvent> for MarkersPanel<D> {}

impl<D: MarkersData + 'static> Focusable for MarkersPanel<D> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: MarkersData + 'static> BasePanel for MarkersPanel<D> {
    fn panel_name(&self) -> &'static str {
        "MarkersPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl<D: MarkersData + 'static> Panel for MarkersPanel<D> {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl<D: MarkersData + 'static> Render for MarkersPanel<D> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(document) = self.document.clone() else {
            return v_flex()
                .id("markers-list")
                .key_context(CONTEXT)
                .track_focus(&self.focus_handle)
                .size_full()
                .into_any_element();
        };
        let selected = self.selected;
        let rows: Rc<Vec<MarkerRow>> = Rc::new(
            document
                .read(cx)
                .snapshot()
                .into_iter()
                .map(|mut row| {
                    if selected == Some(row.id) {
                        row.caret_highlight = true;
                    }
                    row
                })
                .collect(),
        );
        let count = rows.len();
        let entity = cx.entity();

        v_flex()
            .id("markers-list")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &DeleteSelectedMarker, window, cx| {
                this.delete_selected(window, cx);
            }))
            .size_full()
            .child(
                uniform_list("markers-rows", count, {
                    let rows = rows.clone();
                    let entity = entity.clone();
                    let theme = RowTheme {
                        accent: theme.accent,
                        border: theme.border,
                        secondary: theme.secondary,
                        secondary_hover: theme.secondary_hover,
                        foreground: theme.foreground,
                        muted_foreground: theme.muted_foreground,
                    };
                    move |range, _, _cx| {
                        range
                            .map(|ix| marker_row_element(&entity, &rows[ix], &theme))
                            .collect()
                    }
                })
                .flex_1()
                .size_full(),
            )
            .into_any_element()
    }
}

struct RowTheme {
    accent: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    secondary_hover: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    muted_foreground: gpui_kit::Hsla,
}

fn marker_row_element<D: MarkersData + 'static>(
    entity: &Entity<MarkersPanel<D>>,
    row: &MarkerRow,
    theme: &RowTheme,
) -> impl IntoElement {
    let swatch: gpui_kit::Hsla = Rgba {
        r: row.color[0],
        g: row.color[1],
        b: row.color[2],
        a: row.color[3],
    }
    .into();
    let id = row.id;
    let frame = row.frame;
    let entity = entity.clone();
    let mut label = row.kind.clone();
    if !row.note.is_empty() {
        label.push_str("  ");
        label.push_str(&row.note);
    }
    h_flex()
        .id(("marker-row", id))
        .w_full()
        .flex_none()
        .items_center()
        .gap_1p5()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.))
        .border_1()
        .border_color(if row.caret_highlight {
            theme.accent
        } else {
            theme.border
        })
        .bg(if row.caret_highlight {
            theme.accent.opacity(0.25)
        } else {
            theme.secondary
        })
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary_hover))
        .on_click(move |_: &ClickEvent, window, cx| {
            entity.update(cx, |this, cx| {
                this.selected = Some(id);
                this.focus_handle.focus(window, cx);
                if let Some(on_select) = this.on_select.clone() {
                    on_select(id, frame, window, cx);
                }
                cx.notify();
            });
        })
        .child(
            div()
                .w(px(8.))
                .h(px(8.))
                .rounded(px(2.))
                .flex_none()
                .bg(swatch),
        )
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme.foreground)
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(row.stamp.clone()),
        )
}
