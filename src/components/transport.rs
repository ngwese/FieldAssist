// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui::{
    prelude::FluentBuilder as _, px, Action, App, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Colorize as _, Icon, IconName, IconNamed, Selectable as _,
    Sizable as _,
};

use crate::commands::{
    TransportEnd, TransportHome, TransportLoop, TransportNext, TransportPlayPause,
    TransportPrevious,
};
use crate::playback::TransportState;

const CONTROL_SIZE: gpui::Pixels = px(28.);

#[derive(Copy, Clone)]
enum TransportIcon {
    ChevronsLeft,
    SkipBack,
    SkipForward,
    ChevronsRight,
    Repeat,
}

impl IconNamed for TransportIcon {
    fn path(self) -> SharedString {
        match self {
            Self::ChevronsLeft => "icons/chevrons-left.svg",
            Self::SkipBack => "icons/skip-back.svg",
            Self::SkipForward => "icons/skip-forward.svg",
            Self::ChevronsRight => "icons/chevrons-right.svg",
            Self::Repeat => "icons/repeat.svg",
        }
        .into()
    }
}

#[derive(IntoElement)]
pub struct Transport {
    state: TransportState,
    looping: bool,
}

impl Transport {
    pub fn new(state: TransportState, looping: bool) -> Self {
        Self { state, looping }
    }
}

impl RenderOnce for Transport {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let playing = self.state == TransportState::Playing;
        let play_pause_icon = if playing {
            IconName::Pause
        } else {
            IconName::Play
        };
        let play_pause_label = if playing { "Pause" } else { "Play" };
        let loop_label = if self.looping { "Loop On" } else { "Loop Off" };

        h_flex()
            .id("transport-bar")
            .w_full()
            .flex_none()
            .px_3()
            .py_2()
            .items_center()
            .justify_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(
                h_flex()
                    .id("transport")
                    .items_center()
                    .gap_1()
                    .child(transport_button(
                        "transport-home",
                        TransportIcon::ChevronsLeft,
                        "Home",
                        TransportHome,
                    ))
                    .child(transport_button(
                        "transport-prev",
                        TransportIcon::SkipBack,
                        "Previous",
                        TransportPrevious,
                    ))
                    .child(transport_button(
                        "transport-play-pause",
                        play_pause_icon,
                        play_pause_label,
                        TransportPlayPause,
                    ))
                    .child(transport_button(
                        "transport-next",
                        TransportIcon::SkipForward,
                        "Next",
                        TransportNext,
                    ))
                    .child(transport_button(
                        "transport-end",
                        TransportIcon::ChevronsRight,
                        "End",
                        TransportEnd,
                    ))
                    .child({
                        let loop_icon = if self.looping {
                            Icon::new(TransportIcon::Repeat).text_color(theme.primary)
                        } else {
                            Icon::new(TransportIcon::Repeat)
                        };
                        transport_button("transport-loop", loop_icon, loop_label, TransportLoop)
                            .selected(self.looping)
                            .toggled(self.looping)
                            .when(self.looping, |this| {
                                let bg = if theme.is_dark() {
                                    theme.secondary.lighten(0.1).opacity(1.)
                                } else {
                                    theme.secondary.darken(0.1).opacity(1.)
                                };
                                this.text_color(theme.primary).bg(bg)
                            })
                    }),
            )
    }
}

fn transport_button(
    id: &'static str,
    icon: impl Into<Icon>,
    label: &'static str,
    action: impl Action + Clone,
) -> Button {
    Button::new(id)
        .ghost()
        .with_size(CONTROL_SIZE)
        .icon(icon)
        .tooltip_with_action(label, &action, None)
        .accessibility_label(label)
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}
