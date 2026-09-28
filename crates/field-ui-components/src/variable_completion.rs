// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `${…}` variable-reference completion for single-line template inputs.

use field_variables::{
    accept_edit, completion_context, completion_items, prefix_edit, tab_action, CompletionContext,
    CompletionItem, CompletionKind, TabAction, VariableTable,
};
use gpui_kit::component::{
    h_flex,
    input::{Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, OutdentInline},
    ActiveTheme as _, Icon, IconName, Sizable as _,
};
use gpui_kit::{
    actions, anchored, deferred, div, prelude::FluentBuilder as _, px, App, AppContext as _,
    Context, ElementId, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    KeyBinding, MouseButton, ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

actions!(variable_completion, [ConfirmCompletion]);

const MAX_MENU_HEIGHT: Pixels = px(240.);
const MENU_CONTEXT: &str = "VariableCompletion";

/// Completion controller attached to a single-line [`InputState`].
pub struct VariableCompletion {
    input: Entity<InputState>,
    table: VariableTable,
    items: Vec<CompletionItem>,
    highlight: usize,
    /// Popdown visible while the caret is inside `${…}` with matches.
    menu_visible: bool,
    /// Focus has moved from the input into the selection list.
    list_focused: bool,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl VariableCompletion {
    /// Wrap an existing single-line input.
    pub fn attach(input: Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("up", MoveUp, Some(MENU_CONTEXT)),
            KeyBinding::new("down", MoveDown, Some(MENU_CONTEXT)),
            KeyBinding::new("enter", ConfirmCompletion, Some(MENU_CONTEXT)),
            KeyBinding::new("tab", IndentInline, Some(MENU_CONTEXT)),
            KeyBinding::new("escape", Escape, Some(MENU_CONTEXT)),
        ]);

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, _input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.refresh(window, cx);
                }
                InputEvent::Blur => {
                    if this.list_focused {
                        return;
                    }
                    let entity = cx.entity();
                    window.defer(cx, move |window, cx| {
                        entity.update(cx, |this, cx| {
                            if this.list_focused {
                                return;
                            }
                            if this.focus_handle.contains_focused(window, cx) {
                                return;
                            }
                            this.dismiss(window, cx);
                        });
                    });
                }
                _ => {}
            },
        ));

        Self {
            input,
            table: VariableTable::new(),
            items: Vec::new(),
            highlight: 0,
            menu_visible: false,
            list_focused: false,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Create a new input and attach completion to it.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        Self::attach(input, window, cx)
    }

    /// Underlying text field.
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// Replace the candidate variable table and refresh the menu.
    pub fn set_table(&mut self, table: VariableTable, window: &mut Window, cx: &mut Context<Self>) {
        self.table = table;
        self.refresh(window, cx);
    }

    /// Whether the completion popdown is visible.
    pub fn menu_open(&self) -> bool {
        self.menu_visible && !self.items.is_empty()
    }

    /// Hide the menu and clear list focus (input keeps its text).
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_list = self.list_focused;
        self.menu_visible = false;
        self.list_focused = false;
        self.items.clear();
        self.highlight = 0;
        if was_list {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Input + optional completion popdown for host layouts.
    pub fn render_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let menu_open = self.menu_open();
        let list_focused = self.list_focused;
        let items = self.items.clone();
        let highlight = self.highlight;
        let scroll = self.scroll.clone();
        let focus_handle = self.focus_handle.clone();

        div()
            .relative()
            .w_full()
            .on_action({
                let entity = entity.clone();
                move |_: &IndentInline, window, cx| {
                    let handled = entity.update(cx, |this, cx| this.handle_tab(window, cx));
                    if !handled {
                        // Outside an active `${…}` completion, Tab must move
                        // focus — Input's IndentInline would otherwise consume it.
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                move |_: &OutdentInline, window, cx| {
                    window.focus_prev(cx);
                    cx.stop_propagation();
                }
            })
            .on_action({
                let entity = entity.clone();
                move |_: &Escape, window, cx| {
                    let open = entity.read(cx).menu_open();
                    if open {
                        entity.update(cx, |this, cx| {
                            this.dismiss(window, cx);
                        });
                        cx.stop_propagation();
                    }
                }
            })
            .child(Input::new(&self.input).xsmall().w_full())
            .when(menu_open, |el| {
                el.child(deferred(
                    anchored()
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("variable-completion-menu")
                                .occlude()
                                .mt_1()
                                .w_full()
                                .min_w(px(200.))
                                .max_h(MAX_MENU_HEIGHT)
                                .overflow_y_scroll()
                                .track_scroll(&scroll)
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_md()
                                .text_xs()
                                .when(list_focused, |el| {
                                    el.track_focus(&focus_handle)
                                        .key_context(MENU_CONTEXT)
                                        .on_action({
                                            let entity = entity.clone();
                                            move |_: &MoveUp, _window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.move_highlight(-1, cx);
                                                });
                                                cx.stop_propagation();
                                            }
                                        })
                                        .on_action({
                                            let entity = entity.clone();
                                            move |_: &MoveDown, _window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.move_highlight(1, cx);
                                                });
                                                cx.stop_propagation();
                                            }
                                        })
                                        .on_action({
                                            let entity = entity.clone();
                                            move |_: &ConfirmCompletion, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.accept_highlighted(window, cx);
                                                });
                                                cx.stop_propagation();
                                            }
                                        })
                                        .on_action({
                                            let entity = entity.clone();
                                            move |_: &IndentInline, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.accept_highlighted(window, cx);
                                                });
                                                cx.stop_propagation();
                                            }
                                        })
                                        .on_action({
                                            let entity = entity.clone();
                                            move |_: &Escape, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.dismiss(window, cx);
                                                });
                                                cx.stop_propagation();
                                            }
                                        })
                                })
                                .children(items.into_iter().enumerate().map(|(ix, item)| {
                                    let entity = entity.clone();
                                    let selected = ix == highlight;
                                    let is_scope = item.kind == CompletionKind::Scope;
                                    let label = SharedString::from(item.label.clone());
                                    let value = SharedString::from(item.value.clone());
                                    h_flex()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "var-complete-{ix}"
                                        ))))
                                        .w_full()
                                        .gap_2()
                                        .px_2()
                                        .py_0p5()
                                        .items_center()
                                        .when(selected, |row| row.bg(theme.accent.opacity(0.25)))
                                        .hover(|row| row.bg(theme.secondary_hover))
                                        .cursor_pointer()
                                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.highlight = ix;
                                                this.accept_highlighted(window, cx);
                                            });
                                            cx.stop_propagation();
                                        })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(label),
                                        )
                                        .when(is_scope, |row| {
                                            row.child(
                                                Icon::new(IconName::ChevronRight)
                                                    .xsmall()
                                                    .text_color(muted),
                                            )
                                        })
                                        .when(!is_scope && !value.is_empty(), |row| {
                                            row.child(
                                                div()
                                                    .flex_none()
                                                    .max_w(px(140.))
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_color(muted)
                                                    .child(value),
                                            )
                                        })
                                })),
                        ),
                ))
            })
    }

    fn refresh(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (text, caret, has_selection) = {
            let state = self.input.read(cx);
            let range = state.selected_range();
            (
                state.value().to_string(),
                state.cursor(),
                range.start != range.end,
            )
        };
        if has_selection {
            self.menu_visible = false;
            self.items.clear();
            self.list_focused = false;
            cx.notify();
            return;
        }
        let Some(ctx) = completion_context(&text, caret) else {
            self.menu_visible = false;
            self.items.clear();
            self.list_focused = false;
            cx.notify();
            return;
        };
        self.items = completion_items(&ctx, &self.table);
        self.menu_visible = !self.items.is_empty();
        if !self.menu_visible {
            self.list_focused = false;
        }
        if self.highlight >= self.items.len() {
            self.highlight = 0;
        }
        cx.notify();
    }

    /// Handle Tab while the input (not the list) is focused.
    ///
    /// Returns whether the action was consumed.
    fn handle_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.list_focused {
            self.accept_highlighted(window, cx);
            return true;
        }
        let (text, caret, has_selection) = {
            let state = self.input.read(cx);
            let range = state.selected_range();
            (
                state.value().to_string(),
                state.cursor(),
                range.start != range.end,
            )
        };
        if has_selection {
            return false;
        }
        let Some(ctx) = completion_context(&text, caret) else {
            return false;
        };
        let items = completion_items(&ctx, &self.table);
        match tab_action(&items, &ctx.prefix) {
            TabAction::None => false,
            TabAction::Accept(item) => {
                self.apply_accept(&ctx, &text, &item, window, cx);
                true
            }
            TabAction::InsertPrefix(extra) => {
                let edit = prefix_edit(&ctx, &extra);
                let range = ctx.prefix_range();
                self.apply_range_edit(range, &edit.replacement, window, cx);
                true
            }
            TabAction::FocusList => {
                self.items = items;
                self.menu_visible = !self.items.is_empty();
                self.highlight = 0;
                self.list_focused = true;
                self.focus_handle.focus(window, cx);
                cx.notify();
                true
            }
        }
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        let len = self.items.len() as isize;
        let next = (self.highlight as isize + delta).rem_euclid(len) as usize;
        self.highlight = next;
        cx.notify();
    }

    fn accept_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(self.highlight).cloned() else {
            return;
        };
        let (text, caret) = {
            let state = self.input.read(cx);
            (state.value().to_string(), state.cursor())
        };
        let Some(ctx) = completion_context(&text, caret) else {
            self.dismiss(window, cx);
            return;
        };
        self.apply_accept(&ctx, &text, &item, window, cx);
    }

    fn apply_accept(
        &mut self,
        ctx: &CompletionContext,
        text: &str,
        item: &CompletionItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = accept_edit(ctx, text, item);
        let range = ctx.segment_range.clone();
        let was_scope = item.kind == CompletionKind::Scope;
        self.apply_range_edit(range, &edit.replacement, window, cx);
        self.list_focused = false;
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        if !was_scope {
            self.refresh(window, cx);
        }
    }

    fn apply_range_edit(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let replacement = replacement.to_string();
        self.input.update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(replacement, window, cx);
        });
    }
}

impl Focusable for VariableCompletion {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VariableCompletion {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_field(cx)
    }
}
