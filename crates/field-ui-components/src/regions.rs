// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Regions list panel driven by a host [`RegionsData`] provider.

use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex, v_flex, ActiveTheme as _, Icon, IconName, IconNamed, Sizable as _, StyledExt as _,
};
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, App, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

/// Width of the xsmall region-card icon column (matches `Icon::xsmall` / `size_3`).
const REGION_ICON_SLOT: f32 = 12.;
const CLOSE_HIT: f32 = 18.;

struct RegionStartIcon;

impl IconNamed for RegionStartIcon {
    fn path(self) -> SharedString {
        "icons/arrow-right-from-line.svg".into()
    }
}

/// One region row inside a [`RegionGroup`].
#[derive(Clone, Debug)]
pub struct RegionRow {
    /// Stable region id.
    pub id: u64,
    /// Start sample frame (inclusive).
    pub start: usize,
    /// End sample frame (inclusive).
    pub end: usize,
    /// First line: start/end in seconds.
    pub time_line: String,
    /// Second line: start–end samples and length.
    pub sample_line: String,
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

/// Invoked when the user deletes a region row.
///
/// Arguments: `(collection, id, window, app)`.
pub type RegionDeleteHandler = Rc<dyn Fn(String, u64, &mut Window, &mut App)>;

/// Detail-dock panel listing region collections.
pub struct RegionsPanel<D: RegionsData + 'static> {
    title: SharedString,
    document: Option<Entity<D>>,
    on_select: Option<RegionSelectHandler>,
    on_delete: Option<RegionDeleteHandler>,
    selected: Option<(String, u64)>,
    hovered_close: Option<(String, u64)>,
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
            on_delete: None,
            selected: None,
            hovered_close: None,
            last_fingerprint: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
        }
    }

    /// Bind a data source and selection/delete callbacks.
    pub fn set_target(
        &mut self,
        document: Entity<D>,
        on_select: RegionSelectHandler,
        on_delete: RegionDeleteHandler,
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
        let hovered_close = self.hovered_close.clone();
        let groups = document.read(cx).snapshot();

        let mut children: Vec<gpui_kit::AnyElement> = Vec::new();
        for group in groups {
            children.push(
                div()
                    .id(SharedString::from(format!(
                        "region-header-{}",
                        group.collection
                    )))
                    .w_full()
                    .flex_none()
                    .px_1p5()
                    .py_1()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(group.collection.clone())
                    .into_any_element(),
            );

            for region in group.regions {
                let is_selected = selected
                    .as_ref()
                    .is_some_and(|(col, id)| col == &group.collection && *id == region.id);
                let show_close = hovered_close
                    .as_ref()
                    .is_some_and(|(col, id)| col == &group.collection && *id == region.id);
                let id = region.id;
                let start = region.start;
                let collection = group.collection.clone();
                let collection_close = collection.clone();
                children.push(
                    v_flex()
                        .id(("region-row", id))
                        .w_full()
                        .flex_none()
                        .gap_0()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if is_selected {
                            theme.accent
                        } else {
                            theme.border
                        })
                        .when(is_selected, |this| this.bg(theme.accent.opacity(0.25)))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.secondary_hover))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.selected = Some((collection.clone(), id));
                            this.focus_handle.focus(window, cx);
                            if let Some(on_select) = this.on_select.clone() {
                                on_select(collection.clone(), id, start, window, cx);
                            }
                            cx.notify();
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    Icon::new(RegionStartIcon)
                                        .xsmall()
                                        .flex_shrink_0()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .font_semibold()
                                        .text_color(theme.foreground)
                                        .child(region.time_line),
                                )
                                .child(
                                    div()
                                        .id(("region-close", id))
                                        .w(px(CLOSE_HIT))
                                        .h(px(CLOSE_HIT))
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .on_hover(cx.listener({
                                            let collection = collection_close.clone();
                                            move |this, hovered: &bool, _, cx| {
                                                this.hovered_close = if *hovered {
                                                    Some((collection.clone(), id))
                                                } else {
                                                    None
                                                };
                                                cx.notify();
                                            }
                                        }))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation();
                                        })
                                        .on_click(cx.listener({
                                            let collection = collection_close.clone();
                                            move |this, _: &ClickEvent, window, cx| {
                                                cx.stop_propagation();
                                                if let Some(on_delete) = this.on_delete.clone() {
                                                    on_delete(collection.clone(), id, window, cx);
                                                }
                                                if this.selected.as_ref().is_some_and(
                                                    |(col, sid)| col == &collection && *sid == id,
                                                ) {
                                                    this.selected = None;
                                                }
                                                this.hovered_close = None;
                                                cx.notify();
                                            }
                                        }))
                                        .when(show_close, |this| {
                                            this.child(
                                                Button::new(SharedString::from(format!(
                                                    "close-region-{id}"
                                                )))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::Close)
                                                .tab_stop(false)
                                                .on_click(cx.listener({
                                                    let collection = collection_close.clone();
                                                    move |this, _: &ClickEvent, window, cx| {
                                                        cx.stop_propagation();
                                                        if let Some(on_delete) =
                                                            this.on_delete.clone()
                                                        {
                                                            on_delete(
                                                                collection.clone(),
                                                                id,
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                        if this.selected.as_ref().is_some_and(
                                                            |(col, sid)| {
                                                                col == &collection && *sid == id
                                                            },
                                                        ) {
                                                            this.selected = None;
                                                        }
                                                        this.hovered_close = None;
                                                        cx.notify();
                                                    }
                                                })),
                                            )
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .pl(px(REGION_ICON_SLOT + 6.))
                                .pr(px(CLOSE_HIT + 6.))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(region.sample_line),
                        )
                        .into_any_element(),
                );
            }
        }

        v_flex()
            .id("regions-list")
            .track_focus(&self.focus_handle)
            .size_full()
            .gap_1()
            .overflow_y_scroll()
            .children(children)
            .into_any_element()
    }
}
