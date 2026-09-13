// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Monitor / DSP parameter panel driven by host snapshots and callbacks.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderEvent, SliderScale, SliderState},
    v_flex, ActiveTheme as _, Icon, IconNamed, Sizable as _,
};
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, relative, App, AppContext as _, ClickEvent, Context,
    Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::param_ui::{ChainChoice, MonitorSnapshot, ParamUiNode};

/// Host callbacks for monitor panel interactions.
#[derive(Clone)]
pub struct MonitorCallbacks {
    /// Set the monitor chain (`None` = Direct).
    pub set_chain: Rc<dyn Fn(Option<&str>, &mut Window, &mut App)>,
    /// Write a parameter value.
    pub set_param: Rc<dyn Fn(&str, f32, &mut App)>,
    /// Toggle parameter pin.
    pub toggle_pin: Rc<dyn Fn(&mut Window, &mut App)>,
    /// Enable or disable a playback channel index.
    pub set_playback_channel: Rc<dyn Fn(usize, bool, &mut Window, &mut App)>,
    /// Select an output device (`None` = system default).
    pub select_output: Rc<dyn Fn(Option<&str>, &mut Window, &mut App)>,
}

/// Provides a [`MonitorSnapshot`] on each render.
pub type MonitorSnapshotProvider = Rc<dyn Fn(&App) -> Option<MonitorSnapshot>>;

struct PinIcon;

impl IconNamed for PinIcon {
    fn path(self) -> SharedString {
        "icons/pin.svg".into()
    }
}

/// Detail-dock monitor panel.
pub struct MonitorPanel {
    title: SharedString,
    snapshot: Option<MonitorSnapshotProvider>,
    callbacks: Option<MonitorCallbacks>,
    focus_handle: FocusHandle,
    sliders: HashMap<String, Entity<SliderState>>,
    slider_defaults: HashMap<String, f32>,
    slider_subs: Vec<Subscription>,
    bound_schema: Option<u64>,
}

impl MonitorPanel {
    /// Create a monitor panel with the given dock tab title.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            title: title.into(),
            snapshot: None,
            callbacks: None,
            focus_handle: cx.focus_handle(),
            sliders: HashMap::new(),
            slider_defaults: HashMap::new(),
            slider_subs: Vec::new(),
            bound_schema: None,
        }
    }

    /// Bind a snapshot provider and interaction callbacks.
    pub fn set_host(
        &mut self,
        snapshot: MonitorSnapshotProvider,
        callbacks: MonitorCallbacks,
        cx: &mut Context<Self>,
    ) {
        self.snapshot = Some(snapshot);
        self.callbacks = Some(callbacks);
        self.bound_schema = None;
        self.sliders.clear();
        self.slider_defaults.clear();
        self.slider_subs.clear();
        cx.notify();
    }

    /// Clear the host binding.
    pub fn clear_host(&mut self, cx: &mut Context<Self>) {
        self.snapshot = None;
        self.callbacks = None;
        self.sliders.clear();
        self.slider_defaults.clear();
        self.slider_subs.clear();
        self.bound_schema = None;
        cx.notify();
    }

    fn ensure_sliders(&mut self, snap: &MonitorSnapshot, cx: &mut Context<Self>) {
        if self.bound_schema == Some(snap.schema_id) {
            return;
        }
        self.bound_schema = Some(snap.schema_id);
        self.sliders.clear();
        self.slider_defaults.clear();
        self.slider_subs.clear();
        let Some(callbacks) = self.callbacks.clone() else {
            return;
        };
        bind_sliders(
            &snap.params_ui,
            &snap.live_params,
            &callbacks,
            &mut self.sliders,
            &mut self.slider_defaults,
            &mut self.slider_subs,
            cx,
        );
    }
}

impl EventEmitter<PanelEvent> for MonitorPanel {}

