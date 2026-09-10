// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use gpui_component::{
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _, StyledExt as _,
};

use crate::components::waveform::WaveformDisplay;
use crate::model::buffer::{ChannelScope, RegionId};
use crate::model::document::BufferDocument;
use crate::model::regions::SELECTION_COLLECTION;

pub struct RegionsPanel {
    document: Option<Entity<BufferDocument>>,
    waveform: Option<Entity<WaveformDisplay>>,
    selected: Option<(String, RegionId)>,
    last_fingerprint: Option<(usize, u64, usize)>,
    focus_handle: FocusHandle,
    _document_observe: Option<Subscription>,
}

impl RegionsPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            document: None,
            waveform: None,
            selected: None,
            last_fingerprint: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
        }
    }

    fn fingerprint(&self, cx: &App) -> Option<(usize, u64, usize)> {
        let document = self.document.as_ref()?;
        let doc = document.read(cx);
        let named = doc.composition.read().unwrap();
        Some((
            doc.selection.regions.len(),
            named
                .collections()
                .iter()
                .map(|col| col.regions.len() as u64)
                .sum(),
            named.collections().len(),
        ))
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
        self.last_fingerprint = self.fingerprint(cx);
        if let Some(document) = &self.document {
            self._document_observe = Some(cx.observe(document, |this, _, cx| {
                let next = this.fingerprint(cx);
                if this.last_fingerprint == next {
                    return;
                }
                this.last_fingerprint = next;
                cx.notify();
            }));
        }
        cx.notify();
    }

    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.document = None;
        self.waveform = None;
        self.selected = None;
        self.last_fingerprint = None;
        self._document_observe = None;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for RegionsPanel {}

impl Focusable for RegionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for RegionsPanel {
    fn panel_name(&self) -> &'static str {
        "RegionsPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for RegionsPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::components::dock_skin::DETAIL_TAB_REGIONS
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for RegionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(document) = self.document.clone() else {
            return v_flex()
                .id("regions-list")
                .track_focus(&self.focus_handle)
                .size_full()
                .into_any_element();
        };
        let doc = document.read(cx);
        let sample_rate = doc.sample_rate();
        let selected = self.selected.clone();
        let mut rows = Vec::new();
        push_collection_rows(
            &mut rows,
            SELECTION_COLLECTION,
            &doc.selection.regions,
            sample_rate,
            selected.as_ref(),
        );
        for collection in doc.composition.read().unwrap().collections() {
            push_collection_rows(
                &mut rows,
                &collection.name,
                &collection.regions,
                sample_rate,
                selected.as_ref(),
            );
        }
        let rows = Rc::new(rows);
        let count = rows.len();
        let entity = cx.entity();

        v_flex()
            .id("regions-list")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                uniform_list("regions-rows", count, {
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
                            .map(|ix| region_row_element(&entity, &rows[ix], &theme))
                            .collect()
                    }
                })
                .flex_1()
                .size_full(),
            )
            .into_any_element()
    }
}

enum RegionRow {
    Header {
        name: String,
    },
    Region {
        collection: String,
        id: RegionId,
        label: String,
        stamp: String,
        start: usize,
        selected: bool,
    },
}

struct RowTheme {
    accent: gpui::Hsla,
    border: gpui::Hsla,
    secondary: gpui::Hsla,
    secondary_hover: gpui::Hsla,
    foreground: gpui::Hsla,
    muted_foreground: gpui::Hsla,
}

fn push_collection_rows(
    rows: &mut Vec<RegionRow>,
    name: &str,
    regions: &[crate::model::Region],
    sample_rate: u32,
    selected: Option<&(String, RegionId)>,
) {
    rows.push(RegionRow::Header {
        name: name.to_string(),
    });
    for region in regions {
        let mut label = region
            .label
            .clone()
            .unwrap_or_else(|| format!("{}–{}", region.start, region.end));
        match &region.channels {
            ChannelScope::AllChannels => {}
            ChannelScope::Channels(channels) => {
                label.push_str("  ch ");
                label.push_str(
                    &channels
                        .iter()
                        .map(|ch| ch.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
        }
        rows.push(RegionRow::Region {
            collection: name.to_string(),
            id: region.id,
            label,
            stamp: format_stamp(region.start as u64, sample_rate),
            start: region.start,
            selected: selected.is_some_and(|(col, id)| col == name && *id == region.id),
        });
    }
}

fn region_row_element(
    entity: &Entity<RegionsPanel>,
    row: &RegionRow,
    theme: &RowTheme,
) -> impl IntoElement {
    match row {
        RegionRow::Header { name } => div()
            .id(SharedString::from(format!("region-header-{name}")))
            .w_full()
            .px_1p5()
            .py_1()
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(name.clone())
            .into_any_element(),
        RegionRow::Region {
            collection,
            id,
            label,
            stamp,
            start,
            selected,
        } => {
            let collection = collection.clone();
            let id = *id;
            let start = *start;
            let entity = entity.clone();
            h_flex()
                .id(("region-row", id.0))
                .w_full()
                .flex_none()
                .items_center()
                .gap_1p5()
                .px_1p5()
                .py_0p5()
                .rounded(px(4.))
                .border_1()
                .border_color(if *selected {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(if *selected {
                    theme.accent.opacity(0.25)
                } else {
                    theme.secondary
                })
                .cursor_pointer()
                .hover(|this| this.bg(theme.secondary_hover))
                .on_click(move |_: &ClickEvent, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.selected = Some((collection.clone(), id));
                        this.focus_handle.focus(window, cx);
                        let Some(document) = this.document.as_ref() else {
                            return;
                        };
                        document.update(cx, |doc, cx| {
                            doc.set_position(start, ChannelScope::all());
                            cx.notify();
                        });
                        if let Some(waveform) = this.waveform.as_ref() {
                            waveform.update(cx, |view, cx| {
                                view.scroll_sample_into_view(start as f64, cx);
                            });
                        }
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child(label.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(stamp.clone()),
                )
                .into_any_element()
        }
    }
}

fn format_stamp(frame: u64, sample_rate: u32) -> String {
    let secs = frame as f64 / f64::from(sample_rate.max(1));
    format!("{secs:.2}s · {frame} smp")
}
