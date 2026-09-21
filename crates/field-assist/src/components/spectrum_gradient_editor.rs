// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Spectrum heatmap gradient editor for the Waveform settings page.

use field_ui_components::{SpectrumGradient, SPECTRUM_GRADIENT_DB_FLOOR};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    canvas, div, fill, point, px, size, App, AppContext as _, Bounds, Context, DispatchPhase,
    Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render, Rgba, SharedString,
    Styled as _, TextAlign, Window,
};

use crate::settings;

const TRACK_HEIGHT: f32 = 28.0;
const CHIP_SIZE: f32 = 22.0;
const EDITOR_PAD_X: f32 = 12.0;

/// Interactive five-stop spectrum colormap editor.
pub struct SpectrumGradientEditor {
    pickers: [Entity<ColorPickerState>; 5],
    drag: Option<DragState>,
    track_bounds: Bounds<Pixels>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

#[derive(Clone, Copy)]
struct DragState {
    index: usize,
    /// Pointer x within the track at drag start (for hit feedback only).
    _origin_x: f32,
}

impl SpectrumGradientEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let gradient = settings::store(cx)
            .settings
            .waveform
            .spectrum_gradient_enum();
        let mut pickers = Vec::with_capacity(5);
        let mut subscriptions = Vec::with_capacity(5);
        for (i, stop) in gradient.stops.iter().enumerate() {
            let color = rgb_to_hsla(stop.rgb);
            let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(color));
            let index = i;
            subscriptions.push(cx.subscribe(
                &picker,
                move |_this, _picker, event: &ColorPickerEvent, cx| {
                    let ColorPickerEvent::Change(Some(color)) = event else {
                        return;
                    };
                    let rgb = hsla_to_rgb(*color);
                    let _ = settings::update_and_save(cx, |s| {
                        s.waveform.set_spectrum_stop_rgb(index, rgb);
                    });
                    crate::app::apply_waveform_default_from_settings(cx);
                    cx.notify();
                },
            ));
            pickers.push(picker);
        }
        let pickers: [Entity<ColorPickerState>; 5] = pickers
            .try_into()
            .unwrap_or_else(|_| unreachable!("exactly five spectrum stops"));
        Self {
            pickers,
            drag: None,
            track_bounds: Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(1.0), px(1.0)),
            },
            _subscriptions: subscriptions,
        }
    }

    fn sync_pickers_from_settings(&self, window: &mut Window, cx: &mut Context<Self>) {
        let gradient = settings::store(cx)
            .settings
            .waveform
            .spectrum_gradient_enum();
        for (picker, stop) in self.pickers.iter().zip(gradient.stops.iter()) {
            let color = rgb_to_hsla(stop.rgb);
            picker.update(cx, |state, cx| {
                if state.value() != Some(color) {
                    state.set_value(color, window, cx);
                }
            });
        }
    }

    fn set_stop_db_from_x(&mut self, index: usize, x: f32, cx: &mut Context<Self>) {
        // Endpoints stay pinned at −∞ / 0 dB; only middle stops slide.
        if !(1..=3).contains(&index) {
            return;
        }
        let width = self.track_bounds.size.width.as_f32().max(1.0);
        let local = ((x - self.track_bounds.origin.x.as_f32()) / width).clamp(0.0, 1.0);
        let db = SPECTRUM_GRADIENT_DB_FLOOR + local * (0.0 - SPECTRUM_GRADIENT_DB_FLOOR);
        let _ = settings::update_and_save(cx, |s| {
            s.waveform.set_spectrum_stop_db(index, db);
        });
        crate::app::apply_waveform_default_from_settings(cx);
        cx.notify();
    }
}

/// Window-level listeners so drag continues after the pointer leaves the handle.
fn install_drag_listeners(entity: Entity<SpectrumGradientEditor>, window: &mut Window) {
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            entity.update(cx, |this, cx| {
                let Some(drag) = this.drag else {
                    return;
                };
                this.set_stop_db_from_x(drag.index, event.position.x.as_f32(), cx);
            });
        }
    });
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            entity.update(cx, |this, cx| {
                if this.drag.is_some() {
                    this.drag = None;
                    cx.notify();
                }
            });
        }
    });
}