impl Focusable for MonitorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for MonitorPanel {
    fn panel_name(&self) -> &'static str {
        "MonitorPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for MonitorPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MonitorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let snap = self.snapshot.as_ref().and_then(|provider| provider(cx));
        let output_selected = snap.as_ref().and_then(|s| s.output_device.clone());
        let output_devices = snap
            .as_ref()
            .map(|s| s.output_devices.clone())
            .unwrap_or_default();
        let callbacks = self.callbacks.clone();

        v_flex()
            .id("monitor-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.render_chain_scroll(snap, muted, theme.accent, theme.secondary, cx))
            .child(output_section(
                output_selected,
                output_devices,
                callbacks,
                muted,
                theme.border,
            ))
    }
}

impl MonitorPanel {
    fn render_chain_scroll(
        &mut self,
        snap: Option<MonitorSnapshot>,
        muted: gpui_kit::Hsla,
        accent: gpui_kit::Hsla,
        secondary: gpui_kit::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(snap) = snap else {
            return v_flex()
                .id("monitor-chain-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .into_any_element();
        };
        self.ensure_sliders(&snap, cx);
        let callbacks = self.callbacks.clone();
        let selected_count = match snap.playback_channels.as_deref() {
            Some(channels) => channels.len(),
            None => snap.channel_labels.len(),
        };
        let expected = snap.expected_inputs;

        v_flex()
            .id("monitor-chain-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .gap_3()
            .child(chain_header(
                snap.params_pinned,
                callbacks.clone(),
                muted,
                cx,
            ))
            .child(chain_dropdown(
                snap.chain_label.clone(),
                snap.chain_id.clone(),
                snap.chain_choices.clone(),
                callbacks.clone(),
                muted,
            ))
            .child(section_label("Playback channels", muted))
            .when_some(expected, |this, n_in| {
                let mismatch = selected_count != n_in;
                this.child(div().text_xs().text_color(muted).child(format!(
                    "Chain expects {n_in} input{} · {selected_count} selected{}",
                    if n_in == 1 { "" } else { "s" },
                    if mismatch { " (pad / truncate)" } else { "" }
                )))
            })
            .child(h_flex().gap_2().flex_wrap().children(
                snap.channel_labels.iter().enumerate().map(|(i, label)| {
                    let checked = match snap.playback_channels.as_deref() {
                        Some(channels) => channels.contains(&i),
                        None => true,
                    };
                    let callbacks = callbacks.clone();
                    Checkbox::new(("monitor-ch", i as u64))
                        .label(label.clone())
                        .checked(checked)
                        .on_click(move |enabled: &bool, window, cx| {
                            if let Some(cb) = &callbacks {
                                (cb.set_playback_channel)(i, *enabled, window, cx);
                            }
                        })
                }),
            ))
            .child(render_schema(
                &snap.params_ui,
                &self.sliders,
                &self.slider_defaults,
                &snap.live_params,
                &snap.meters,
                callbacks,
                muted,
                accent,
                secondary,
                cx,
            ))
            .into_any_element()
    }
}

fn section_label(text: &'static str, muted: gpui_kit::Hsla) -> impl IntoElement {
    div().text_xs().text_color(muted).child(text)
}

fn output_section(
    selected: Option<String>,
    devices: Vec<String>,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
) -> impl IntoElement {
    v_flex()
        .flex_none()
        .w_full()
        .border_t_1()
        .border_color(border)
        .px_2()
        .py_2()
        .gap_1()
        .child(section_label("Output", muted))
        .child(output_dropdown(selected, devices, callbacks, muted))
}

fn output_dropdown(
    selected: Option<String>,
    devices: Vec<String>,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
) -> impl IntoElement {
    let label = selected
        .clone()
        .unwrap_or_else(|| "System Default".to_string());
    Button::new("monitor-output")
        .outline()
        .small()
        .w_full()
        .label(label)
        .dropdown_menu(
            move |mut menu: PopupMenu, _: &mut Window, _: &mut gpui_kit::Context<PopupMenu>| {
                let callbacks_default = callbacks.clone();
                let default_selected = selected.is_none();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().text_xs().text_color(muted).child("System Default")
                    })
                    .checked(default_selected)
                    .on_click(move |_, window, cx| {
                        if let Some(cb) = &callbacks_default {
                            (cb.select_output)(None, window, cx);
                        }
                    }),
                );
                for name in &devices {
                    let callbacks = callbacks.clone();
                    let checked = selected.as_deref() == Some(name.as_str());
                    let item_label = name.clone();
                    let name = name.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().text_xs().text_color(muted).child(item_label.clone())
                        })
                        .checked(checked)
                        .on_click(move |_, window, cx| {
                            if let Some(cb) = &callbacks {
                                (cb.select_output)(Some(&name), window, cx);
                            }
                        }),
                    );
                }
                menu
            },
        )
}

