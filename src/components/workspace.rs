// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gpui::{
    div, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, Styled as _, Window,
};
use gpui_component::{
    dock::{BasePanel, Panel, PanelEvent},
    v_flex,
};

use crate::components::transport::Transport;
use crate::components::waveform::WaveformDisplay;
use crate::model::document::BufferDocument;
use crate::model::DocumentId;
use crate::playback::TransportState;

type ActivatedFn = Rc<dyn Fn(DocumentId, &mut Window, &mut App)>;

pub struct WorkspacePanel {
    document_id: DocumentId,
    document: Entity<BufferDocument>,
    waveform: Entity<WaveformDisplay>,
    transport_state: TransportState,
    looping: bool,
    on_activated: Option<ActivatedFn>,
    last_doc_fingerprint: Option<(u64, String)>,
}

impl WorkspacePanel {
    pub fn new(
        document_id: DocumentId,
        document: Entity<BufferDocument>,
        waveform: Entity<WaveformDisplay>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&document, |this, entity, cx| {
            let composition = entity.read(cx).composition.read().unwrap();
            let next = (composition.frames(), composition.display_name());
            drop(composition);
            if this.last_doc_fingerprint.as_ref() == Some(&next) {
                return;
            }
            this.last_doc_fingerprint = Some(next);
            cx.notify();
        })
        .detach();
        Self {
            document_id,
            document,
            waveform,
            transport_state: TransportState::Stopped,
            looping: false,
            on_activated: None,
            last_doc_fingerprint: None,
        }
    }

    pub fn set_on_activated(&mut self, handler: ActivatedFn) {
        self.on_activated = Some(handler);
    }

    pub fn sync_transport(&mut self, state: TransportState, looping: bool, cx: &mut Context<Self>) {
        if self.transport_state == state && self.looping == looping {
            return;
        }
        self.transport_state = state;
        self.looping = looping;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for WorkspacePanel {}

impl Focusable for WorkspacePanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.waveform.focus_handle(cx)
    }
}

impl BasePanel for WorkspacePanel {
    fn panel_name(&self) -> &'static str {
        "WorkspacePanel"
    }

    fn closable(&self, _: &App) -> bool {
        true
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !active {
            return;
        }
        if let Some(handler) = self.on_activated.clone() {
            let id = self.document_id;
            handler(id, window, cx);
        }
    }
}

impl Panel for WorkspacePanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.document
            .read(cx)
            .composition
            .read()
            .unwrap()
            .display_name()
    }

    fn tab_name(&self, cx: &App) -> Option<gpui::SharedString> {
        Some(
            self.document
                .read(cx)
                .composition
                .read()
                .unwrap()
                .display_name()
                .into(),
        )
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for WorkspacePanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.waveform.clone()),
            )
            .child(Transport::new(self.transport_state, self.looping))
    }
}
