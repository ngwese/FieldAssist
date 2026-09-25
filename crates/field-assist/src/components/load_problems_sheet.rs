// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Aggregated load-problems UI for a single open operation.

use std::collections::HashSet;

use field_core::{OpenReport, ProblemCategory};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex, ActiveTheme as _, Icon, IconName, StyledExt as _,
};
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, App, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};

/// Inset of the dialog from the window top/bottom edges.
pub const WINDOW_INSET: f32 = 50.;
/// Vertical space inside the panel outside the list (title, intro, footer, gaps).
/// Sized generously so the panel never overflows the window inset.
const PANEL_CHROME: f32 = 180.;
/// Right inset so group boxes clear the overlaid vertical scrollbar.
const SCROLLBAR_GUTTER: f32 = 14.;

/// Scrollable body listing open problems by category.
pub struct LoadProblemsSheet {
    target: SharedString,
    groups: Vec<CategoryGroup>,
    collapsed: HashSet<ProblemCategory>,
}

#[derive(Clone)]
struct CategoryGroup {
    category: ProblemCategory,
    title: SharedString,
    items: Vec<(SharedString, SharedString)>,
}

impl LoadProblemsSheet {
    /// Build from an aggregated open report.
    pub fn new(report: OpenReport, _window: &Window, _cx: &mut Context<Self>) -> Self {
        let target = SharedString::from(report.target.to_string());
        let groups = report
            .grouped()
            .into_iter()
            .map(|(category, problems)| CategoryGroup {
                category,
                title: SharedString::from(category.title()),
                items: problems
                    .into_iter()
                    .map(|p| {
                        (
                            SharedString::from(p.subject.clone()),
                            SharedString::from(p.detail.clone()),
                        )
                    })
                    .collect(),
            })
            .collect();
        Self {
            target,
            groups,
            collapsed: HashSet::new(),
        }
    }

    /// Max panel height that keeps a [`WINDOW_INSET`] gap top and bottom.
    pub fn panel_height_for(window: &Window) -> Pixels {
        let available = f32::from(window.bounds().size.height) - WINDOW_INSET * 2.;
        px(available.max(240.))
    }

    /// Viewport height for the category scroller given the window size.
    pub fn list_height_for(window: &Window) -> Pixels {
        let available = f32::from(Self::panel_height_for(window)) - PANEL_CHROME;
        px(available.max(160.))
    }

    fn toggle(&mut self, category: ProblemCategory, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&category) {
            self.collapsed.insert(category);
        }
        cx.notify();
    }
}

impl Render for LoadProblemsSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let list_h = Self::list_height_for(window);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child("The following errors were encountered when trying to load"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(self.target.clone()),
                    ),
            )
            .child(
                // Match gpui-component scroll tests: definite `.h()` on the
                // scrollable, content in an inner child that can grow taller.
                div()
                    .id("load-problems-groups")
                    .w_full()
                    .h(list_h)
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            // Keep group borders clear of the overlaid scrollbar.
                            .pr(px(SCROLLBAR_GUTTER))
                            .gap_2()
                            .children(self.groups.iter().enumerate().map(|(index, group)| {
                                let open = !self.collapsed.contains(&group.category);
                                let category = group.category;
                                v_flex()
                                    .w_full()
                                    .flex_shrink_0()
                                    .border_1()
                                    .border_color(theme.border)
                                    .rounded_md()
                                    .child(
                                        h_flex()
                                            .id(("load-problem-cat", index))
                                            .w_full()
                                            .px_2()
                                            .py_1()
                                            .gap_2()
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(theme.secondary))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle(category, cx);
                                            }))
                                            .child(
                                                Icon::new(if open {
                                                    IconName::ChevronDown
                                                } else {
                                                    IconName::ChevronRight
                                                })
                                                .size_4(),
                                            )
                                            .child(div().text_sm().font_semibold().child(format!(
                                                "{} ({})",
                                                group.title,
                                                group.items.len()
                                            ))),
                                    )
                                    .when(open, |this| {
                                        this.child(
                                            v_flex().w_full().px_3().pb_2().gap_2().children(
                                                group.items.iter().map(|(subject, detail)| {
                                                    v_flex()
                                                        .gap_0p5()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_semibold()
                                                                .text_color(theme.foreground)
                                                                .child(subject.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(theme.muted_foreground)
                                                                .child(detail.clone()),
                                                        )
                                                }),
                                            ),
                                        )
                                    })
                            })),
                    ),
            )
    }
}

/// Build footer OK control for hosts that want a standalone button.
#[allow(dead_code)]
pub fn ok_button(
    id: &'static str,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Button::new(id)
        .primary()
        .label("OK")
        .on_click(move |_, window, cx| {
            on_ok(window, cx);
        })
}