fn chain_header(
    pinned: bool,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
    cx: &App,
) -> impl IntoElement {
    let color = if pinned { cx.theme().cyan } else { muted };
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .child(section_label("Chain", muted))
        .child(
            Button::new("monitor-pin")
                .ghost()
                .xsmall()
                .p_0()
                .text_color(color)
                .child(
                    Icon::new(PinIcon)
                        .with_size(gpui_kit::px(14.))
                        .text_color(color),
                )
                .tooltip(if pinned {
                    "Unpin monitor parameters"
                } else {
                    "Pin monitor parameters"
                })
                .toggled(pinned)
                .on_click(move |_, window, cx| {
                    if let Some(cb) = &callbacks {
                        (cb.toggle_pin)(window, cx);
                    }
                }),
        )
}

fn menu_dropdown(
    label: String,
    address: String,
    items: Vec<(String, f32)>,
    current: f32,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
) -> impl IntoElement {
    let selected = items
        .iter()
        .min_by(|a, b| {
            (a.1 - current)
                .abs()
                .partial_cmp(&(b.1 - current).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| format!("{current:.0}"));
    let id = address_id(&address);
    v_flex().gap_1().child(div().text_xs().child(label)).child(
        Button::new(("monitor-menu", id))
            .outline()
            .small()
            .w_full()
            .label(selected)
            .dropdown_menu(move |mut menu, _, _| {
                for (name, value) in &items {
                    let callbacks = callbacks.clone();
                    let address = address.clone();
                    let name_el = name.clone();
                    let checked = (current - value).abs() < 0.01;
                    let value = *value;
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().text_xs().text_color(muted).child(name_el.clone())
                        })
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            if let Some(cb) = &callbacks {
                                (cb.set_param)(&address, value, cx);
                            }
                        }),
                    );
                }
                menu
            }),
    )
}

fn chain_dropdown(
    label: String,
    current: Option<String>,
    choices: Vec<ChainChoice>,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
) -> impl IntoElement {
    Button::new("monitor-chain")
        .outline()
        .small()
        .w_full()
        .label(label)
        .dropdown_menu(
            move |mut menu: PopupMenu, _: &mut Window, _: &mut gpui_kit::Context<PopupMenu>| {
                for choice in &choices {
                    let callbacks = callbacks.clone();
                    let id = choice.id.clone();
                    let checked = current == id;
                    let item_label = choice.label.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().text_xs().text_color(muted).child(item_label.clone())
                        })
                        .checked(checked)
                        .on_click(move |_, window, cx| {
                            if let Some(cb) = &callbacks {
                                (cb.set_chain)(id.as_deref(), window, cx);
                            }
                        }),
                    );
                }
                menu
            },
        )
}

