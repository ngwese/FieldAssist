// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use gpui::{
    div, prelude::FluentBuilder as _, px, relative, App, AppContext as _, Context, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window,
};
use gpui_component::{
    button::Button,
    checkbox::Checkbox,
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderEvent, SliderScale, SliderState},
    v_flex, ActiveTheme as _, Sizable as _,
};

use crate::app::AppView;
use crate::model::document::BufferDocument;
use crate::monitor::{meta_value, parse_ui_json, FaustUiNode, FaustUiRoot, MonitorChain};

pub struct MonitorPanel {
    app: WeakEntity<AppView>,
    document: Option<Entity<BufferDocument>>,
    focus_handle: FocusHandle,
    sliders: HashMap<String, Entity<SliderState>>,
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
            slider_subs: Vec::new(),
            bound_json: None,
            _document_observe: None,
        }
    }

    pub fn set_target(&mut self, document: Entity<BufferDocument>, cx: &mut Context<Self>) {
        self.document = Some(document);
        if let Some(document) = &self.document {
            self._document_observe = Some(cx.observe(document, |_, _, cx| cx.notify()));
        }
        cx.notify();
    }

    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.document = None;
        self._document_observe = None;
        self.sliders.clear();
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
        "Monitor"
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MonitorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let Some(document) = self.document.clone() else {
            return v_flex()
                .id("monitor-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .into_any_element();
        };

        let (chain_id, playback, channel_count, labels) = {
            let composition = document.read(cx).composition.read().unwrap();
            let n = composition.channel_count();
            let labels: Vec<String> = (0..n).map(|ch| composition.channel_label(ch)).collect();
            (
                composition.monitor_chain().map(str::to_string),
                composition.playback_channels().map(|ch| ch.to_vec()),
                n,
                labels,
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
            .id("monitor-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .gap_3()
            .child(section_label("Chain", muted))
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
                            &params,
                            &meters,
                            app.clone(),
                            muted,
                            theme.accent,
                            theme.secondary,
                        )
                    }),
            )
            .into_any_element()
    }
}

fn section_label(text: &'static str, muted: gpui::Hsla) -> impl IntoElement {
    div().text_xs().text_color(muted).child(text)
}

fn chain_dropdown(
    label: String,
    current: Option<String>,
    app: WeakEntity<AppView>,
    muted: gpui::Hsla,
) -> impl IntoElement {
    Button::new("monitor-chain")
        .outline()
        .small()
        .w_full()
        .label(label)
        .dropdown_menu(
            move |mut menu: PopupMenu, _: &mut Window, _: &mut gpui::Context<PopupMenu>| {
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
    subs: &mut Vec<Subscription>,
    cx: &mut Context<MonitorPanel>,
) {
    for node in nodes {
        match node {
            FaustUiNode::VGroup { items, .. }
            | FaustUiNode::HGroup { items, .. }
            | FaustUiNode::TGroup { items, .. } => {
                bind_sliders(items, app, sliders, subs, cx);
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
                let mut state = SliderState::new()
                    .min(*min)
                    .max(*max)
                    .step((*step).abs().max(0.0001))
                    .default_value(*init);
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
            }
            _ => {}
        }
    }
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
    params: &HashMap<String, f32>,
    meters: &HashMap<String, f32>,
    app: WeakEntity<AppView>,
    muted: gpui::Hsla,
    accent: gpui::Hsla,
    secondary: gpui::Hsla,
) -> impl IntoElement {
    v_flex().gap_2().children(root.ui.iter().map(|node| {
        render_node(
            node,
            sliders,
            params,
            meters,
            app.clone(),
            muted,
            accent,
            secondary,
            true,
        )
    }))
}

fn render_node(
    node: &FaustUiNode,
    sliders: &HashMap<String, Entity<SliderState>>,
    params: &HashMap<String, f32>,
    meters: &HashMap<String, f32>,
    app: WeakEntity<AppView>,
    muted: gpui::Hsla,
    accent: gpui::Hsla,
    secondary: gpui::Hsla,
    skip_outer_label: bool,
) -> gpui::AnyElement {
    match node {
        FaustUiNode::VGroup { label, items }
        | FaustUiNode::HGroup { label, items }
        | FaustUiNode::TGroup { label, items } => {
            let body = v_flex().gap_2().children(items.iter().map(|child| {
                render_node(
                    child,
                    sliders,
                    params,
                    meters,
                    app.clone(),
                    muted,
                    accent,
                    secondary,
                    false,
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
        FaustUiNode::HSlider { label, address, .. }
        | FaustUiNode::VSlider { label, address, .. }
        | FaustUiNode::NEntry { label, address, .. } => {
            let value = params.get(address).copied();
            let slider = sliders.get(address).cloned();
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .child(div().text_xs().child(label.clone()))
                        .child(div().text_xs().text_color(muted).child(format_param(value))),
                )
                .when_some(slider, |this, state| {
                    this.child(Slider::new(&state).horizontal().w_full())
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
            let id = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                address.hash(&mut hasher);
                hasher.finish()
            };
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
            ..
        }
        | FaustUiNode::VBargraph {
            label,
            address,
            min,
            max,
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
                                .child(format!("{value:.1}")),
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

fn format_param(value: Option<f32>) -> String {
    match value {
        Some(value) => format!("{value:.2}"),
        None => "—".into(),
    }
}
