// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Reusable variables table (name / value / scope).

use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex, v_flex, ActiveTheme as _, Sizable as _, StyledExt as _,
};
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, App, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};

/// One row in a variables table.
#[derive(Clone, Debug)]
pub struct VariableRow {
    /// Leaf name.
    pub name: String,
    /// Current value.
    pub value: String,
    /// Full scope path (`user`, `source.bwf`, …).
    pub scope: String,
    /// Optional description (shown after the name when present).
    pub description: Option<String>,
}

/// Capabilities for a variables table instance.
#[derive(Clone, Copy, Debug, Default)]
pub struct VariablesTableOptions {
    /// Allow editing values (click value to notify host).
    pub editable_values: bool,
    /// Allow changing scope by cycling choices on click.
    pub editable_scopes: bool,
    /// Show add/remove controls.
    pub allow_add_remove: bool,
}

/// Invoked when a value is edited.
pub type VariableValueHandler = Rc<dyn Fn(usize, String, &mut Window, &mut App)>;
/// Invoked when a scope is changed.
pub type VariableScopeHandler = Rc<dyn Fn(usize, String, &mut Window, &mut App)>;
/// Invoked when a row is removed.
pub type VariableRemoveHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// Invoked when the user adds a row.
pub type VariableAddHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Stateless variables table element.
#[derive(IntoElement)]
pub struct VariablesTable {
    rows: Vec<VariableRow>,
    options: VariablesTableOptions,
    scope_choices: Vec<SharedString>,
    on_value: Option<VariableValueHandler>,
    on_scope: Option<VariableScopeHandler>,
    on_remove: Option<VariableRemoveHandler>,
    on_add: Option<VariableAddHandler>,
}

impl VariablesTable {
    /// Create a table from rows.
    pub fn new(rows: Vec<VariableRow>) -> Self {
        Self {
            rows,
            options: VariablesTableOptions::default(),
            scope_choices: Vec::new(),
            on_value: None,
            on_scope: None,
            on_remove: None,
            on_add: None,
        }
    }

    /// Set capability flags.
    pub fn options(mut self, options: VariablesTableOptions) -> Self {
        self.options = options;
        self
    }

    /// Scopes offered when [`VariablesTableOptions::editable_scopes`].
    pub fn scope_choices(
        mut self,
        choices: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        self.scope_choices = choices.into_iter().map(Into::into).collect();
        self
    }

    /// Value-edit callback.
    pub fn on_value_change(mut self, handler: VariableValueHandler) -> Self {
        self.on_value = Some(handler);
        self
    }

    /// Scope-change callback.
    pub fn on_scope_change(mut self, handler: VariableScopeHandler) -> Self {
        self.on_scope = Some(handler);
        self
    }

    /// Remove-row callback.
    pub fn on_remove(mut self, handler: VariableRemoveHandler) -> Self {
        self.on_remove = Some(handler);
        self
    }

    /// Add-row callback.
    pub fn on_add(mut self, handler: VariableAddHandler) -> Self {
        self.on_add = Some(handler);
        self
    }
}

impl RenderOnce for VariablesTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let options = self.options;
        let scope_choices = self.scope_choices.clone();
        let on_value = self.on_value.clone();
        let on_scope = self.on_scope.clone();
        let on_remove = self.on_remove.clone();
        let on_add = self.on_add.clone();

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().w(px(140.)).text_xs().font_semibold().child("name"))
                    .child(div().flex_1().text_xs().font_semibold().child("value"))
                    .child(div().w(px(140.)).text_xs().font_semibold().child("scope"))
                    .when(options.allow_add_remove, |row| {
                        row.child(div().w(px(48.)).child(""))
                    }),
            )
            .child(
                v_flex()
                    .id("variables-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                        let on_value = on_value.clone();
                        let on_scope = on_scope.clone();
                        let on_remove = on_remove.clone();
                        let scope_choices = scope_choices.clone();
                        let row_scope = row.scope.clone();
                        h_flex()
                            .id(("var-row", index))
                            .w_full()
                            .px_2()
                            .py_1()
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .hover(|s| s.bg(theme.muted.opacity(0.35)))
                            .child(
                                div()
                                    .w(px(140.))
                                    .text_xs()
                                    .truncate()
                                    .whitespace_nowrap()
                                    .child(single_line_display(&row.name)),
                            )
                            .child({
                                let display = single_line_display(&row.value);
                                let mut value_el = div()
                                    .id(("var-value", index))
                                    .flex_1()
                                    .text_xs()
                                    .truncate()
                                    .whitespace_nowrap()
                                    .child(display);
                                if options.editable_values {
                                    let on_value = on_value.clone();
                                    let current = row.value.clone();
                                    value_el = value_el.on_click(move |_, window, cx| {
                                        if let Some(handler) = &on_value {
                                            handler(index, current.clone(), window, cx);
                                        }
                                    });
                                }
                                value_el
                            })
                            .child({
                                let mut scope_el = div()
                                    .id(("var-scope", index))
                                    .w(px(140.))
                                    .text_xs()
                                    .truncate()
                                    .whitespace_nowrap()
                                    .child(single_line_display(&row.scope));
                                if options.editable_scopes && !scope_choices.is_empty() {
                                    let on_scope = on_scope.clone();
                                    let choices = scope_choices.clone();
                                    scope_el = scope_el.on_click(move |_, window, cx| {
                                        if let Some(handler) = &on_scope {
                                            let next = choices
                                                .iter()
                                                .position(|c| c.as_ref() == row_scope)
                                                .map(|i| (i + 1) % choices.len())
                                                .unwrap_or(0);
                                            handler(index, choices[next].to_string(), window, cx);
                                        }
                                    });
                                }
                                scope_el
                            })
                            .when(options.allow_add_remove, |row_el| {
                                let on_remove = on_remove.clone();
                                row_el.child(
                                    Button::new(("var-del", index))
                                        .ghost()
                                        .xsmall()
                                        .label("Del")
                                        .on_click(move |_, window, cx| {
                                            if let Some(handler) = &on_remove {
                                                handler(index, window, cx);
                                            }
                                        }),
                                )
                            })
                    })),
            )
            .when(options.allow_add_remove, |col| {
                let on_add = on_add.clone();
                col.child(
                    h_flex().w_full().px_2().py_1().child(
                        Button::new("var-add")
                            .ghost()
                            .xsmall()
                            .label("Add variable")
                            .on_click(move |_, window, cx| {
                                if let Some(handler) = &on_add {
                                    handler(window, cx);
                                }
                            }),
                    ),
                )
            })
    }
}

/// Collapse CR/LF/tabs and runs of whitespace so table cells stay one line.
fn single_line_display(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
