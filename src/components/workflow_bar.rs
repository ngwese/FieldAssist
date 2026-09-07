// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui::{
    div, App, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString,
    Styled as _, WeakEntity, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Sizable as _, StyledExt as _,
};

use crate::app::AppView;
use crate::script::ToolbarItem;

#[derive(IntoElement)]
pub struct WorkflowBar {
    title: SharedString,
    items: Vec<ToolbarItem>,
    app: WeakEntity<AppView>,
}

impl WorkflowBar {
    pub fn new(
        title: impl Into<SharedString>,
        items: Vec<ToolbarItem>,
        app: WeakEntity<AppView>,
    ) -> Self {
        Self {
            title: title.into(),
            items,
            app,
        }
    }
}

impl RenderOnce for WorkflowBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        h_flex()
            .id("workflow-bar")
            .w_full()
            .flex_none()
            .px_3()
            .py_1()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(theme.foreground)
                    .child(self.title),
            )
            .children(self.items.into_iter().enumerate().map(|(ix, item)| {
                let app = self.app.clone();
                let command = item.command.clone();
                Button::new(("workflow-cmd", ix))
                    .ghost()
                    .small()
                    .label(item.label)
                    .on_click(move |_, window, cx| {
                        if let Some(app) = app.upgrade() {
                            app.update(cx, |this, cx| {
                                this.dispatch_workflow_command(&command, window, cx);
                            });
                        }
                    })
            }))
    }
}
