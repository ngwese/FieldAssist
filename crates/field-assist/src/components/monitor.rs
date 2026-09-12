// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use gpui_kit::{
    div, prelude::FluentBuilder as _, px, relative, App, AppContext as _, ClickEvent, Context,
    Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, WeakEntity, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderEvent, SliderScale, SliderState},
    v_flex, ActiveTheme as _, Icon, IconNamed, Sizable as _,
};

use crate::app::AppView;
use crate::model::document::BufferDocument;
use crate::monitor::{
    menu_items_from_meta, meta_value, parse_ui_json, FaustUiNode, FaustUiRoot, MonitorChain,
};

pub struct MonitorPanel {
    app: WeakEntity<AppView>,
    document: Option<Entity<BufferDocument>>,
    focus_handle: FocusHandle,
    sliders: HashMap<String, Entity<SliderState>>,
    slider_defaults: HashMap<String, f32>,
    slider_subs: Vec<Subscription>,
    bound_json: Option<&'static str>,
    _document_observe: Option<Subscription>,
}

impl MonitorPanel {
    pub fn new(app: WeakEntity<AppView>, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            document: None,
            focus_handle: cx.focus_handle(),
            sliders: HashMap::new(),
            slider_defaults: HashMap::new(),
            slider_subs: Vec::new(),
            bound_json: None,
            _document_observe: None,
        }
    }

    pub fn set_target(&mut self, document: Entity<BufferDocument>, cx: &mut Context<Self>) {
        self.document = Some(document);
        self.bound_json = None;
        if let Some(document) = &self.document {
            self._document_observe = Some(cx.observe(document, |_, _, cx| cx.notify()));
        }
        cx.notify();
    }

    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.document = None;
        self._document_observe = None;
        self.sliders.clear();
        self.slider_defaults.clear();
        self.slider_subs.clear();
        self.bound_json = None;
        cx.notify();
    }

    fn ensure_sliders(&mut self, json: Option<&'static str>, cx: &mut Context<Self>) {
        if self.bound_json == json {
            return;
        }
        self.bound_json = json;
        self.sliders.clear();
        self.slider_defaults.clear();
        self.slider_subs.clear();
        let Some(json) = json else {
            return;
        };
        let Ok(root) = parse_ui_json(json) else {
            return;
        };
        bind_sliders(
            &root.ui,
            &self.app,
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
        crate::components::dock_skin::DETAIL_TAB_MONITOR
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MonitorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let app = self.app.clone();
        let output_selected = app
            .upgrade()
            .and_then(|app| app.read(cx).output_device().map(str::to_string));

        v_flex()
            .id("monitor-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.render_chain_scroll(muted, theme.accent, theme.secondary, cx))
            .child(output_section(output_selected, app, muted, theme.border))
    }
}

impl MonitorPanel {
    fn render_chain_scroll(
        &mut self,
        muted: gpui_kit::Hsla,
        accent: gpui_kit::Hsla,
        secondary: gpui_kit::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(document) = self.document.clone() else {
            return v_flex()
                .id("monitor-chain-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .into_any_element();
        };

        let (chain_id, playback, channel_count, labels, params_pinned) = {
            let doc = document.read(cx);
            let composition = doc.composition.read().unwrap();
            let n = composition.channel_count();
            let labels: Vec<String> = (0..n).map(|ch| composition.channel_label(ch)).collect();
            (
                composition.monitor_chain().map(str::to_string),
                composition.playback_channels().map(|ch| ch.to_vec()),
                n,
                labels,
                doc.monitor_params_pinned,
            )
        };
        let chain = chain_id.as_deref().and_then(MonitorChain::parse);
        let ui_json = self
            .app
            .upgrade()
            .and_then(|app| app.read(cx).monitor_ui_json());
        self.ensure_sliders(ui_json, cx);
        let meters = self
            .app
            .upgrade()
            .map(|app| app.read(cx).monitor_meters())
            .unwrap_or_default();
        let params: HashMap<String, f32> = ui_json
            .and_then(|json| parse_ui_json(json).ok())
            .map(|root| {
                collect_live_addresses(&root)
                    .into_iter()
                    .filter_map(|address| {
                        let value = self
                            .app
                            .upgrade()
                            .and_then(|app| app.read(cx).monitor_param(&address));
                        value.map(|value| (address, value))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let selected_count = match playback.as_deref() {
            Some(channels) => channels.len(),
            None => channel_count,
        };
        let expected = chain.map(MonitorChain::num_inputs);
        let app = self.app.clone();
        let chain_label = chain
            .map(MonitorChain::label)
            .unwrap_or("Direct")
            .to_string();

        v_flex()
            .id("monitor-chain-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .gap_3()
            .child(chain_header(params_pinned, app.clone(), muted, cx))
            .child(chain_dropdown(
                chain_label,
                chain_id.clone(),
                app.clone(),
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
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .children(labels.iter().enumerate().map(|(i, label)| {
                        let checked = match playback.as_deref() {
                            Some(channels) => channels.contains(&i),
                            None => true,
                        };
                        let app = app.clone();
                        Checkbox::new(("monitor-ch", i as u64))
                            .label(label.clone())
                            .checked(checked)
                            .on_click(move |enabled: &bool, window, cx| {
                                if let Some(app) = app.upgrade() {
                                    app.update(cx, |this, cx| {
                                        this.set_playback_channel(i, *enabled, window, cx);
                                    });
                                }
                            })
                    })),
            )
            .children(
                ui_json
                    .and_then(|json| parse_ui_json(json).ok())
                    .map(|root| {
                        render_schema(
                            &root,
                            &self.sliders,
                            &self.slider_defaults,
                            &params,
                            &meters,
                            app.clone(),
                            muted,
                            accent,
                            secondary,
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }
}

fn section_label(text: &'static str, muted: gpui_kit::Hsla) -> impl IntoElement {
    div().text_xs().text_color(muted).child(text)
}

fn output_section(
    selected: Option<String>,
    app: WeakEntity<AppView>,
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
        .child(output_dropdown(selected, app, muted))
}

fn output_dropdown(
    selected: Option<String>,
    app: WeakEntity<AppView>,
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
                let app_default = app.clone();
                let default_selected = selected.is_none();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().text_xs().text_color(muted).child("System Default")
                    })
                    .checked(default_selected)
                    .on_click(move |_, window, cx| {
                        if let Some(app) = app_default.upgrade() {
                            app.update(cx, |this, cx| {
                                this.select_output_device(None, window, cx);
                            });
                        }
                    }),
                );
                if let Ok(devices) = crate::playback::list_output_devices() {
                    for info in devices {
                        let app = app.clone();
                        let name = info.name.clone();
                        let checked = selected.as_deref() == Some(name.as_str());
                        let item_label = name.clone();
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div().text_xs().text_color(muted).child(item_label.clone())
                            })
                            .checked(checked)
                            .on_click(move |_, window, cx| {
                                if let Some(app) = app.upgrade() {
                                    app.update(cx, |this, cx| {
                                        this.select_output_device(Some(&name), window, cx);
                                    });
                                }
                            }),
                        );
                    }
                }
                menu
            },
        )
}

struct PinIcon;

impl IconNamed for PinIcon {
    fn path(self) -> gpui_kit::SharedString {
        "icons/pin.svg".into()
    }
}

fn chain_header(
    pinned: bool,
    app: WeakEntity<AppView>,
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
                .on_click(move |_, _, cx| {
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |this, cx| {
                            this.toggle_monitor_params_pin(cx);
                        });
                    }
                }),
        )
}

fn menu_dropdown(
    label: String,
    address: String,
    items: Vec<(String, f32)>,
    current: f32,
    app: WeakEntity<AppView>,
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
                    let app = app.clone();
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
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, _| {
                                    this.set_monitor_param(&address, value);
                                });
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
    app: WeakEntity<AppView>,
    muted: gpui_kit::Hsla,
) -> impl IntoElement {
    Button::new("monitor-chain")
        .outline()
        .small()
        .w_full()
        .label(label)
        .dropdown_menu(
            move |mut menu: PopupMenu, _: &mut Window, _: &mut gpui_kit::Context<PopupMenu>| {
                let app_direct = app.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().text_xs().text_color(muted).child("Direct")
                    })
                    .checked(current.is_none())
                    .on_click(move |_, window, cx| {
                        if let Some(app) = app_direct.upgrade() {
                            app.update(cx, |this, cx| {
                                this.set_monitor_chain(None, window, cx);
                            });
                        }
                    }),
                );
                for chain in MonitorChain::ALL {
                    let app = app.clone();
                    let id = chain.id();
                    let checked = current.as_deref() == Some(id);
                    let item_label = chain.label().to_string();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().text_xs().text_color(muted).child(item_label.clone())
                        })
                        .checked(checked)
                        .on_click(move |_, window, cx| {
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, cx| {
                                    this.set_monitor_chain(Some(id), window, cx);
                                });
                            }
                        }),
                    );
                }
                menu
            },
        )
}

