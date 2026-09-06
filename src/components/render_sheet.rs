// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use gpui::{
    div, rems, App, AppContext as _, Context, Entity, ExternalPaths, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Window,
};
use gpui_component::{
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex, ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
};

use crate::model::composition::Composition;
use crate::render::{
    encoder, encoders, format_rate, snap_format, EncodeSpec, PcmFormat, RenderJob, RATE_PRESETS,
};

const LABEL_WIDTH: gpui::Rems = rems(7.);
const VALUE_WIDTH: gpui::Rems = rems(11.);

pub struct RenderSheet {
    encoder_id: String,
    sample_format: Option<PcmFormat>,
    sample_rate: u32,
    channels_selected: Vec<bool>,
    channel_labels: Vec<String>,
    directory: Entity<InputState>,
    filename: Entity<InputState>,
}

impl RenderSheet {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let directory = cx.new(|cx| InputState::new(window, cx));
        let filename = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(&directory, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&filename, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        Self {
            encoder_id: "wav".into(),
            sample_format: Some(PcmFormat::S24),
            sample_rate: 48_000,
            channels_selected: vec![true],
            channel_labels: vec!["Mono".into()],
            directory,
            filename,
        }
    }

    pub fn configure(
        &mut self,
        composition: &Composition,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let encoder = encoder("wav").expect("wav encoder");
        self.encoder_id = encoder.id().into();
        let bits = composition
            .pool()
            .first()
            .and_then(|media| media.bits_per_sample);
        self.sample_format = snap_format(
            encoder.capabilities(),
            bits.and_then(PcmFormat::from_bits).or(Some(PcmFormat::S24)),
        );
        self.sample_rate = composition.sample_rate().max(1);
        let count = composition.channel_count().max(1);
        self.channels_selected = vec![true; count];
        self.channel_labels = (0..count).map(|ch| composition.channel_label(ch)).collect();
        let filename = format!("{}.{}", composition.display_name(), encoder.extension());
        self.directory.update(cx, |input, cx| {
            input.set_value(directory.to_string_lossy().into_owned(), window, cx);
        });
        self.filename.update(cx, |input, cx| {
            input.set_value(filename, window, cx);
        });
        cx.notify();
    }

    fn selected_count(&self) -> u16 {
        self.channels_selected.iter().filter(|on| **on).count() as u16
    }

    fn spec(&self) -> EncodeSpec {
        EncodeSpec {
            sample_rate: self.sample_rate,
            sample_format: self.sample_format,
            channel_count: self.selected_count().max(1),
        }
    }

    fn set_encoder(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(encoder) = encoder(id) else {
            return;
        };
        self.encoder_id = encoder.id().into();
        self.sample_format = snap_format(encoder.capabilities(), self.sample_format);
        let stem = Path::new(&self.filename.read(cx).value().to_string())
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "render".into());
        self.filename.update(cx, |input, cx| {
            input.set_value(format!("{stem}.{}", encoder.extension()), window, cx);
        });
        cx.notify();
    }

    pub fn can_render(&self, cx: &App) -> bool {
        let Some(encoder) = encoder(&self.encoder_id) else {
            return false;
        };
        if self.selected_count() == 0 {
            return false;
        }
        if self.directory.read(cx).value().trim().is_empty() {
            return false;
        }
        if self.filename.read(cx).value().trim().is_empty() {
            return false;
        }
        encoder.supports(&self.spec())
    }

    pub fn job(&self, cx: &App) -> Option<RenderJob> {
        if !self.can_render(cx) {
            return None;
        }
        let directory = PathBuf::from(self.directory.read(cx).value().to_string());
        let filename = self.filename.read(cx).value().to_string();
        let dest = directory.join(filename.trim());
        Some(RenderJob {
            encoder_id: self.encoder_id.clone(),
            spec: self.spec(),
            channel_indices: self
                .channels_selected
                .iter()
                .enumerate()
                .filter_map(|(i, on)| on.then_some(i))
                .collect(),
            dest,
        })
    }

    fn prompt_directory(&self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.directory.update(cx, |input, cx| {
                        input.set_value(path.to_string_lossy().into_owned(), window, cx);
                    });
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn apply_dropped_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf())
        };
        self.directory.update(cx, |input, cx| {
            input.set_value(dir.to_string_lossy().into_owned(), window, cx);
        });
        cx.notify();
    }
}

