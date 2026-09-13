// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Transport control strip for audio editors.

use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Icon, IconName, IconNamed, Sizable as _,
};
use gpui_kit::{
    px, Action, App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window,
};

const CONTROL_SIZE: gpui_kit::Pixels = px(28.);

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

/// Callback that returns a boxed GPUI action for dispatch and tooltips.
pub type TransportAction = Rc<dyn Fn() -> Box<dyn Action>>;

/// Transport control strip.
///
/// The host supplies play/loop state and six actions (home, previous,
/// play/pause, next, end, loop) so this widget stays free of app command types.
#[derive(IntoElement)]
pub struct Transport {
    playing: bool,
    looping: bool,
    home: TransportAction,
    previous: TransportAction,
    play_pause: TransportAction,
    next: TransportAction,
    end: TransportAction,
    toggle_loop: TransportAction,
}

impl Transport {
    /// Build a transport bar from host play state and action factories.
    pub fn new(
        playing: bool,
        looping: bool,
        home: TransportAction,
        previous: TransportAction,
        play_pause: TransportAction,
        next: TransportAction,
        end: TransportAction,
        toggle_loop: TransportAction,
    ) -> Self {
        Self {
            playing,
            looping,
            home,
            previous,
            play_pause,
            next,
            end,
            toggle_loop,
        }
    }
}

impl RenderOnce for Transport {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let play_pause_icon = if self.playing {
            IconName::Pause
        } else {
            IconName::Play
        };
        let play_pause_label = if self.playing { "Pause" } else { "Play" };
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
                        self.home,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-prev",
                        TransportIcon::SkipBack,
                        "Previous",
                        self.previous,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-play-pause",
                        play_pause_icon,
                        play_pause_label,
                        self.play_pause,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-next",
                        TransportIcon::SkipForward,
                        "Next",
                        self.next,
                        muted,
                    ))
                    .child(transport_button(
                        "transport-end",
                        TransportIcon::ChevronsRight,
                        "End",
                        self.end,
                        muted,
                    ))
                    .child({
                        let loop_color = if self.looping { theme.cyan } else { muted };
                        transport_button(
                            "transport-loop",
                            TransportIcon::Repeat,
                            loop_label,
                            self.toggle_loop,
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
    action: TransportAction,
    color: Hsla,
) -> Button {
    let action_for_tooltip = (action)();
    Button::new(id)
        .ghost()
        .with_size(CONTROL_SIZE)
        .text_color(color)
        .icon(Icon::new(icon).text_color(color))
        .tooltip_with_action(label, action_for_tooltip.as_ref(), None)
        .accessibility_label(label)
        .on_click(move |_, window, cx| {
            window.dispatch_action((action)(), cx);
        })
}
