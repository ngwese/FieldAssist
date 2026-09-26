// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Standalone Preferences window built on gpui Settings.

use field_ui_components::content_foreground;
use gpui_kit::component::button::Button;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::input::{
    InputEvent, InputState, NumberInput, NumberInputEvent, StepAction,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::setting::{
    SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{
    ActiveTheme as _, AxisExt as _, Disableable as _, IconName, Root, Sizable as _, Size, Theme,
    ThemeRegistry,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window,
};

use crate::playback::list_output_devices;
use crate::settings::{self, AppSettings};

const DECIMAL_STEP: f64 = 0.1;
const DECIMAL_FINE_STEP: f64 = 0.01;
const DECIMAL_PLACES: usize = 2;

/// Build the Settings tree bound to the global [`settings::AppSettingsStore`].
pub fn build_settings_ui(cx: &App) -> Settings {
    let theme_options: Vec<(SharedString, SharedString)> = ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .map(|theme| {
            let name: SharedString = theme.name.clone();
            (name.clone(), name)
        })
        .collect();

    let mut device_options: Vec<(SharedString, SharedString)> =
        vec![("".into(), "System Default".into())];
    if let Ok(devices) = list_output_devices() {
        for info in devices {
            let name: SharedString = info.name.into();
            device_options.push((name.clone(), name));
        }
    }

    Settings::new("field-assist-settings")
        .with_size(Size::Small)
        .with_group_variant(GroupBoxVariant::Fill)
        .page(
            SettingPage::new("General")
                .icon(IconName::Settings)
                .default_open(true)
                .description("Appearance, layout, waveform, and selection defaults")
                .group(
                    SettingGroup::new()
                        .title("Appearance")
                        .item(
                            SettingItem::new(
                                "Theme",
                                SettingField::dropdown(
                                    theme_options,
                                    |cx| {
                                        settings::store(cx)
                                            .settings
                                            .appearance
                                            .theme_name
                                            .clone()
                                            .into()
                                    },
                                    |value, cx| {
                                        let name = value.to_string();
                                        match crate::script::theme::apply_theme_name_in_app(
                                            &name, None, cx,
                                        ) {
                                            Ok(()) => {
                                                let mode =
                                                    Theme::global(cx).mode.name().to_string();
                                                let _ = settings::update_and_save(cx, |s| {
                                                    s.appearance.theme_name = name;
                                                    s.appearance.theme_mode = mode;
                                                });
                                            }
                                            Err(err) => {
                                                eprintln!("FieldAssist: theme: {err}");
                                            }
                                        }
                                    },
                                )
                                .default_value(SharedString::from(
                                    AppSettings::default().appearance.theme_name,
                                )),
                            )
                            .description("Registered color theme"),
                        )
                        .item(
                            SettingItem::new(
                                "Mode",
                                SettingField::dropdown(
                                    vec![
                                        ("light".into(), "Light".into()),
                                        ("dark".into(), "Dark".into()),
                                    ],
                                    |cx| {
                                        settings::store(cx)
                                            .settings
                                            .appearance
                                            .theme_mode
                                            .clone()
                                            .into()
                                    },
                                    |value, cx| {
                                        let mode = value.to_string();
                                        let _ = settings::update_and_save(cx, |s| {
                                            s.appearance.theme_mode = mode.clone();
                                        });
                                        match crate::script::theme::parse_theme_mode(&mode) {
                                            Ok(parsed) => {
                                                crate::script::theme::apply_theme_mode_in_app(
                                                    parsed, None, cx,
                                                );
                                            }
                                            Err(err) => {
                                                eprintln!("FieldAssist: theme mode: {err}");
                                            }
                                        }
                                    },
                                )
                                .default_value(SharedString::from("dark")),
                            )
                            .description("Light or dark appearance"),
                        ),
                )
                .group(
                    SettingGroup::new()
                        .title("View")
                        .item(dock_switch(
                            "Explorer",
                            "Show the explorer dock by default",
                            |s| s.view.explorer,
                            |s, v| s.view.explorer = v,
                            false,
                            "view.show-explorer",
                            "view.hide-explorer",
                        ))
                        .item(dock_tab_menu(
                            "Detail",
                            "Default detail dock tab (or hide)",
                            "detail-dock-pref",
                            crate::dock_titles::DETAIL_DOCK_TAB_TITLES,
                            settings::DOCK_HIDDEN,
                            |s| s.view.detail.as_str(),
                            |s, v| s.view.detail = v.into(),
                            crate::app::apply_detail_from_settings,
                        ))
                        .item(dock_tab_menu(
                            "Script",
                            "Default script dock tab (or hide)",
                            "script-dock-pref",
                            crate::dock_titles::BOTTOM_DOCK_TAB_TITLES,
                            settings::DOCK_HIDDEN,
                            |s| s.view.script.as_str(),
                            |s, v| s.view.script = v.into(),
                            crate::app::apply_script_from_settings,
                        )),
                )
                .group(
                    SettingGroup::new()
                        .title("Waveform")
                        .item(
                            SettingItem::new(
                                "Follow Playhead",
                                SettingField::switch(
                                    |cx| settings::store(cx).settings.waveform.follow_playhead,
                                    |on, cx| {
                                        let _ = settings::update_and_save(cx, |s| {
                                            s.waveform.follow_playhead = on;
                                        });
                                        crate::app::apply_waveform_default_from_settings(cx);
                                    },
                                )
                                .default_value(true),
                            )
                            .description("Keep the playhead in view while playing"),
                        )
                        .item(
                            SettingItem::new(
                                "Representation",
                                SettingField::dropdown(
                                    vec![
                                        ("peaks".into(), "Peaks".into()),
                                        ("spectrum".into(), "Spectrum".into()),
                                        ("peaks_spectrum".into(), "Peaks + Spectrum".into()),
                                    ],
                                    |cx| {
                                        settings::store(cx)
                                            .settings
                                            .waveform
                                            .representation
                                            .clone()
                                            .into()
                                    },
                                    |value, cx| {
                                        let rep = value.to_string();
                                        let _ = settings::update_and_save(cx, |s| {
                                            s.waveform.set_representation_str(&rep);
                                        });
                                        crate::app::apply_waveform_default_from_settings(cx);
                                    },
                                )
                                .default_value(SharedString::from("peaks")),
                            )
                            .description("Default waveform body for new documents"),
                        )
                        .item(
                            SettingItem::new(
                                "Peak Rendering",
                                SettingField::dropdown(
                                    vec![
                                        ("simple".into(), "Simple".into()),
                                        ("threaded".into(), "Threaded".into()),
                                    ],
                                    |cx| {
                                        settings::store(cx)
                                            .settings
                                            .waveform
                                            .peak_rendering
                                            .clone()
                                            .into()
                                    },
                                    |value, cx| {
                                        let mode = value.to_string();
                                        let _ = settings::update_and_save(cx, |s| {
                                            s.waveform.set_peak_rendering_str(&mode);
                                        });
                                        crate::app::apply_waveform_default_from_settings(cx);
                                    },
                                )
                                .default_value(SharedString::from("threaded")),
                            )
                            .description("Overview peaks: single color, or shell plus −dB ribbon"),
                        )
                        .item(waveform_decimal_field(
                            "Threaded Shell Value",
                            "HSV value reduction for the Threaded outer peak (0–1)",
                            "waveform-threaded-shell",
                            0.0,
                            1.0,
                            field_ui_components::DEFAULT_THREADED_SHELL_VALUE_REDUCE as f64,
                            |s| s.waveform.threaded_shell_value_reduce as f64,
                            |s, v| s.waveform.set_threaded_shell_value_reduce(v as f32),
                        ))
                        .item(waveform_decimal_field(
                            "Threaded Ribbon dB",
                            "Inner ribbon amplitude vs full peak when Threaded (dBFS)",
                            "waveform-threaded-ribbon",
                            -48.0,
                            0.0,
                            field_ui_components::DEFAULT_THREADED_RIBBON_DB as f64,
                            |s| s.waveform.threaded_ribbon_db as f64,
                            |s, v| s.waveform.set_threaded_ribbon_db(v as f32),
                        )),
                )
                .group(
                    SettingGroup::new()
                        .title("Selection")
                        .item(selection_switch(
                            "Zero Crossing",
                            "Snap placement to zero crossings by default",
                            |s| s.selection.zero_crossing,
                            |s, v| s.selection.zero_crossing = v,
                            true,
                        ))
                        .item(selection_switch(
                            "Snap to Marker",
                            "Snap placement to markers by default",
                            |s| s.selection.snap_to_marker,
                            |s, v| s.selection.snap_to_marker = v,
                            false,
                        ))
                        .item(selection_switch(
                            "Add at Hover",
                            "Place markers at the pointer instead of the caret",
                            |s| s.selection.add_at_hover,
                            |s, v| s.selection.add_at_hover = v,
                            true,
                        )),
                ),
        )
        .page(
            SettingPage::new("Audio")
                .icon(IconName::HardDrive)
                .description("Playback output")
                .group(
                    SettingGroup::new().title("Device").item(
                        SettingItem::new(
                            "Output",
                            SettingField::scrollable_dropdown(
                                device_options,
                                |cx| {
                                    settings::store(cx)
                                        .settings
                                        .audio
                                        .output_device
                                        .clone()
                                        .unwrap_or_default()
                                        .into()
                                },
                                |value, cx| {
                                    let device = {
                                        let text = value.to_string();
                                        if text.is_empty() {
                                            None
                                        } else {
                                            Some(text)
                                        }
                                    };
                                    let _ = settings::update_and_save(cx, |s| {
                                        s.audio.output_device = device.clone();
                                    });
                                    crate::app::apply_audio_device_from_settings(cx);
                                },
                            )
                            .default_value(SharedString::from("")),
                        )
                        .description("Preferred output device (substring match)"),
                    ),
                ),
        )
        .page(
            SettingPage::new("Experimental")
                .icon(IconName::Settings)
                .description("Unstable and preview features")
                .group(
                    SettingGroup::new()
                        .title("Flags")
                        .item(flag_switch(
                            "Content Credentials",
                            "Enable Content Credentials (C2PA) tooling when available",
                            |s| s.experimental.flags.content_credentials,
                            |s, v| s.experimental.flags.content_credentials = v,
                            false,
                        ))
                        .item(flag_switch(
                            "Analysis Ops",
                            "Enable the Analyze menu and analysis commands",
                            |s| s.experimental.flags.analysis_ops,
                            |s, v| s.experimental.flags.analysis_ops = v,
                            false,
                        )),
                ),
        )
}

fn flag_switch(
    title: &'static str,
    description: &'static str,
    get: fn(&AppSettings) -> bool,
    set: fn(&mut AppSettings, bool),
    default: bool,
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx| get(&settings::store(cx).settings),
            move |on, cx| {
                let _ = settings::update_and_save(cx, |s| set(s, on));
                crate::app::apply_experimental_from_settings(cx);
            },
        )
        .default_value(default),
    )
    .description(description)
}