impl Render for RenderSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let drop_highlight = theme.secondary;
        let spec = self.spec();
        let encoder_id = self.encoder_id.clone();
        let current_encoder = encoder(&encoder_id);
        let caps = current_encoder.map(|enc| enc.capabilities());
        let stores_format = caps.is_some_and(|caps| caps.stores_sample_format());
        let format_label = if stores_format {
            self.sample_format
                .map(PcmFormat::label)
                .unwrap_or("n/a")
                .to_string()
        } else {
            "n/a".into()
        };
        let rate_label = format_rate(self.sample_rate);
        let codec_label = current_encoder
            .map(|enc| enc.label().to_string())
            .unwrap_or_else(|| encoder_id.clone());
        let rate_choices = rate_choices(self.sample_rate);

        let codec_menu = {
            let this = cx.entity();
            let encoder_id = encoder_id.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                for encoder in encoders() {
                    let disabled = !encoder.supports(&spec);
                    let id = encoder.id().to_string();
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(encoder.label())
                            .disabled(disabled)
                            .checked(id == encoder_id)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.set_encoder(&id, window, cx);
                                });
                            }),
                    );
                }
                menu
            }
        };

        let format_menu = {
            let this = cx.entity();
            let encoder_id = encoder_id.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                let Some(encoder) = encoder(&encoder_id) else {
                    return menu;
                };
                let caps = encoder.capabilities();
                if !caps.stores_sample_format() {
                    return menu.label("Vorbis does not store PCM format");
                }
                for format in PcmFormat::ALL {
                    let mut probe = spec;
                    probe.sample_format = Some(format);
                    let disabled = !encoder.supports(&probe);
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format.label())
                            .disabled(disabled)
                            .checked(spec.sample_format == Some(format))
                            .on_click(move |_, _, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.sample_format = Some(format);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            }
        };

        let rate_menu = {
            let this = cx.entity();
            let encoder_id = encoder_id.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                let Some(encoder) = encoder(&encoder_id) else {
                    return menu;
                };
                for rate in rate_choices.iter().copied() {
                    let mut probe = spec;
                    probe.sample_rate = rate;
                    let disabled = !encoder.supports(&probe);
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format_rate(rate))
                            .disabled(disabled)
                            .checked(spec.sample_rate == rate)
                            .on_click(move |_, _, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.sample_rate = rate;
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            }
        };

        v_flex()
            .gap_4()
            .w_full()
            .child(
                h_flex()
                    .gap_6()
                    .w_full()
                    .items_start()
                    .child(
                        div().flex_none().child(group(
                            "Format",
                            v_flex()
                                .gap_2()
                                .child(form_row(
                                    "Type",
                                    muted,
                                    Some(VALUE_WIDTH),
                                    dropdown("render-type", codec_label, false, codec_menu),
                                ))
                                .child(form_row(
                                    "Format",
                                    muted,
                                    Some(VALUE_WIDTH),
                                    dropdown(
                                        "render-format",
                                        format_label,
                                        !stores_format,
                                        format_menu,
                                    ),
                                ))
                                .child(form_row(
                                    "Sample Rate",
                                    muted,
                                    Some(VALUE_WIDTH),
                                    dropdown("render-rate", rate_label, false, rate_menu),
                                )),
                        )),
                    )
                    .child(div().flex_1().min_w_0().child(group(
                        "Channels",
                        h_flex().gap_3().flex_wrap().children(
                            self.channel_labels.iter().enumerate().map(|(i, label)| {
                                let checked =
                                    self.channels_selected.get(i).copied().unwrap_or(false);
                                Checkbox::new(("render-ch", i as u64))
                                    .label(label.clone())
                                    .checked(checked)
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                        if let Some(slot) = this.channels_selected.get_mut(i) {
                                            *slot = *checked;
                                        }
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ))),
            )
            .child(group(
                "Location",
                v_flex()
                    .gap_2()
                    .w_full()
                    .child(form_row(
                        "Directory",
                        muted,
                        None,
                        h_flex()
                            .gap_2()
                            .w_full()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .drag_over::<ExternalPaths>(move |style, _, _, _| {
                                        style.bg(drop_highlight)
                                    })
                                    .on_drop(cx.listener(
                                        |this, paths: &ExternalPaths, window, cx| {
                                            if let Some(path) = paths.paths().first() {
                                                this.apply_dropped_path(path, window, cx);
                                            }
                                        },
                                    ))
                                    .child(Input::new(&self.directory).small().w_full()),
                            )
                            .child(
                                Button::new("browse-dir")
                                    .outline()
                                    .small()
                                    .label("Browse…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.prompt_directory(window, cx);
                                    })),
                            ),
                    ))
                    .child(form_row(
                        "Name",
                        muted,
                        None,
                        Input::new(&self.filename).small().w_full(),
                    )),
            ))
    }
}

fn group(title: &'static str, child: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(div().text_sm().font_semibold().child(title))
        .child(child)
}

fn form_row(
    label: &'static str,
    muted: Hsla,
    value_width: Option<gpui::Rems>,
    control: impl IntoElement,
) -> impl IntoElement {
    let row = h_flex().gap_3().items_center().child(
        div()
            .w(LABEL_WIDTH)
            .flex_none()
            .flex()
            .justify_end()
            .child(div().text_sm().text_color(muted).child(label)),
    );
    match value_width {
        Some(width) => row.child(div().w(width).flex_none().child(control)),
        None => row.w_full().child(div().flex_1().min_w_0().child(control)),
    }
}

fn dropdown(
    id: &'static str,
    value: String,
    disabled: bool,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    Button::new(id)
        .outline()
        .small()
        .w_full()
        .label(value)
        .disabled(disabled)
        .dropdown_menu(menu)
}

fn rate_choices(current: u32) -> Vec<u32> {
    let mut rates = RATE_PRESETS.to_vec();
    if current > 0 && !rates.contains(&current) {
        rates.push(current);
        rates.sort_unstable();
    }
    rates
}
