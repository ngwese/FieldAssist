// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui::{
    px, Action, App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Icon, IconName, IconNamed, Sizable as _,
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
        let muted = theme.muted_foreground;

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
                        muted,
                    ))
                    .child(transport_button(
                        "transport-prev",
                        TransportIcon::SkipBack,
                        "Previous",
                        TransportPrevious,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-play-pause",
                        play_pause_icon,
                        play_pause_label,
                        TransportPlayPause,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-next",
                        TransportIcon::SkipForward,
                        "Next",
                        TransportNext,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-end",
                        TransportIcon::ChevronsRight,
                        "End",
                        TransportEnd,
                        muted,
                    ))
                    .child({
                        let loop_color = if self.looping { theme.cyan } else { muted };
                        transport_button(
                            "transport-loop",
                            TransportIcon::Repeat,
                            loop_label,
                            TransportLoop,
                            loop_color,
                        )
                        .toggled(self.looping)
                    }),
            )
    }
}

fn transport_button(
    id: &'static str,
    icon: impl Into<Icon>,
    label: &'static str,
    action: impl Action + Clone,
    color: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .with_size(CONTROL_SIZE)
        .text_color(color)
        .icon(Icon::new(icon).text_color(color))
        .tooltip_with_action(label, &action, None)
        .accessibility_label(label)
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}