fn dock_switch(
    title: &'static str,
    description: &'static str,
    get: fn(&AppSettings) -> bool,
    set: fn(&mut AppSettings, bool),
    default: bool,
    show_cmd: &'static str,
    hide_cmd: &'static str,
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx| get(&settings::store(cx).settings),
            move |on, cx| {
                let _ = settings::update_and_save(cx, |s| set(s, on));
                let cmd = if on { show_cmd } else { hide_cmd };
                let _ = crate::commands::dispatch(cmd, cx);
            },
        )
        .default_value(default),
    )
    .description(description)
}

fn dock_pref_label(value: &str) -> SharedString {
    if value == settings::DOCK_HIDDEN {
        "Hidden".into()
    } else {
        value.to_string().into()
    }
}

fn dock_tab_menu(
    title: &'static str,
    description: &'static str,
    button_id: &'static str,
    tabs: &'static [&'static str],
    default: &'static str,
    get: fn(&AppSettings) -> &str,
    set: fn(&mut AppSettings, &str),
    apply: fn(&mut App),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::render(move |options, _, cx| {
            let current = get(&settings::store(cx).settings).to_string();
            let label = dock_pref_label(&current);
            Button::new(button_id)
                .label(label)
                .dropdown_caret(true)
                .outline()
                .disabled(options.is_disabled())
                .with_size(options.size())
                .dropdown_menu(move |mut menu, _, _| {
                    let checked = current == settings::DOCK_HIDDEN;
                    menu = menu.item(PopupMenuItem::new("Hidden").checked(checked).on_click(
                        move |_, _, cx| {
                            let _ = settings::update_and_save(cx, |s| {
                                set(s, settings::DOCK_HIDDEN);
                            });
                            apply(cx);
                        },
                    ));
                    menu = menu.separator();
                    for tab in tabs {
                        let value = (*tab).to_string();
                        let checked = current == *tab;
                        menu = menu.item(PopupMenuItem::new(*tab).checked(checked).on_click(
                            move |_, _, cx| {
                                let value = value.clone();
                                let _ = settings::update_and_save(cx, |s| set(s, &value));
                                apply(cx);
                            },
                        ));
                    }
                    menu
                })
        })
        .on_reset(
            move |cx| get(&settings::store(cx).settings) != default,
            move |_, cx| {
                let _ = settings::update_and_save(cx, |s| set(s, default));
                apply(cx);
            },
        ),
    )
    .description(description)
}

