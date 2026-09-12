// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gpui_kit::{
    actions, div, px, uniform_list, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, Rgba,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _, StyledExt as _,
};

use crate::components::waveform::WaveformDisplay;
use crate::model::buffer::ChannelScope;
use crate::model::composition::MarkerId;
use crate::model::document::BufferDocument;

actions!(markers_panel, [DeleteSelectedMarker]);

const CONTEXT: &str = "Markers";

pub struct MarkersPanel {
    document: Option<Entity<BufferDocument>>,
    waveform: Option<Entity<WaveformDisplay>>,
    selected: Option<MarkerId>,
    last_markers: Option<(u64, usize, Option<u64>)>,
    focus_handle: FocusHandle,
    _document_observe: Option<Subscription>,
}

impl MarkersPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("delete", DeleteSelectedMarker, Some(CONTEXT)),
            KeyBinding::new("backspace", DeleteSelectedMarker, Some(CONTEXT)),
        ]);
        Self {
            document: None,
            waveform: None,
            selected: None,
            last_markers: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
        }
    }

    fn markers_fingerprint(&self, cx: &App) -> Option<(u64, usize, Option<u64>)> {
        let document = self.document.as_ref()?;
        let doc = document.read(cx);
        let caret = doc.current_position.as_ref().map(|pos| pos.sample as u64);
        let composition = doc.composition.read().unwrap();
        let markers = composition.markers();
        let hit = caret.and_then(|frame| markers.get_at(frame).map(|marker| marker.id.0));
        Some((markers.generation(), markers.len(), hit))
    }

    pub fn set_target(
        &mut self,
        document: Entity<BufferDocument>,
        waveform: Entity<WaveformDisplay>,
        cx: &mut Context<Self>,
    ) {
        self.document = Some(document);
        self.waveform = Some(waveform);
        self.selected = None;
        self.last_markers = self.markers_fingerprint(cx);
        if let Some(document) = &self.document {
            self._document_observe = Some(cx.observe(document, |this, _, cx| {
                let next = this.markers_fingerprint(cx);
                if this.last_markers == next {
                    return;
                }
                this.last_markers = next;
                cx.notify();
            }));
        }
        cx.notify();
    }

    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.document = None;
        self.waveform = None;
        self.selected = None;
        self.last_markers = None;
        self._document_observe = None;
        cx.notify();
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let id = self.selected.or_else(|| {
            let doc = document.read(cx);
            let sample = doc.current_position.as_ref().map(|pos| pos.sample as u64)?;
            doc.composition
                .read()
                .unwrap()
                .markers()
                .get_at(sample)
                .map(|marker| marker.id)
        });
        let Some(id) = id else {
            return;
        };
        document.update(cx, |doc, cx| {
            doc.remove_marker(id);
            cx.notify();
        });
        self.selected = None;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for MarkersPanel {}

impl Focusable for MarkersPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for MarkersPanel {
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

impl Panel for MarkersPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::components::dock_skin::DETAIL_TAB_MARKER
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MarkersPanel {
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
        let doc = document.read(cx);
        let caret = doc.current_position.as_ref().map(|pos| pos.sample as u64);
        let sample_rate = doc.sample_rate();
        let selected = self.selected;
        let composition = doc.composition.read().unwrap();
        let rows: Rc<Vec<MarkerRow>> = Rc::new(
            composition
                .markers()
                .iter()
                .map(|marker| MarkerRow {
                    id: marker.id,
                    frame: marker.frame,
                    kind: marker.marker_type.clone(),
                    color: composition.resolved_marker_color(&marker.marker_type),
                    note: marker.note.clone().unwrap_or_default(),
                    stamp: format_stamp(marker.frame, sample_rate),
                    selected: selected == Some(marker.id) || Some(marker.frame) == caret,
                })
                .collect(),
        );
        let count = rows.len();
        let entity = cx.entity();

        v_flex()
            .id("markers-list")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &DeleteSelectedMarker, _, cx| {
                this.delete_selected(cx);
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

struct MarkerRow {
    id: MarkerId,
    frame: u64,
    kind: String,
    color: [f32; 4],
    note: String,
    stamp: String,
    selected: bool,
}

struct RowTheme {
    accent: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    secondary_hover: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    muted_foreground: gpui_kit::Hsla,
}

fn marker_row_element(
    entity: &Entity<MarkersPanel>,
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
        .id(("marker-row", id.0))
        .w_full()
        .flex_none()
        .items_center()
        .gap_1p5()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.))
        .border_1()
        .border_color(if row.selected {
            theme.accent
        } else {
            theme.border
        })
        .bg(if row.selected {
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
                let Some(document) = this.document.as_ref() else {
                    return;
                };
                document.update(cx, |doc, cx| {
                    doc.set_position(frame as usize, ChannelScope::all());
                    cx.notify();
                });
                if let Some(waveform) = this.waveform.as_ref() {
                    waveform.update(cx, |view, cx| {
                        view.scroll_sample_into_view(frame as f64, cx);
                    });
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

fn format_stamp(frame: u64, sample_rate: u32) -> String {
    let secs = frame as f64 / f64::from(sample_rate.max(1));
    format!("{secs:.2}s · {frame} smp")
}
