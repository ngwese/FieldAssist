// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Compact channel on/off chip for monitor and export channel lists.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, App, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};

/// Width large enough for a three-digit label at `text_xs` with even
/// horizontal inset; height matches xsmall Button / dropdown (`h_5`).
const CHANNEL_TOGGLE_WIDTH: gpui_kit::Pixels = px(28.);

/// Channel selection chip: label centered in a bordered box; primary fill when
/// on.
#[derive(IntoElement)]
pub struct ChannelToggle {
    id: ElementId,
    label: SharedString,
    checked: bool,
    on_click: Option<Rc<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
}

impl ChannelToggle {
    /// Create a channel toggle with the given element id.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: SharedString::default(),
            checked: false,
            on_click: None,
        }
    }

    /// Set the label drawn centered in the chip.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }

    /// Set whether the channel is selected.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Click handler receives the *new* enabled value (same contract as
    /// [`gpui_kit::component::checkbox::Checkbox`]).
    pub fn on_click(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ChannelToggle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let checked = self.checked;
        let (bg, border, fg) = if checked {
            (theme.primary, theme.primary, theme.primary_foreground)
        } else {
            (theme.transparent, theme.border, theme.muted_foreground)
        };
        // Match default Button / dropdown rounding (`ButtonRounded::Medium`).
        let radius = theme.radius;
        let hover_bg = if checked {
            theme.primary
        } else {
            theme.secondary
        };
        let on_click = self.on_click;

        div()
            .id(self.id)
            .w(CHANNEL_TOGGLE_WIDTH)
            // Same vertical size as `.xsmall()` labeled Button / pulldown.
            .h_5()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_xs()
            .text_color(fg)
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(|this| this.bg(hover_bg))
            .child(self.label)
            .when_some(on_click, |this, on_click| {
                this.on_click(move |_, window, cx| {
                    on_click(&!checked, window, cx);
                })
            })
    }
}