fn bind_sliders(
    nodes: &[ParamUiNode],
    live: &HashMap<String, f32>,
    callbacks: &MonitorCallbacks,
    sliders: &mut HashMap<String, Entity<SliderState>>,
    defaults: &mut HashMap<String, f32>,
    subs: &mut Vec<Subscription>,
    cx: &mut Context<MonitorPanel>,
) {
    for node in nodes {
        match node {
            ParamUiNode::Group { items, .. } => {
                bind_sliders(items, live, callbacks, sliders, defaults, subs, cx);
            }
            ParamUiNode::Menu { address, init, .. } => {
                defaults.insert(address.clone(), *init);
            }
            ParamUiNode::Slider {
                address,
                init,
                min,
                max,
                step,
                logarithmic,
                ..
            } => {
                let start = live.get(address).copied().unwrap_or(*init);
                let mut state = SliderState::new()
                    .max(*max)
                    .min(*min)
                    .step((*step).abs().max(0.0001))
                    .default_value(start);
                if *logarithmic {
                    state = state.scale(SliderScale::Logarithmic);
                }
                let entity = cx.new(|_| state);
                let address_for_sub = address.clone();
                let set_param = callbacks.set_param.clone();
                subs.push(cx.subscribe(&entity, move |_, _, event: &SliderEvent, cx| {
                    let value = match event {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                    };
                    set_param(&address_for_sub, value, cx);
                }));
                sliders.insert(address.clone(), entity);
                defaults.insert(address.clone(), *init);
            }
            _ => {}
        }
    }
}

fn render_schema(
    nodes: &[ParamUiNode],
    sliders: &HashMap<String, Entity<SliderState>>,
    defaults: &HashMap<String, f32>,
    params: &HashMap<String, f32>,
    meters: &HashMap<String, f32>,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
    accent: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    cx: &App,
) -> impl IntoElement {
    v_flex().gap_2().children(nodes.iter().map(|node| {
        render_node(
            node,
            sliders,
            defaults,
            params,
            meters,
            callbacks.clone(),
            muted,
            accent,
            secondary,
            true,
            cx,
        )
    }))
}

fn render_node(
    node: &ParamUiNode,
    sliders: &HashMap<String, Entity<SliderState>>,
    defaults: &HashMap<String, f32>,
    params: &HashMap<String, f32>,
    meters: &HashMap<String, f32>,
    callbacks: Option<MonitorCallbacks>,
    muted: gpui_kit::Hsla,
    accent: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    skip_outer_label: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    match node {
        ParamUiNode::Group { label, items } => {
            let body = v_flex().gap_2().children(items.iter().map(|child| {
                render_node(
                    child,
                    sliders,
                    defaults,
                    params,
                    meters,
                    callbacks.clone(),
                    muted,
                    accent,
                    secondary,
                    false,
                    cx,
                )
            }));
            if skip_outer_label {
                body.into_any_element()
            } else {
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(muted).child(label.clone()))
                    .child(body)
                    .into_any_element()
            }
        }
        ParamUiNode::Menu {
            label,
            address,
            init,
            items,
        } => {
            let value = params.get(address).copied().unwrap_or(*init);
            menu_dropdown(
                label.clone(),
                address.clone(),
                items.clone(),
                value,
                callbacks,
                muted,
            )
            .into_any_element()
        }
        ParamUiNode::Slider {
            label,
            address,
            unit,
            ..
        } => {
            let slider = sliders.get(address).cloned();
            let default = defaults.get(address).copied();
            let value = slider
                .as_ref()
                .map(|state| state.read(cx).value().start())
                .or_else(|| params.get(address).copied());
            let id = address_id(address);
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .child(div().text_xs().child(label.clone()))
                        .child(resettable_value(
                            id,
                            format_param(value, unit.as_deref()),
                            muted,
                            slider.clone(),
                            address.clone(),
                            default,
                            callbacks.clone(),
                        )),
                )
                .when_some(slider, |this, state| {
                    this.child(resettable_slider(
                        id,
                        state,
                        address.clone(),
                        default,
                        callbacks.clone(),
                    ))
                })
                .into_any_element()
        }
        ParamUiNode::Checkbox {
            label,
            address,
            init,
        } => {
            let checked = params.get(address).copied().unwrap_or(*init) > 0.5;
            let address = address.clone();
            let id = address_id(&address);
            Checkbox::new(("monitor-param", id))
                .label(label.clone())
                .checked(checked)
                .on_click(move |enabled: &bool, _, cx| {
                    if let Some(cb) = &callbacks {
                        (cb.set_param)(&address, if *enabled { 1.0 } else { 0.0 }, cx);
                    }
                })
                .into_any_element()
        }
        ParamUiNode::Bargraph {
            label,
            address,
            min,
            max,
            unit,
        } => {
            let value = meters.get(address).copied().unwrap_or(*min);
            let span = (*max - *min).abs().max(1e-6);
            let t = ((value - *min) / span).clamp(0.0, 1.0);
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .child(div().text_xs().child(label.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(format_value_with_unit(value, 1, unit.as_deref())),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_sm()
                        .bg(secondary)
                        .child(div().h_full().w(relative(t)).rounded_sm().bg(accent)),
                )
                .into_any_element()
        }
        ParamUiNode::Other => div().into_any_element(),
    }
}

