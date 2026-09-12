// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Regions list panel driven by a host [`RegionsData`] provider.

use std::rc::Rc;

use gpui_kit::{
    div, px, uniform_list, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _, StyledExt as _,
};

/// One region row inside a [`RegionGroup`].
#[derive(Clone, Debug)]
pub struct RegionRow {
    /// Stable region id.
    pub id: u64,
    /// Display label (may include channel hints).
    pub label: String,
    /// Start sample frame.
    pub start: usize,
    /// Formatted time / sample stamp.
    pub stamp: String,
}

/// Named collection of regions for the list panel.
#[derive(Clone, Debug)]
pub struct RegionGroup {
    /// Collection name (for example `"selection"`).
    pub collection: String,
    /// Regions in display order.
    pub regions: Vec<RegionRow>,
}

/// Host-provided region list data.
pub trait RegionsData {
    /// Cheap change detector for list refresh.
    fn fingerprint(&self) -> u64;
    /// Snapshot of groups and rows for rendering.
    fn snapshot(&self) -> Vec<RegionGroup>;
}

/// Invoked when the user selects a region row.
///
/// Arguments: `(collection, id, start, window, app)`.
pub type RegionSelectHandler = Rc<dyn Fn(String, u64, usize, &mut Window, &mut App)>;

/// Detail-dock panel listing region collections.
pub struct RegionsPanel<D: RegionsData + 'static> {
    title: SharedString,
    document: Option<Entity<D>>,
    on_select: Option<RegionSelectHandler>,
    selected: Option<(String, u64)>,
    last_fingerprint: Option<u64>,
    focus_handle: FocusHandle,
    _document_observe: Option<Subscription>,
}

impl<D: RegionsData + 'static> RegionsPanel<D> {
    /// Create a panel with the given dock tab title.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            title: title.into(),
            document: None,
            on_select: None,
            selected: None,
            last_fingerprint: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
        }
    }

    /// Bind a data source and selection callback.
    pub fn set_target(
        &mut self,
        document: Entity<D>,
        on_select: RegionSelectHandler,
        cx: &mut Context<Self>,
    ) {
        self.document = Some(document);
        self.on_select = Some(on_select);
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
        self.selected = None;
        self.last_fingerprint = None;
        self._document_observe = None;
        cx.notify();
    }
}

impl<D: RegionsData + 'static> EventEmitter<PanelEvent> for RegionsPanel<D> {}

impl<D: RegionsData + 'static> Focusable for RegionsPanel<D> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: RegionsData + 'static> BasePanel for RegionsPanel<D> {
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

impl<D: RegionsData + 'static> Panel for RegionsPanel<D> {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl<D: RegionsData + 'static> Render for RegionsPanel<D> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(document) = self.document.clone() else {
            return v_flex()
                .id("regions-list")
                .track_focus(&self.focus_handle)
                .size_full()
                .into_any_element();
        };
        let selected = self.selected.clone();
        let mut rows = Vec::new();
        for group in document.read(cx).snapshot() {
            rows.push(ListRow::Header {
                name: group.collection.clone(),
            });
            for region in group.regions {
                let selected = selected
                    .as_ref()
                    .is_some_and(|(col, id)| col == &group.collection && *id == region.id);
                rows.push(ListRow::Region {
                    collection: group.collection.clone(),
                    id: region.id,
                    label: region.label,
                    stamp: region.stamp,
                    start: region.start,
                    selected,
                });
            }
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

enum ListRow {
    Header {
        name: String,
    },
    Region {
        collection: String,
        id: u64,
        label: String,
        stamp: String,
        start: usize,
        selected: bool,
    },
}

struct RowTheme {
    accent: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    secondary_hover: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    muted_foreground: gpui_kit::Hsla,
}

fn region_row_element<D: RegionsData + 'static>(
    entity: &Entity<RegionsPanel<D>>,
    row: &ListRow,
    theme: &RowTheme,
) -> impl IntoElement {
    match row {
        ListRow::Header { name } => div()
            .id(SharedString::from(format!("region-header-{name}")))
            .w_full()
            .px_1p5()
            .py_1()
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(name.clone())
            .into_any_element(),
        ListRow::Region {
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
                .id(("region-row", id))
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
                        if let Some(on_select) = this.on_select.clone() {
                            on_select(collection.clone(), id, start, window, cx);
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
