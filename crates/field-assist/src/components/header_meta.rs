// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui_kit::{
    div, prelude::FluentBuilder as _, App, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window,
};
use gpui_kit::component::{h_flex, ActiveTheme as _};

use crate::model::document::BufferDocument;
use crate::playback::TransportState;
use field_ui_components::WaveformDisplay;

/// Title-bar session readout. Lives in its own view so hover, playhead, and
/// selection updates do not rebuild the window chrome or docks.
pub struct HeaderMeta {
    document: Option<Entity<BufferDocument>>,
    waveform: Option<Entity<WaveformDisplay<BufferDocument>>>,
    transport: TransportState,
    last_hover: Option<usize>,
    focus_handle: FocusHandle,
    _document_observe: Option<Subscription>,
    _waveform_observe: Option<Subscription>,
}

impl HeaderMeta {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            document: None,
            waveform: None,
            transport: TransportState::Stopped,
            last_hover: None,
            focus_handle: cx.focus_handle(),
            _document_observe: None,
            _waveform_observe: None,
        }
    }

    pub fn set_target(
        &mut self,
        document: Option<Entity<BufferDocument>>,
        waveform: Option<Entity<WaveformDisplay<BufferDocument>>>,
        cx: &mut Context<Self>,
    ) {
        self.document = document;
        self.waveform = waveform;
        self.last_hover = None;
        self._document_observe = self
            .document
            .as_ref()
            .map(|document| cx.observe(document, |_, _, cx| cx.notify()));
        self._waveform_observe = self.waveform.as_ref().map(|waveform| {
            cx.observe(waveform, |this, wave, cx| {
                let hover = wave.read(cx).hover_sample();
                if this.last_hover == hover {
                    return;
                }
                this.last_hover = hover;
                cx.notify();
            })
        });
        cx.notify();
    }

    pub fn set_transport(&mut self, state: TransportState, cx: &mut Context<Self>) {
        if self.transport == state {
            return;
        }
        self.transport = state;
        cx.notify();
    }
}

impl Focusable for HeaderMeta {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HeaderMeta {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let meta = self
            .document
            .as_ref()
            .map(|document| format_header_meta(document.read(cx), self.transport))
            .unwrap_or_else(|| {
                format!("No file open  ·  {}", transport_state_label(self.transport))
            });
        let hover_meta =
            self.document
                .as_ref()
                .zip(self.waveform.as_ref())
                .and_then(|(document, waveform)| {
                    let sample = waveform.read(cx).hover_sample()?;
                    Some(format_hover_meta(document.read(cx), sample))
                });

        h_flex()
            .id("header-meta")
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(meta),
            )
            .when_some(hover_meta, |this, hover_meta| {
                this.child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .whitespace_nowrap()
                        .child(hover_meta),
                )
            })
    }
}

fn format_secs(secs: f64) -> String {
    if secs.is_nan() || secs.is_infinite() {
        return "0.000000s".into();
    }
    format!("{secs:.6}s")
}

fn transport_state_label(state: TransportState) -> &'static str {
    match state {
        TransportState::Stopped => "Stopped",
        TransportState::Playing => "Playing",
        TransportState::Paused => "Paused",
    }
}

fn format_hover_meta(doc: &BufferDocument, sample: usize) -> String {
    format!(
        "hover {}  ·  {sample} smp",
        format_secs(doc.sample_to_secs(sample))
    )
}

fn format_header_meta(doc: &BufferDocument, transport: TransportState) -> String {
    if !doc.is_loaded() {
        return format!("No file open  ·  {}", transport_state_label(transport));
    }

    let mut parts = vec![transport_state_label(transport).to_string()];

    if let Some(pos) = &doc.current_position {
        parts.push(format!(
            "pos {}",
            format_secs(doc.sample_to_secs(pos.sample))
        ));
    }

    if let Some((start, end)) = doc.selection.bounding_span() {
        if end > start {
            let start_secs = doc.sample_to_secs(start);
            let end_secs = doc.sample_to_secs(end);
            let len_samples = end - start + 1;
            let len_secs = len_samples as f64 / f64::from(doc.sample_rate().max(1));
            parts.push(format!(
                "region {}–{}",
                format_secs(start_secs),
                format_secs(end_secs)
            ));
            parts.push(format!("len {}", format_secs(len_secs)));
            parts.push(format!("{len_samples} smp"));
        }
    }

    parts.join("  ·  ")
}
