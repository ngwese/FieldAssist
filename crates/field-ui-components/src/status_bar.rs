// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Status bar chrome for open media and layout controls.

use std::rc::Rc;

use gpui_kit::{
    div, prelude::FluentBuilder as _, px, rems, App, Hsla, IntoElement, ParentElement as _,
    RenderOnce, SharedString, Styled as _, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    status_bar::StatusBar,
    ActiveTheme as _, Icon, IconNamed, Selectable as _, Sizable as _,
};

const HEIGHT: gpui_kit::Pixels = px(24.);
const MONITOR_ICON_SIZE: gpui_kit::Pixels = px(20.);

/// Snapshot of open-file metadata shown in the status bar.
#[derive(Clone, Debug)]
pub struct FileStatus {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Bits per sample when known.
    pub bits_per_sample: Option<u32>,
    /// Channel count.
    pub channel_count: usize,
    /// Duration in seconds.
    pub duration_secs: f64,
    /// On-disk size when known.
    pub size_bytes: Option<u64>,
}

/// Channel-layout picker binding for the status bar.
pub struct LayoutPicker {
    /// Currently selected layout name.
    pub current: Option<String>,
    /// Available `(name, description)` choices.
    pub choices: Vec<(String, String)>,
    /// Invoked when the user picks a layout name.
    pub on_choose: Rc<dyn Fn(&str, &mut Window, &mut App)>,
}

/// Bottom status bar with file metadata, progress, layout, and monitor toggles.
#[derive(IntoElement)]
pub struct FileStatusBar {
    file: Option<FileStatus>,
    progress_message: Option<String>,
    layout: Option<LayoutPicker>,
    on_monitor: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    monitor_selected: bool,
    on_preview: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    preview_selected: bool,
}

impl FileStatusBar {
    /// Create a status bar, optionally showing open-file metadata.
    pub fn new(file: Option<FileStatus>) -> Self {
        Self {
            file,
            progress_message: None,
            layout: None,
            on_monitor: None,
            monitor_selected: false,
            on_preview: None,
            preview_selected: false,
        }
    }

    /// Show a background-job message in the center.
    pub fn with_progress_message(mut self, message: Option<String>) -> Self {
        self.progress_message = message;
        self
    }

    /// Attach a channel-layout picker on the right.
    pub fn with_layout(mut self, layout: Option<LayoutPicker>) -> Self {
        self.layout = layout;
        self
    }

    /// Attach a monitor-panel toggle.
    pub fn with_monitor(mut self, on_monitor: Option<Rc<dyn Fn(&mut Window, &mut App)>>) -> Self {
        self.on_monitor = on_monitor;
        self
    }

    /// Whether the monitor toggle appears selected.
    pub fn with_monitor_selected(mut self, selected: bool) -> Self {
        self.monitor_selected = selected;
        self
    }

    /// Attach a preview-autoplay toggle.
    pub fn with_preview(mut self, on_preview: Option<Rc<dyn Fn(&mut Window, &mut App)>>) -> Self {
        self.on_preview = on_preview;
        self
    }

    /// Whether the preview toggle appears selected.
    pub fn with_preview_selected(mut self, selected: bool) -> Self {
        self.preview_selected = selected;
        self
    }
}

impl RenderOnce for FileStatusBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut bar = StatusBar::new()
            .w_full()
            .flex_none()
            .h(HEIGHT)
            .min_h(HEIGHT)
            .max_h(HEIGHT)
            .py_0()
            .text_xs()
            .when(cfg!(target_os = "macos"), |this| this.px(rems(1.)));
        let muted = cx.theme().muted_foreground;
        if let Some(on_preview) = self.on_preview {
            bar = bar.left(preview_button(on_preview, self.preview_selected, muted, cx));
        }
        let has_file = self.file.is_some();
        if let Some(file) = self.file {
            bar = bar
                .left(format!("{} Hz", file.sample_rate))
                .left(format_bit_depth(file.bits_per_sample))
                .left(format!("{} ch", file.channel_count))
                .left(format_duration(file.duration_secs))
                .left(
                    file.size_bytes
                        .map(format_bytes)
                        .unwrap_or_else(|| "—".into()),
                );
        }
        if let Some(message) = self.progress_message.as_ref() {
            if !has_file {
                bar = bar.left("");
            }
            bar = bar.child(message.clone());
        }
        if self.on_monitor.is_some() || self.layout.is_some() {
            if let Some(layout) = self.layout {
                bar = bar.right(layout_dropdown(layout, muted));
            }
            if let Some(on_monitor) = self.on_monitor {
                bar = bar.right(monitor_button(on_monitor, muted, self.monitor_selected));
            }
        } else if self.progress_message.is_some() {
            bar = bar.right("");
        }
        bar
    }
}

