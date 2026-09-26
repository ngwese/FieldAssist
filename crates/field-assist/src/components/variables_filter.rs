// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Search + scope filter chrome shared by Variables dock and User Variables window.

use std::collections::HashSet;
use std::rc::Rc;

use field_ui_components::VariableRow;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    Icon, IconName, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, Window,
};

/// Name search + enabled scopes for a variables table.
#[derive(Default)]
pub struct VariablesFilter {
    search_input: Option<Entity<InputState>>,
    /// Scopes currently shown. Unknown scopes are enabled when first seen.
    enabled_scopes: HashSet<String>,
    seen_scopes: HashSet<String>,
}

impl VariablesFilter {
    /// Ensure the search field exists and is subscribed for redraws.
    pub fn ensure_search<V: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<V>,
        on_change: impl Fn(&mut V, &mut Context<V>) + 'static,
    ) {
        if self.search_input.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state = state.placeholder("Search names…");
            state
        });
        cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                on_change(this, cx);
            }
        })
        .detach();
        self.search_input = Some(input);
    }

    /// Search string from the input (empty when not ready).
    pub fn query(&self, cx: &App) -> String {
        self.search_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Enable any newly appeared scopes (so new data stays visible by default).
    pub fn sync_scopes_from_rows(&mut self, rows: &[VariableRow]) {
        for row in rows {
            if self.seen_scopes.insert(row.scope.clone()) {
                self.enabled_scopes.insert(row.scope.clone());
            }
        }
    }

    /// Sorted unique scopes present in `rows`.
    pub fn available_scopes(rows: &[VariableRow]) -> Vec<String> {
        let mut scopes: Vec<String> = rows
            .iter()
            .map(|r| r.scope.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        scopes.sort();
        scopes
    }

    /// Toggle one scope in or out of the enabled set.
    pub fn toggle_scope(&mut self, scope: &str) {
        if !self.enabled_scopes.remove(scope) {
            self.enabled_scopes.insert(scope.to_string());
        }
        self.seen_scopes.insert(scope.to_string());
    }

    /// Whether a row passes the current name + scope filters.
    pub fn row_visible(&self, row: &VariableRow, cx: &App) -> bool {
        if !self.enabled_scopes.contains(&row.scope) {
            return false;
        }
        let q = self.query(cx);
        let q = q.trim();
        if q.is_empty() {
            return true;
        }
        row.name.to_lowercase().contains(&q.to_lowercase())
    }

    /// Indices of rows that pass the filter (stable order).
    pub fn filtered_indices(&self, rows: &[VariableRow], cx: &App) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| self.row_visible(row, cx))
            .map(|(ix, _)| ix)
            .collect()
    }

    pub fn search_input(&self) -> Option<&Entity<InputState>> {
        self.search_input.as_ref()
    }
}

/// Build the search field + Scopes dropdown + add/remove controls.
pub fn filter_bar<V: 'static>(
    filter: &VariablesFilter,
    rows: &[VariableRow],
    entity: Entity<V>,
    divider: Hsla,
    on_toggle_scope: impl Fn(&mut V, String, &mut Context<V>) + 'static,
    on_add: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_remove: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> impl IntoElement {
    let available = VariablesFilter::available_scopes(rows);
    let enabled_snapshot = filter.enabled_scopes.clone();
    let search = filter.search_input().cloned();
    let on_toggle_scope = Rc::new(on_toggle_scope);
    let on_add = Rc::new(on_add);
    let on_remove = Rc::new(on_remove);

    h_flex()
        .id("variables-filter-bar")
        .w_full()
        .flex_none()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .child(div().flex_1().min_w_0().when_some(search, |el, input| {
            el.child(
                Input::new(&input).xsmall().w_full().cleanable(true).prefix(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .pl_1()
                        .child(Icon::new(IconName::Search).xsmall()),
                ),
            )
        }))
        .child(
            Button::new("variables-scopes-filter")
                .ghost()
                .xsmall()
                .label("Scopes")
                .dropdown_caret(true)
                .dropdown_menu({
                    let entity = entity.clone();
                    move |mut menu: PopupMenu, _, _| {
                        for scope in available.clone() {
                            let checked = enabled_snapshot.contains(&scope);
                            let entity_scope = entity.clone();
                            let scope_for_click = scope.clone();
                            let on_toggle = on_toggle_scope.clone();
                            menu = menu.item(PopupMenuItem::new(scope).checked(checked).on_click(
                                move |_, _, cx| {
                                    entity_scope.update(cx, |this, cx| {
                                        on_toggle(this, scope_for_click.clone(), cx);
                                    });
                                },
                            ));
                        }
                        menu
                    }
                }),
        )
        .child(div().w(px(1.)).h(px(16.)).flex_none().bg(divider))
        .child(
            Button::new("variables-add")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .tooltip("Add variable")
                .on_click({
                    let entity = entity.clone();
                    let on_add = on_add.clone();
                    move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            on_add(this, window, cx);
                        });
                    }
                }),
        )
        .child(
            Button::new("variables-remove")
                .ghost()
                .xsmall()
                .icon(IconName::Minus)
                .tooltip("Remove selected")
                .on_click({
                    let entity = entity.clone();
                    let on_remove = on_remove.clone();
                    move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            on_remove(this, window, cx);
                        });
                    }
                }),
        )
}