fn bind_sliders(
    nodes: &[FaustUiNode],
    app: &WeakEntity<AppView>,
    sliders: &mut HashMap<String, Entity<SliderState>>,
    defaults: &mut HashMap<String, f32>,
    subs: &mut Vec<Subscription>,
    cx: &mut Context<MonitorPanel>,
) {
    for node in nodes {
        match node {
            FaustUiNode::VGroup { items, .. }
            | FaustUiNode::HGroup { items, .. }
            | FaustUiNode::TGroup { items, .. } => {
                bind_sliders(items, app, sliders, defaults, subs, cx);
            }
            FaustUiNode::NEntry {
                address,
                init,
                meta,
                ..
            } if menu_items_from_meta(meta).is_some() => {
                defaults.insert(address.clone(), *init);
            }
            FaustUiNode::HSlider {
                address,
                init,
                min,
                max,
                step,
                meta,
                ..
            }
            | FaustUiNode::VSlider {
                address,
                init,
                min,
                max,
                step,
                meta,
                ..
            }
            | FaustUiNode::NEntry {
                address,
                init,
                min,
                max,
                step,
                meta,
                ..
            } => {
                let start = live_or_init(app, address, *init, cx);
                let mut state = SliderState::new()
                    .max(*max)
                    .min(*min)
                    .step((*step).abs().max(0.0001))
                    .default_value(start);
                if *min > 0.0
                    && meta_value(meta, "scale")
                        .is_some_and(|scale| scale.eq_ignore_ascii_case("log"))
                {
                    state = state.scale(SliderScale::Logarithmic);
                }
                let entity = cx.new(|_| state);
                let address_for_sub = address.clone();
                let app = app.clone();
                subs.push(cx.subscribe(&entity, move |_, _, event: &SliderEvent, cx| {
                    let value = match event {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                    };
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |this, _| {
                            this.set_monitor_param(&address_for_sub, value);
                        });
                    }
                }));
                sliders.insert(address.clone(), entity);
                defaults.insert(address.clone(), *init);
            }
            _ => {}
        }
    }
}

