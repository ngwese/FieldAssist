// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui::{
    div, prelude::FluentBuilder as _, App, Context, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, WeakEntity, Window,
};
use gpui_component::{
    dock::{BasePanel, Panel, PanelEvent},
    ActiveTheme as _,
};

use crate::app::AppView;
use crate::components::drop_overlay::file_drop_overlay;

/// Empty dock pane used when no composition is open.
pub struct EmptyPane {
    name: &'static str,
    title: SharedString,
    message: Option<SharedString>,
    focus_handle: FocusHandle,
    app: WeakEntity<AppView>,
}

impl EmptyPane {
    pub fn new(
        name: &'static str,
        title: impl Into<SharedString>,
        app: WeakEntity<AppView>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            name,
            title: title.into(),
            message: None,
            focus_handle: cx.focus_handle(),
            app,
        }
    }

    pub fn with_message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }
}

impl EventEmitter<PanelEvent> for EmptyPane {}

impl Focusable for EmptyPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for EmptyPane {
    fn panel_name(&self) -> &'static str {
        self.name
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for EmptyPane {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for EmptyPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let app = self.app.clone();
        let layout = self
            .app
            .upgrade()
            .and_then(|app| app.update(cx, |this, cx| this.sync_file_drop_layout(cx)));
        div()
            .id(self.name)
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .drag_over::<ExternalPaths>(move |style, _, _, cx| {
                if let Some(app) = app.upgrade() {
                    app.update(cx, |this, cx| this.ensure_file_drop_layout(cx));
                }
                style
            })
            .children(self.message.clone())
            .when_some(layout, |this, layout| {
                this.child(file_drop_overlay(layout, self.app.clone()))
            })
    }
}