struct MonitorSpeakerIcon;

impl IconNamed for MonitorSpeakerIcon {
    fn path(self) -> SharedString {
        "icons/monitor-speaker.svg".into()
    }
}

struct CirclePlayIcon;

impl IconNamed for CirclePlayIcon {
    fn path(self) -> SharedString {
        "icons/circle-play.svg".into()
    }
}

fn preview_button(
    on_click: Rc<dyn Fn(&mut Window, &mut App)>,
    selected: bool,
    muted: Hsla,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let color = if selected { theme.cyan } else { muted };
    Button::new("preview-toggle")
        .ghost()
        .size(MONITOR_ICON_SIZE)
        .p_0()
        .text_color(color)
        .child(
            Icon::new(CirclePlayIcon)
                .with_size(MONITOR_ICON_SIZE)
                .text_color(color),
        )
        .tooltip("Preview")
        .toggled(selected)
        .on_click(move |_, window, cx| {
            (on_click)(window, cx);
        })
}

fn monitor_button(
    on_click: Rc<dyn Fn(&mut Window, &mut App)>,
    muted: Hsla,
    selected: bool,
) -> impl IntoElement {
    Button::new("monitor-tab")
        .ghost()
        .size(MONITOR_ICON_SIZE)
        .p_0()
        .text_color(muted)
        .child(Icon::new(MonitorSpeakerIcon).with_size(MONITOR_ICON_SIZE))
        .tooltip(if selected {
            "Hide Monitor"
        } else {
            "Show Monitor"
        })
        .selected(selected)
        .on_click(move |_, window, cx| {
            (on_click)(window, cx);
        })
}

fn layout_dropdown(picker: LayoutPicker, muted: Hsla) -> impl IntoElement {
    let current = picker.current.clone();
    let label = current.clone().unwrap_or_else(|| "—".into());
    let tooltip = picker
        .choices
        .iter()
        .find(|(name, _)| current.as_deref() == Some(name.as_str()))
        .map(|(_, description)| description.clone())
        .filter(|description| !description.is_empty())
        .unwrap_or_else(|| "Channel layout".into());
    let choices = picker.choices;
    let on_choose = picker.on_choose;
    Button::new("channel-layout")
        .ghost()
        .xsmall()
        .text_xs()
        .text_color(muted)
        .label(label)
        .tooltip(tooltip)
        .dropdown_menu(
            move |mut menu: PopupMenu, _: &mut Window, _: &mut gpui_kit::Context<PopupMenu>| {
                for (name, _) in choices.clone() {
                    let checked = current.as_deref() == Some(name.as_str());
                    let on_choose = on_choose.clone();
                    let chosen = name.clone();
                    let item_label = name.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().text_xs().text_color(muted).child(item_label.clone())
                        })
                        .checked(checked)
                        .on_click(move |_, window, cx| {
                            (on_choose)(&chosen, window, cx);
                        }),
                    );
                }
                menu
            },
        )
}

fn format_bit_depth(bits: Option<u32>) -> String {
    match bits {
        Some(bits) => format!("{bits}-bit"),
        None => "—".into(),
    }
}

fn format_duration(secs: f64) -> String {
    if secs.is_nan() || secs.is_infinite() {
        return "0.00s".into();
    }
    if secs < 60.0 {
        format!("{secs:.2}s")
    } else {
        let m = (secs / 60.0).floor() as u32;
        let s = secs % 60.0;
        format!("{m}m {s:05.2}s")
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_byte_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(1_572_864), "1.5 MB");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(0.0), "0.00s");
        assert_eq!(format_duration(1.5), "1.50s");
        assert_eq!(format_duration(61.5), "1m 01.50s");
    }

    #[test]
    fn formats_bit_depth() {
        assert_eq!(format_bit_depth(Some(16)), "16-bit");
        assert_eq!(format_bit_depth(None), "—");
    }
}
