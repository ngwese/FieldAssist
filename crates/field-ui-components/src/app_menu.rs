// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui_kit::{
    anchored, deferred, div, prelude::FluentBuilder as _, px, transparent_white, App,
    AppContext as _, ClickEvent, Context, DismissEvent, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, OwnedMenu, OwnedMenuItem,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    kbd::Kbd,
    menu::{PopupMenu, PopupMenuItem},
    ActiveTheme as _, GlobalState, InteractiveElementExt as _, Selectable as _, Sizable as _,
};

/// Application menu bar for Windows and Linux, painted in muted chrome colors.
pub struct AppMenuBar {
    menus: Vec<Entity<AppMenu>>,
    selected_index: Option<usize>,
    action_context: Option<FocusHandle>,
}

impl AppMenuBar {
    /// `new`.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let mut this = Self {
                selected_index: None,
                action_context: None,
                menus: Vec::new(),
            };
            this.reload(cx);
            this
        })
    }

    /// `reload`.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let menu_bar = cx.entity();
        let menus: Vec<OwnedMenu> = GlobalState::global(cx)
            .app_menus()
            .iter()
            .cloned()
            .collect();
        self.menus = menus
            .iter()
            .enumerate()
            .map(|(ix, menu)| AppMenu::new(ix, menu, menu_bar.clone(), cx))
            .collect();
        self.selected_index = None;
        self.action_context = None;
        cx.notify();
    }

    fn set_selected_index(
        &mut self,
        ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_index.is_none() && ix.is_some() {
            self.action_context = window.focused(cx);
        } else if ix.is_none() {
            if let Some(action_context) = self.action_context.as_ref() {
                action_context.focus(window, cx);
            }
            self.action_context = None;
        }

        self.selected_index = ix;
        cx.notify();
    }

    fn has_activated_menu(&self) -> bool {
        self.selected_index.is_some()
    }
}

impl Render for AppMenuBar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("app-menu-bar")
            .role(Role::MenuBar)
            .size_full()
            .gap_x_1()
            .overflow_x_scroll()
            .lock_scroll_axis()
            .children(self.menus.clone())
    }
}

struct AppMenu {
    menu_bar: Entity<AppMenuBar>,
    ix: usize,
    name: SharedString,
    menu: OwnedMenu,
    popup_menu: Option<Entity<PopupMenu>>,
    _subscription: Option<Subscription>,
}

impl AppMenu {
    fn new(
        ix: usize,
        menu: &OwnedMenu,
        menu_bar: Entity<AppMenuBar>,
        cx: &mut App,
    ) -> Entity<Self> {
        let name = menu.name.clone();
        cx.new(|_| Self {
            ix,
            menu_bar,
            name,
            menu: menu.clone(),
            popup_menu: None,
            _subscription: None,
        })
    }

    fn is_selected(&self, cx: &App) -> bool {
        self.menu_bar.read(cx).selected_index == Some(self.ix)
    }

    fn build_popup_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<PopupMenu> {
        let action_context = self.menu_bar.read(cx).action_context.clone();
        let popup_menu = match self.popup_menu.as_ref() {
            None => {
                let items = self.menu.items.clone();
                let popup_menu = PopupMenu::build(window, cx, |menu, window, cx| {
                    append_muted_items(menu, &items, action_context.clone(), window, cx)
                });
                self._subscription =
                    Some(cx.subscribe_in(&popup_menu, window, Self::handle_dismiss));
                self.popup_menu = Some(popup_menu.clone());
                popup_menu
            }
            Some(menu) => menu.clone(),
        };

        let focus_handle = popup_menu.read(cx).focus_handle(cx);
        if !focus_handle.contains_focused(window, cx) {
            focus_handle.focus(window, cx);
        }

        popup_menu
    }

    fn handle_dismiss(
        &mut self,
        _: &Entity<PopupMenu>,
        _: &DismissEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._subscription.take();
        self.popup_menu.take();
        self.menu_bar.update(cx, |state, cx| {
            state.set_selected_index(None, window, cx);
        });
    }

    fn handle_trigger_click(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, ClickEvent::Mouse(_)) {
            return;
        }
        self.toggle(window, cx);
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_selected = self.is_selected(cx);
        _ = self.menu_bar.update(cx, |state, cx| {
            let new_ix = if is_selected { None } else { Some(self.ix) };
            state.set_selected_index(new_ix, window, cx);
        });
    }

    fn handle_hover(&mut self, hovered: &bool, window: &mut Window, cx: &mut Context<Self>) {
        if !*hovered {
            return;
        }
        if !self.menu_bar.read(cx).has_activated_menu() {
            return;
        }
        _ = self.menu_bar.update(cx, |state, cx| {
            state.set_selected_index(Some(self.ix), window, cx);
        });
    }
}

impl Render for AppMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_selected = self.is_selected(cx);
        let muted = cx.theme().muted_foreground;

        div()
            .id(self.ix)
            .relative()
            .child(
                Button::new("menu")
                    .small()
                    .py_0p5()
                    .compact()
                    .ghost()
                    .text_color(muted)
                    .label(self.name.clone())
                    .selected(is_selected)
                    .on_mouse_down(
                        MouseButton::Left,
                        window.listener_for(&cx.entity(), move |this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle(window, cx);
                        }),
                    )
                    .on_click(cx.listener(Self::handle_trigger_click)),
            )
            .on_hover(cx.listener(Self::handle_hover))
            .when(is_selected, |this| {
                this.child(deferred(
                    anchored()
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .size_full()
                                .occlude()
                                .top_1()
                                .child(self.build_popup_menu(window, cx)),
                        ),
                ))
            })
    }
}

fn append_muted_items(
    mut menu: PopupMenu,
    items: &[OwnedMenuItem],
    action_context: Option<FocusHandle>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    if let Some(handle) = action_context.clone() {
        menu = menu.action_context(handle);
    }
    let muted = cx.theme().muted_foreground;
    for item in items {
        menu = match item {
            OwnedMenuItem::Separator => menu.separator(),
            OwnedMenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => {
                let label = name.clone();
                let action = action.boxed_clone();
                let key_action = action.boxed_clone();
                let key_context = action_context.clone();
                menu.item(
                    PopupMenuItem::element(move |window, _| {
                        let key = key_context
                            .as_ref()
                            .and_then(|handle| {
                                Kbd::binding_for_action_in(key_action.as_ref(), handle, window)
                            })
                            .or_else(|| Kbd::binding_for_action(key_action.as_ref(), None, window))
                            .map(|kbd| kbd.p_0().flex_nowrap().border_0().bg(transparent_white()));
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .justify_between()
                            .child(div().text_color(muted).child(label.clone()))
                            .children(key)
                    })
                    .checked(*checked)
                    .disabled(*disabled)
                    .action(action),
                )
            }
            OwnedMenuItem::Submenu(submenu) => {
                let name = submenu.name.clone();
                let nested = submenu.items.clone();
                let ctx = action_context.clone();
                menu.submenu(name, window, cx, move |menu, window, cx| {
                    append_muted_items(menu, &nested, ctx.clone(), window, cx)
                })
            }
            OwnedMenuItem::SystemMenu(_) => menu,
        };
    }
    menu
}
