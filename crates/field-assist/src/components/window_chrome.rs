// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Shared custom title-bar chrome for secondary windows (Settings, About).

use gpui_kit::component::{h_flex, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, img, px, size, Bounds, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Point, SharedString, Size, Styled as _, TitlebarOptions, WindowBounds, WindowOptions,
};

/// [`WindowOptions`] that pair with a drawn [`TitleBar`] (theme-matching chrome).
pub fn themed_window_options(
    title: impl Into<SharedString>,
    origin: Point<Pixels>,
    window_size: Size<Pixels>,
) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..TitleBar::title_bar_options()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin,
            size: window_size,
        })),
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Client),
        ..TitleBar::window_options()
    }
}

/// Custom title bar content matching the main window theme.
pub fn window_title_bar(title: impl Into<SharedString>) -> impl IntoElement {
    let title = title.into();
    TitleBar::new().child(
        h_flex()
            .id("secondary-window-title-bar")
            .h_full()
            .items_center()
            .gap_2()
            .when(!cfg!(target_os = "macos"), |this| {
                this.child(img("icons/app-mark.svg").size(px(16.)).flex_none())
            })
            .child(div().text_sm().child(title)),
    )
}

/// Convenience size constructor for secondary windows.
pub fn window_size(w: f32, h: f32) -> Size<Pixels> {
    size(px(w), px(h))
}