impl Render for SpectrumGradientEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        install_drag_listeners(cx.entity(), window);
        self.sync_pickers_from_settings(window, cx);
        let gradient = settings::store(cx)
            .settings
            .waveform
            .spectrum_gradient_enum();
        let theme = cx.theme();
        let entity = cx.entity();
        let track_w = self.track_bounds.size.width.as_f32().max(1.0);
        let drag_index = self.drag.map(|d| d.index);

        v_flex()
            .gap_2()
            .w_full()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Spectrum Gradient"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Low → high intensity stops (−∞ … 0 dB)"),
            )
            .child(
                div()
                    .id("spectrum-gradient-track")
                    .w_full()
                    .h(px(TRACK_HEIGHT + CHIP_SIZE + 8.0))
                    .relative()
                    .px(px(EDITOR_PAD_X))
                    .child({
                        let g = gradient;
                        let entity = entity.clone();
                        canvas(
                            move |bounds, _, cx| {
                                entity.update(cx, |this, _| {
                                    this.track_bounds = Bounds {
                                        origin: point(
                                            bounds.origin.x + px(EDITOR_PAD_X),
                                            bounds.origin.y + px(CHIP_SIZE * 0.5),
                                        ),
                                        size: size(
                                            (bounds.size.width - px(EDITOR_PAD_X * 2.0))
                                                .max(px(1.0)),
                                            px(TRACK_HEIGHT),
                                        ),
                                    };
                                });
                                bounds
                            },
                            move |bounds, _, window, _| {
                                let x0 = bounds.origin.x.as_f32() + EDITOR_PAD_X;
                                let y0 = bounds.origin.y.as_f32() + CHIP_SIZE * 0.5;
                                let w = (bounds.size.width.as_f32() - EDITOR_PAD_X * 2.0).max(1.0);
                                let h = TRACK_HEIGHT;
                                // Preview strip sampled across the track.
                                let cols = w.ceil().max(1.0) as usize;
                                for col in 0..cols {
                                    let t = col as f32 / (cols.saturating_sub(1).max(1) as f32);
                                    let db = SPECTRUM_GRADIENT_DB_FLOOR
                                        + t * (0.0 - SPECTRUM_GRADIENT_DB_FLOOR);
                                    let (r, gch, b) = g.sample(db);
                                    let color = Hsla::from(Rgba {
                                        r: r as f32 / 255.0,
                                        g: gch as f32 / 255.0,
                                        b: b as f32 / 255.0,
                                        a: 1.0,
                                    });
                                    window.paint_quad(fill(
                                        Bounds {
                                            origin: point(px(x0 + col as f32), px(y0)),
                                            size: size(px(1.0), px(h)),
                                        },
                                        color,
                                    ));
                                }
                            },
                        )
                        .size_full()
                        .absolute()
                        .inset_0()
                    })
                    .children((0..5).map(|i| {
                        let stop = gradient.stops[i];
                        let t = ((stop.db - SPECTRUM_GRADIENT_DB_FLOOR)
                            / (0.0 - SPECTRUM_GRADIENT_DB_FLOOR))
                            .clamp(0.0, 1.0);
                        let left = EDITOR_PAD_X + t * track_w - CHIP_SIZE * 0.5;
                        let picker = self.pickers[i].clone();
                        let movable = (1..=3).contains(&i);
                        div()
                            .id(("spectrum-stop", i))
                            .absolute()
                            .left(px(left))
                            .top(px(0.0))
                            .w(px(CHIP_SIZE))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_0p5()
                            .child(
                                ColorPicker::new(&picker)
                                    .with_size(Size::Small)
                                    .accessibility_label(SharedString::from(format!(
                                        "Spectrum stop {}",
                                        i + 1
                                    ))),
                            )
                            .when(movable, |this| {
                                this.child(
                                    div()
                                        .id(("spectrum-stop-handle", i))
                                        .w(px(10.0))
                                        .h(px(10.0))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(theme.background)
                                        .cursor_ew_resize()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                                this.drag = Some(DragState {
                                                    index: i,
                                                    _origin_x: ev.position.x.as_f32(),
                                                });
                                                cx.notify();
                                            }),
                                        ),
                                )
                            })
                    })),
            )
            .child({
                let legend = h_flex()
                    .id("spectrum-gradient-legend")
                    .w_full()
                    .h(px(16.0))
                    .px(px(EDITOR_PAD_X))
                    .relative()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().absolute().left(px(EDITOR_PAD_X)).child("−∞"))
                    .child(div().absolute().right(px(EDITOR_PAD_X)).child("0 dB"));
                if let Some(i) = drag_index.filter(|i| (1..=3).contains(i)) {
                    let stop = gradient.stops[i];
                    let t = ((stop.db - SPECTRUM_GRADIENT_DB_FLOOR)
                        / (0.0 - SPECTRUM_GRADIENT_DB_FLOOR))
                        .clamp(0.0, 1.0);
                    // Center the label roughly under the chip; keep inside the track.
                    let label = format_stop_db(stop.db);
                    let label_w = 52.0_f32;
                    let center = EDITOR_PAD_X + t * track_w;
                    let left = (center - label_w * 0.5)
                        .clamp(EDITOR_PAD_X, EDITOR_PAD_X + track_w - label_w);
                    legend.child(
                        div()
                            .absolute()
                            .left(px(left))
                            .w(px(label_w))
                            .text_center()
                            .font_weight(FontWeight::MEDIUM)
                            .text_align(TextAlign::Center)
                            .child(label),
                    )
                } else {
                    legend
                }
            })
    }
}

fn format_stop_db(db: f32) -> SharedString {
    SharedString::from(format!("{db:.0} dB"))
}

fn rgb_to_hsla(rgb: [f32; 3]) -> Hsla {
    Hsla::from(Rgba {
        r: rgb[0].clamp(0.0, 1.0),
        g: rgb[1].clamp(0.0, 1.0),
        b: rgb[2].clamp(0.0, 1.0),
        a: 1.0,
    })
}

fn hsla_to_rgb(color: Hsla) -> [f32; 3] {
    let rgba = color.to_rgb();
    [rgba.r, rgba.g, rgba.b]
}

/// Reset handler dirty check for the settings item.
pub fn spectrum_gradient_is_dirty(cx: &App) -> bool {
    settings::store(cx)
        .settings
        .waveform
        .spectrum_gradient_enum()
        != SpectrumGradient::classic()
}

/// Reset the gradient to classic defaults and refresh open waveforms.
pub fn spectrum_gradient_reset(_window: &mut Window, cx: &mut App) {
    let _ = settings::update_and_save(cx, |s| {
        s.waveform
            .set_spectrum_gradient(SpectrumGradient::classic());
    });
    crate::app::apply_waveform_default_from_settings(cx);
}