fn live_or_init(app: &WeakEntity<AppView>, address: &str, init: f32, cx: &App) -> f32 {
    app.upgrade()
        .and_then(|app| app.read(cx).monitor_param(address))
        .unwrap_or(init)
}

fn collect_live_addresses(root: &FaustUiRoot) -> Vec<String> {
    let mut out = Vec::new();
    collect_live_nodes(&root.ui, &mut out);
    out
}

fn collect_live_nodes(nodes: &[FaustUiNode], out: &mut Vec<String>) {
    for node in nodes {
        match node {
            FaustUiNode::VGroup { items, .. }
            | FaustUiNode::HGroup { items, .. }
            | FaustUiNode::TGroup { items, .. } => collect_live_nodes(items, out),
            FaustUiNode::HSlider { address, .. }
            | FaustUiNode::VSlider { address, .. }
            | FaustUiNode::NEntry { address, .. }
            | FaustUiNode::Checkbox { address, .. }
            | FaustUiNode::Button { address, .. }
            | FaustUiNode::HBargraph { address, .. }
            | FaustUiNode::VBargraph { address, .. } => out.push(address.clone()),
            FaustUiNode::Other => {}
        }
    }
}

fn render_schema(
    root: &FaustUiRoot,
    sliders: &HashMap<String, Entity<SliderState>>,
    defaults: &HashMap<String, f32>,
    params: &HashMap<String, f32>,
    meters: &HashMap<String, f32>,
    app: WeakEntity<AppView>,
    muted: gpui_kit::Hsla,
    accent: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    cx: &App,
) -> impl IntoElement {
    v_flex().gap_2().children(root.ui.iter().map(|node| {
        render_node(
            node,
            sliders,
            defaults,
            params,
            meters,
            app.clone(),
            muted,
            accent,
            secondary,
            true,
            cx,
        )
    }))
}