fn selection_switch(
    title: &'static str,
    description: &'static str,
    get: fn(&AppSettings) -> bool,
    set: fn(&mut AppSettings, bool),
    default: bool,
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx| get(&settings::store(cx).settings),
            move |on, cx| {
                let _ = settings::update_and_save(cx, |s| set(s, on));
                crate::app::apply_selection_defaults_from_settings(cx);
            },
        )
        .default_value(default),
    )
    .description(description)
}

fn format_decimal(value: f64) -> SharedString {
    SharedString::from(format!("{value:.DECIMAL_PLACES$}"))
}

fn quantize_decimal(value: f64, min: f64, max: f64) -> f64 {
    let scale = 10f64.powi(DECIMAL_PLACES as i32);
    ((value.clamp(min, max) * scale).round() / scale).clamp(min, max)
}

fn persist_decimal(set: fn(&mut AppSettings, f64), value: f64, cx: &mut App) {
    let _ = settings::update_and_save(cx, |s| set(s, value));
    crate::app::apply_waveform_default_from_settings(cx);
}

/// Number stepper showing two decimals; ±0.1, or ±0.01 while Shift is held.
fn waveform_decimal_field(
    title: &'static str,
    description: &'static str,
    field_id: &'static str,
    min: f64,
    max: f64,
    default: f64,
    get: fn(&AppSettings) -> f64,
    set: fn(&mut AppSettings, f64),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::render(move |options, window, cx| {
            let value = quantize_decimal(get(&settings::store(cx).settings), min, max);
            let state_entity = window.use_keyed_state(
                SharedString::from(format!(
                    "decimal-{}-{}-{}-{}",
                    field_id,
                    options.page_ix(),
                    options.group_ix(),
                    options.item_ix()
                )),
                cx,
                |window, cx| {
                    let input = cx.new(|cx| {
                        // Default InputState uses Fixed(1.) steps and never emits
                        // NumberInputEvent::Step. Clear that so we own stepping.
                        let mut state =
                            InputState::new(window, cx).default_value(format_decimal(value));
                        state.set_step(None, window, cx);
                        state
                    });
                    let _subscriptions = vec![
                        cx.subscribe_in(&input, window, {
                            move |state: &mut DecimalFieldState,
                                  input,
                                  event: &NumberInputEvent,
                                  window,
                                  cx| {
                                let NumberInputEvent::Step(action) = event;
                                let Ok(current) = input.read(cx).value().parse::<f64>() else {
                                    return;
                                };
                                let step = if window.modifiers().shift {
                                    DECIMAL_FINE_STEP
                                } else {
                                    DECIMAL_STEP
                                };
                                let next = match *action {
                                    StepAction::Increment => current + step,
                                    StepAction::Decrement => current - step,
                                };
                                let clamped = quantize_decimal(next, min, max);
                                if (clamped - current).abs() < 1e-12 {
                                    return;
                                }
                                let text = format_decimal(clamped);
                                input.update(cx, |input, cx| {
                                    input.set_value(text, window, cx);
                                });
                                persist_decimal(set, clamped, cx);
                                state.initial_value = clamped;
                            }
                        }),
                        cx.subscribe_in(&input, window, {
                            move |state: &mut DecimalFieldState,
                                  input,
                                  event: &InputEvent,
                                  window,
                                  cx| {
                                match event {
                                    InputEvent::Change => {
                                        // Persist when the text parses, but do not
                                        // rewrite mid-keystroke (that blocks decimals).
                                        let raw = input.read(cx).value();
                                        let Ok(parsed) = raw.parse::<f64>() else {
                                            return;
                                        };
                                        let clamped = quantize_decimal(parsed, min, max);
                                        if (clamped - state.initial_value).abs() < 1e-9 {
                                            return;
                                        }
                                        persist_decimal(set, clamped, cx);
                                        state.initial_value = clamped;
                                    }
                                    InputEvent::Blur => {
                                        let raw = input.read(cx).value();
                                        let parsed =
                                            raw.parse::<f64>().unwrap_or(state.initial_value);
                                        let clamped = quantize_decimal(parsed, min, max);
                                        let text = format_decimal(clamped);
                                        if raw != text.as_ref() {
                                            input.update(cx, |input, cx| {
                                                input.set_value(text, window, cx);
                                            });
                                        }
                                        if (clamped - state.initial_value).abs() >= 1e-9 {
                                            persist_decimal(set, clamped, cx);
                                            state.initial_value = clamped;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }),
                    ];
                    DecimalFieldState {
                        input,
                        initial_value: value,
                        _subscriptions,
                    }
                },
            );

            state_entity.update(cx, |state, cx| {
                // Keep caller-owned stepping even if a prior keyed state still
                // has InputState's default Fixed(1.) step.
                state.input.update(cx, |input, cx| {
                    input.set_step(None, window, cx);
                });
                if (state.initial_value - value).abs() >= 1e-9 {
                    state.initial_value = value;
                    state.input.update(cx, |input, cx| {
                        input.set_value(format_decimal(value), window, cx);
                    });
                }
            });

            let state = state_entity.read(cx);
            NumberInput::new(&state.input)
                .disabled(options.is_disabled())
                .with_size(options.size())
                .map(|this| {
                    if options.layout().is_horizontal() {
                        this.w_32()
                    } else {
                        this.w_full()
                    }
                })
        })
        .on_reset(
            move |cx| {
                let current = quantize_decimal(get(&settings::store(cx).settings), min, max);
                (current - quantize_decimal(default, min, max)).abs() >= 1e-9
            },
            move |_, cx| {
                let value = quantize_decimal(default, min, max);
                persist_decimal(set, value, cx);
            },
        ),
    )
    .description(description)
}

struct DecimalFieldState {
    input: Entity<InputState>,
    initial_value: f64,
    _subscriptions: Vec<Subscription>,
}

/// Root view for the Preferences window.
pub struct SettingsView {
    focus_handle: FocusHandle,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        settings::ensure_store(cx);
        // Refresh from disk so the dialog matches settings.json even when no
        // editor init has run yet (e.g. Settings opened with no main window).
        settings::reload_from_disk(cx);
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // SettingItem titles use Label, which paints `theme.foreground`. FieldAssist
        // mutes chrome foreground; use content ink so names stay bright and only
        // descriptions (explicit muted_foreground) read secondary.
        let content = content_foreground(cx);
        Theme::global_mut(cx).foreground = content;

        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(content)
            .text_sm()
            .child(build_settings_ui(cx))
    }
}

#[derive(Clone, Copy)]
struct SettingsWindow(gpui_kit::AnyWindowHandle);

impl Global for SettingsWindow {}

fn window_is_open(cx: &App, handle: gpui_kit::AnyWindowHandle) -> bool {
    cx.windows().iter().any(|window| *window == handle)
}

/// Open or focus the singleton Settings window.
pub fn open_settings_window(cx: &mut App) {
    settings::ensure_store(cx);

    if let Some(SettingsWindow(handle)) = cx.try_global::<SettingsWindow>().copied() {
        if window_is_open(cx, handle) {
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
            return;
        }
        let _ = cx.remove_global::<SettingsWindow>();
    }

    use gpui_kit::{
        point, px, size, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions,
    };

    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from("Settings")),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(160.), px(120.)),
            size: size(px(880.), px(640.)),
        })),
        app_owns_titlebar_drag: false,
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Server),
        ..Default::default()
    };

    match cx.open_window(options, |window, cx| {
        let view = cx.new(SettingsView::new);
        window.focus(&view.focus_handle(cx), cx);
        cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
    }) {
        Ok(handle) => {
            cx.set_global(SettingsWindow(handle.into()));
        }
        Err(err) => {
            eprintln!("failed to open Settings window: {err}");
        }
    }
}

/// Clear the Settings singleton when that window closes.
pub fn on_settings_window_closed(cx: &mut App, id: gpui_kit::WindowId) {
    if let Some(SettingsWindow(handle)) = cx.try_global::<SettingsWindow>().copied() {
        if handle.window_id() == id {
            let _ = cx.remove_global::<SettingsWindow>();
            crate::app::apply_muted_chrome(cx);
        }
    }
}
