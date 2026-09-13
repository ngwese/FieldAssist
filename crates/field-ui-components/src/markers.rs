// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Markers list panel driven by a host [`MarkersData`] provider.

use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
};
use gpui_kit::{
    actions, div, prelude::FluentBuilder as _, px, App, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    ParentElement as _, Render, Rgba, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};

#[allow(missing_docs)]
mod marker_actions {
    use super::*;
    actions!(markers_panel, [DeleteSelectedMarker]);
}
/// Deletes the selected or caret-highlighted marker.
pub use marker_actions::DeleteSelectedMarker;

const CONTEXT: &str = "Markers";
const CLOSE_HIT: f32 = 18.;

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
    hovered_close: Option<u64>,
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
            hovered_close: None,
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
        self.hovered_close = None;
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
        self.hovered_close = None;
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
        self.hovered_close = None;
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
        let hovered_close = self.hovered_close;
        let rows: Vec<MarkerRow> = document
            .read(cx)
            .snapshot()
            .into_iter()
            .map(|mut row| {
                if selected == Some(row.id) {
                    row.caret_highlight = true;
                }
                row
            })
            .collect();

        v_flex()
            .id("markers-list")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &DeleteSelectedMarker, window, cx| {
                this.delete_selected(window, cx);
            }))
            .size_full()
            .gap_1()
            .overflow_y_scroll()
            .children(rows.into_iter().map(|row| {
                let swatch: gpui_kit::Hsla = Rgba {
                    r: row.color[0],
                    g: row.color[1],
                    b: row.color[2],
                    a: row.color[3],
                }
                .into();
                let id = row.id;
                let frame = row.frame;
                let mut label = row.kind;
                if !row.note.is_empty() {
                    label.push_str("  ");
                    label.push_str(&row.note);
                }
                let highlighted = row.caret_highlight;
                let show_close = hovered_close == Some(id);
                h_flex()
                    .id(("marker-row", id))
                    .w_full()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(4.))
                    .when(highlighted, |this| this.bg(theme.accent.opacity(0.25)))
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.secondary_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected = Some(id);
                        this.focus_handle.focus(window, cx);
                        if let Some(on_select) = this.on_select.clone() {
                            on_select(id, frame, window, cx);
                        }
                        cx.notify();
                    }))
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
                            .flex_none()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(row.stamp),
                    )
                    .child(div().flex_1().min_w_0())
                    .child(
                        div()
                            .id(("marker-close", id))
                            .w(px(CLOSE_HIT))
                            .h(px(CLOSE_HIT))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                this.hovered_close = if *hovered { Some(id) } else { None };
                                cx.notify();
                            }))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                if let Some(on_delete) = this.on_delete.clone() {
                                    on_delete(id, window, cx);
                                }
                                if this.selected == Some(id) {
                                    this.selected = None;
                                }
                                this.hovered_close = None;
                                cx.notify();
                            }))
                            .when(show_close, |this| {
                                this.child(
                                    Button::new(SharedString::from(format!("close-marker-{id}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .tab_stop(false)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                cx.stop_propagation();
                                                if let Some(on_delete) = this.on_delete.clone() {
                                                    on_delete(id, window, cx);
                                                }
                                                if this.selected == Some(id) {
                                                    this.selected = None;
                                                }
                                                this.hovered_close = None;
                                                cx.notify();
                                            },
                                        )),
                                )
                            }),
                    )
            }))
            .into_any_element()
    }
}