fn render_node(
    node: &FaustUiNode,
    sliders: &HashMap<String, Entity<SliderState>>,
    defaults: &HashMap<String, f32>,
    params: &HashMap<String, f32>,
    meters: &HashMap<String, f32>,
    app: WeakEntity<AppView>,
    muted: gpui_kit::Hsla,
    accent: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    skip_outer_label: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    match node {
        FaustUiNode::VGroup { label, items }
        | FaustUiNode::HGroup { label, items }
        | FaustUiNode::TGroup { label, items } => {
            let body = v_flex().gap_2().children(items.iter().map(|child| {
                render_node(
                    child,
                    sliders,
                    defaults,
                    params,
                    meters,
                    app.clone(),
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
        FaustUiNode::NEntry {
            label,
            address,
            init,
            meta,
            ..
        } if menu_items_from_meta(meta).is_some() => {
            let items = menu_items_from_meta(meta).unwrap_or_default();
            let value = params.get(address).copied().unwrap_or(*init);
            menu_dropdown(
                label.clone(),
                address.clone(),
                items,
                value,
                app.clone(),
                muted,
            )
            .into_any_element()
        }
        FaustUiNode::HSlider {
            label,
            address,
            meta,
            ..
        }
        | FaustUiNode::VSlider {
            label,
            address,
            meta,
            ..
        }
        | FaustUiNode::NEntry {
            label,
            address,
            meta,
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
                            format_param(value, meta_value(meta, "unit")),
                            muted,
                            slider.clone(),
                            address.clone(),
                            default,
                            app.clone(),
                        )),
                )
                .when_some(slider, |this, state| {
                    this.child(resettable_slider(
                        id,
                        state,
                        address.clone(),
                        default,
                        app.clone(),
                    ))
                })
                .into_any_element()
        }
        FaustUiNode::Checkbox {
            label,
            address,
            init,
            ..
        } => {
            let checked = params.get(address).copied().unwrap_or(*init) > 0.5;
            let address = address.clone();
            let id = address_id(&address);
            Checkbox::new(("monitor-param", id))
                .label(label.clone())
                .checked(checked)
                .on_click(move |enabled: &bool, _, cx| {
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |this, _| {
                            this.set_monitor_param(&address, if *enabled { 1.0 } else { 0.0 });
                        });
                    }
                })
                .into_any_element()
        }
        FaustUiNode::HBargraph {
            label,
            address,
            min,
            max,
            meta,
            ..
        }
        | FaustUiNode::VBargraph {
            label,
            address,
            min,
            max,
            meta,
            ..
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
                                .child(format_value_with_unit(value, 1, meta_value(meta, "unit"))),
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
        FaustUiNode::Button { .. } | FaustUiNode::Other => div().into_any_element(),
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
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    address.hash(&mut hasher);
    hasher.finish()
}

fn reset_slider(
    state: &Entity<SliderState>,
    address: &str,
    default: f32,
    app: &WeakEntity<AppView>,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        state.set_value(default, window, cx);
    });
    if let Some(app) = app.upgrade() {
        app.update(cx, |this, _| {
            this.set_monitor_param(address, default);
        });
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
    app: WeakEntity<AppView>,
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
            reset_slider(state, &address, default, &app, window, cx);
        })
        .child(text)
}

fn resettable_slider(
    id: u64,
    state: Entity<SliderState>,
    address: String,
    default: Option<f32>,
    app: WeakEntity<AppView>,
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
            reset_slider(&thumb_state, &address, default, &app, window, cx);
            cx.stop_propagation();
        })
        .child(Slider::new(&state).horizontal().w_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_units_from_faust_meta() {
        assert_eq!(format_value_with_unit(0.0, 2, Some("dB")), "0.00 dB");
        assert_eq!(format_value_with_unit(700.0, 2, Some("Hz")), "700.00 Hz");
        assert_eq!(format_value_with_unit(100.0, 2, Some("%")), "100.00%");
        assert_eq!(format_value_with_unit(-12.5, 1, Some("dB")), "-12.5 dB");
        assert_eq!(format_param(None, Some("dB")), "—");
        assert_eq!(format_param(Some(1.5), None), "1.50");
    }
}