fn format_param(value: Option<f32>, unit: Option<&str>) -> String {
    match value {
        Some(value) => format_value_with_unit(value, 2, unit),
        None => "—".into(),
    }
}

fn format_value_with_unit(value: f32, precision: usize, unit: Option<&str>) -> String {
    let number = format!("{value:.precision$}");
    match unit.map(str::trim).filter(|unit| !unit.is_empty()) {
        Some("%") => format!("{number}%"),
        Some(unit) => format!("{number} {unit}"),
        None => number,
    }
}

fn address_id(address: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    address.hash(&mut hasher);
    hasher.finish()
}

fn reset_slider(
    state: &Entity<SliderState>,
    address: &str,
    default: f32,
    callbacks: &Option<MonitorCallbacks>,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        state.set_value(default, window, cx);
    });
    if let Some(cb) = callbacks {
        (cb.set_param)(address, default, cx);
    }
}

fn click_on_thumb(position: gpui_kit::Point<gpui_kit::Pixels>, state: &SliderState) -> bool {
    let bounds = state.bounds();
    if bounds.size.width <= px(0.) {
        return false;
    }
    let t = state.percentage().end;
    let x = bounds.left() + bounds.size.width * t;
    let y = bounds.center().y;
    (position.x - x).abs() <= px(12.) && (position.y - y).abs() <= px(12.)
}

fn resettable_value(
    id: u64,
    text: String,
    muted: gpui_kit::Hsla,
    slider: Option<Entity<SliderState>>,
    address: String,
    default: Option<f32>,
    callbacks: Option<MonitorCallbacks>,
) -> impl IntoElement {
    div()
        .id(("monitor-param-value", id))
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .on_click(move |event: &ClickEvent, window, cx| {
            if event.click_count() < 2 {
                return;
            }
            let (Some(state), Some(default)) = (slider.as_ref(), default) else {
                return;
            };
            reset_slider(state, &address, default, &callbacks, window, cx);
        })
        .child(text)
}

fn resettable_slider(
    id: u64,
    state: Entity<SliderState>,
    address: String,
    default: Option<f32>,
    callbacks: Option<MonitorCallbacks>,
) -> impl IntoElement {
    let thumb_state = state.clone();
    div()
        .id(("monitor-param-slider", id))
        .w_full()
        .capture_any_mouse_down(move |event: &MouseDownEvent, window, cx| {
            if event.button != MouseButton::Left || event.click_count < 2 {
                return;
            }
            let Some(default) = default else {
                return;
            };
            if !click_on_thumb(event.position, thumb_state.read(cx)) {
                return;
            }
            reset_slider(&thumb_state, &address, default, &callbacks, window, cx);
            cx.stop_propagation();
        })
        .child(Slider::new(&state).horizontal().w_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_units_from_meta() {
        assert_eq!(format_value_with_unit(0.0, 2, Some("dB")), "0.00 dB");
        assert_eq!(format_value_with_unit(700.0, 2, Some("Hz")), "700.00 Hz");
        assert_eq!(format_value_with_unit(100.0, 2, Some("%")), "100.00%");
        assert_eq!(format_value_with_unit(-12.5, 1, Some("dB")), "-12.5 dB");
        assert_eq!(format_param(None, Some("dB")), "—");
        assert_eq!(format_param(Some(1.5), None), "1.50");
    }
}
