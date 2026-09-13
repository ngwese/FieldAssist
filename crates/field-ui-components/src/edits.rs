// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Edit-history list panel driven by a host [`EditsData`] provider.

use std::rc::Rc;

use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    v_flex, ActiveTheme as _, StyledExt as _,
};
use gpui_kit::{
    div, px, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

/// One edit-history card.
#[derive(Clone, Debug)]
pub struct EditCard {
    /// Stable edit id.
    pub id: u64,
    /// Short operation title.
    pub title: String,
    /// Detail line (range, length, etc.).
    pub detail: String,
    /// Whether this edit is the current history tip.
    pub is_current: bool,
    /// Whether this edit is after the current tip (undone future).
    pub is_future: bool,
}

/// Host-provided edit-history data.
pub trait EditsData {
    /// Cheap change detector for list refresh.
    fn fingerprint(&self) -> u64;
    /// Snapshot of edit cards (newest first).
    fn snapshot(&self) -> Vec<EditCard>;
}

/// Invoked when the pointer enters or leaves an edit card.
pub type EditHoverHandler = Rc<dyn Fn(Option<u64>, &mut Window, &mut App)>;

/// Invoked on a single click (scroll / preview).
pub type EditClickHandler = Rc<dyn Fn(u64, &mut Window, &mut App)>;

/// Invoked on a double-click (jump / activate).
pub type EditActivateHandler = Rc<dyn Fn(u64, &mut Window, &mut App)>;

/// Detail-dock panel listing edit history.
pub struct EditsPanel<D: EditsData + 'static> {
    title: SharedString,
    document: Option<Entity<D>>,
    on_hover: Option<EditHoverHandler>,
    on_click: Option<EditClickHandler>,
    on_activate: Option<EditActivateHandler>,
    last_fingerprint: Option<u64>,
    focus_handle: FocusHandle,
    _document_observe: Option<Subscription>,
}

impl<D: EditsData + 'static> EditsPanel<D> {
    /// Create a panel with the given dock tab title.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            title: title.into(),
            document: None,
            on_hover: None,
            on_click: None,
            on_activate: None,
            last_fingerprint: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
        }
    }

    /// Bind a data source and interaction callbacks.
    pub fn set_target(
        &mut self,
        document: Entity<D>,
        on_hover: EditHoverHandler,
        on_click: EditClickHandler,
        on_activate: EditActivateHandler,
        cx: &mut Context<Self>,
    ) {
        self.document = Some(document);
        self.on_hover = Some(on_hover);
        self.on_click = Some(on_click);
        self.on_activate = Some(on_activate);
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

    /// Clear the bound document.
    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.document = None;
        self.on_hover = None;
        self.on_click = None;
        self.on_activate = None;
        self.last_fingerprint = None;
        self._document_observe = None;
        cx.notify();
    }
}

impl<D: EditsData + 'static> EventEmitter<PanelEvent> for EditsPanel<D> {}

impl<D: EditsData + 'static> Focusable for EditsPanel<D> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: EditsData + 'static> BasePanel for EditsPanel<D> {
    fn panel_name(&self) -> &'static str {
        "EditsPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl<D: EditsData + 'static> Panel for EditsPanel<D> {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl<D: EditsData + 'static> Render for EditsPanel<D> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(document) = self.document.clone() else {
            return v_flex().id("edits-list").size_full().into_any_element();
        };
        let cards = document.read(cx).snapshot();

        v_flex()
            .id("edits-list")
            .size_full()
            .gap_1()
            .overflow_y_scroll()
            .children(cards.into_iter().map(|card| {
                let id = card.id;
                v_flex()
                    .id(("edit-card", id))
                    .w_full()
                    .flex_none()
                    .gap_0()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if card.is_current {
                        theme.accent
                    } else {
                        theme.border
                    })
                    .bg(if card.is_current {
                        theme.accent.opacity(0.25)
                    } else {
                        theme.secondary
                    })
                    .opacity(if card.is_future { 0.55 } else { 1.0 })
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.secondary_hover))
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        let Some(on_hover) = this.on_hover.clone() else {
                            return;
                        };
                        on_hover(if *hovered { Some(id) } else { None }, window, cx);
                    }))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            if let Some(on_activate) = this.on_activate.clone() {
                                on_activate(id, window, cx);
                            }
                        } else if let Some(on_click) = this.on_click.clone() {
                            on_click(id, window, cx);
                        }
                    }))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child(card.title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(card.detail),
                    )
            }))
            .into_any_element()
    }
}
