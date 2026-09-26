// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex, v_flex, ActiveTheme as _, StyledExt as _,
};
use gpui_kit::{
    div, img, px, App, Context, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window,
};

pub struct AboutView {
    focus_handle: FocusHandle,
}

impl AboutView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for AboutView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AboutView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus_handle(cx))
            .child(crate::components::window_chrome::window_title_bar(format!(
                "About {}",
                crate::APP_NAME
            )))
            .child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .p_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_3()
                            .child(img("icons/app-mark.svg").size(px(128.)).flex_none())
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(div().font_semibold().text_lg().child(crate::APP_NAME))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(muted)
                                            .child(crate::app_version_detail()),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(div().child(env!("CARGO_PKG_DESCRIPTION")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(crate::APP_COPYRIGHT),
                            ),
                    )
                    .child(
                        h_flex().w_full().justify_end().child(
                            Button::new("about-ok").primary().label("OK").on_click(
                                |_, window, _| {
                                    window.remove_window();
                                },
                            ),
                        ),
                    ),
            )
    }
}
