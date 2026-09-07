// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gpui::{
    actions, div, prelude::FluentBuilder as _, px, App, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    ActiveTheme as _, IconName, Sizable as _,
};

use crate::model::DocumentId;

actions!(explorer, [ConfirmSelected, SelectPrev, SelectNext]);

const CONTEXT: &str = "Compositions";

#[derive(Clone, Copy, Debug)]
pub enum ExplorerEvent {
    Activate(DocumentId),
    OpenTab(DocumentId),
    Close(DocumentId),
}

#[derive(Clone, PartialEq)]
struct ExplorerItem {
    id: DocumentId,
    name: SharedString,
    modified: bool,
}

type EventHandler = Rc<dyn Fn(ExplorerEvent, &mut Window, &mut App)>;

pub struct ExplorerPanel {
    items: Vec<ExplorerItem>,
    active: Option<DocumentId>,
    selected: Option<DocumentId>,
    hovered_close: Option<DocumentId>,
    on_event: Option<EventHandler>,
    focus_handle: FocusHandle,
}

impl ExplorerPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("enter", ConfirmSelected, Some(CONTEXT)),
            KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
            KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        ]);
        Self {
            items: Vec::new(),
            active: None,
            selected: None,
            hovered_close: None,
            on_event: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_handler(&mut self, handler: EventHandler) {
        self.on_event = Some(handler);
    }

    fn handler(&self) -> Option<EventHandler> {
        self.on_event.clone()
    }

    pub fn set_documents(
        &mut self,
        docs: &[(DocumentId, SharedString, bool)],
        active: Option<DocumentId>,
        cx: &mut Context<Self>,
    ) {
        let items: Vec<ExplorerItem> = docs
            .iter()
            .map(|(id, name, modified)| ExplorerItem {
                id: *id,
                name: name.clone(),
                modified: *modified,
            })
            .collect();
        if items == self.items && active == self.active {
            return;
        }
        let keep = self
            .selected
            .filter(|id| items.iter().any(|item| item.id == *id));
        self.items = items;
        self.active = active;
        self.selected = keep.or(active);
        cx.notify();
    }

    fn selected_or_active(&self) -> Option<DocumentId> {
        self.selected.or(self.active)
    }

    fn select_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        let current = self.selected.or(self.active);
        let ix = current
            .and_then(|id| self.items.iter().position(|item| item.id == id))
            .unwrap_or(0);
        let len = self.items.len() as isize;
        let next = (ix as isize + delta).rem_euclid(len) as usize;
        self.selected = Some(self.items[next].id);
        cx.notify();
    }
}

/// Call the host without holding an `ExplorerPanel` lease. `AppView`
/// refreshes this panel, so invoking the handler from `explorer.update`
/// panics.
fn dispatch(
    explorer: &Entity<ExplorerPanel>,
    event: ExplorerEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(handler) = explorer.read(cx).handler() {
        handler(event, window, cx);
    }
}

impl EventEmitter<PanelEvent> for ExplorerPanel {}

impl Focusable for ExplorerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ExplorerPanel {
    fn panel_name(&self) -> &'static str {
        "ExplorerPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ExplorerPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Compositions"
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("Compositions".into())
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ExplorerPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let explorer = cx.entity();
        let hovered_close = self.hovered_close;
        let selected = self.selected;
        let selected_bg = cx.theme().foreground.opacity(0.05);
        let hover_bg = cx.theme().foreground.opacity(0.03);
        let radius = cx.theme().radius;

        div()
            .id("compositions-list")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .on_action({
                let explorer = explorer.clone();
                move |_: &ConfirmSelected, window, cx| {
                    let Some(id) = explorer.read(cx).selected_or_active() else {
                        return;
                    };
                    explorer.update(cx, |this, cx| {
                        this.selected = Some(id);
                        cx.notify();
                    });
                    dispatch(&explorer, ExplorerEvent::Activate(id), window, cx);
                }
            })
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                this.select_delta(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.select_delta(1, cx);
            }))
            .children(self.items.iter().enumerate().map(|(ix, item)| {
                let id = item.id;
                let name = item.name.clone();
                let is_modified = item.modified;
                let highlighted = selected == Some(id);
                let show_close = hovered_close == Some(id);
                let explorer = explorer.clone();
                h_flex()
                    .id(SharedString::from(format!("composition-{id}")))
                    .w_full()
                    .flex_none()
                    .items_center()
                    .px_1p5()
                    .py_0p5()
                    .rounded(radius)
                    .text_xs()
                    .cursor_pointer()
                    .when(highlighted, |this| this.bg(selected_bg))
                    .when(!highlighted, |this| this.hover(|this| this.bg(hover_bg)))
                    .on_click({
                        let explorer = explorer.clone();
                        move |event: &ClickEvent, window, cx| {
                            explorer.update(cx, |this, cx| {
                                this.selected = Some(id);
                                cx.notify();
                            });
                            let event = if event.click_count() >= 2 {
                                ExplorerEvent::OpenTab(id)
                            } else {
                                ExplorerEvent::Activate(id)
                            };
                            dispatch(&explorer, event, window, cx);
                        }
                    })
                    .context_menu({
                        let explorer = explorer.clone();
                        move |menu, _, _| {
                            menu.item(PopupMenuItem::new("Close").on_click({
                                let explorer = explorer.clone();
                                move |_, window, cx| {
                                    dispatch(&explorer, ExplorerEvent::Close(id), window, cx);
                                }
                            }))
                        }
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .child(name),
                    )
                    .child({
                        let explorer = explorer.clone();
                        div()
                            .id(("comp-eol", ix))
                            .w(px(18.))
                            .h(px(18.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .on_hover({
                                let explorer = explorer.clone();
                                move |hovered: &bool, _, cx| {
                                    explorer.update(cx, |this, cx| {
                                        this.hovered_close = if *hovered { Some(id) } else { None };
                                        cx.notify();
                                    });
                                }
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_click({
                                let explorer = explorer.clone();
                                move |_, window, cx| {
                                    cx.stop_propagation();
                                    dispatch(&explorer, ExplorerEvent::Close(id), window, cx);
                                }
                            })
                            .when(show_close, |this| {
                                this.child(
                                    Button::new(("close-comp", ix))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .tab_stop(false)
                                        .on_click({
                                            let explorer = explorer.clone();
                                            move |_, window, cx| {
                                                cx.stop_propagation();
                                                dispatch(
                                                    &explorer,
                                                    ExplorerEvent::Close(id),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                )
                            })
                            .when(!show_close && is_modified, |this| {
                                this.child(
                                    div()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(cx.theme().muted_foreground),
                                )
                            })
                    })
            }))
    }
}
