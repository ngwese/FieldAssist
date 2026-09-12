// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gpui_kit::{
    div, prelude::FluentBuilder as _, Action, App, Context, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, WeakEntity, Window,
};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    v_flex,
};
use field_ui_components::Transport;

use crate::app::AppView;
use crate::commands::{
    TransportEnd, TransportHome, TransportLoop, TransportNext, TransportPlayPause,
    TransportPrevious,
};
use crate::components::drop_overlay::file_drop_overlay;
use crate::model::document::BufferDocument;
use crate::model::DocumentId;
use crate::playback::TransportState;
use field_ui_components::WaveformDisplay;

type ActivatedFn = Rc<dyn Fn(DocumentId, &mut Window, &mut App)>;

pub struct WorkspacePanel {
    document_id: DocumentId,
    document: Entity<BufferDocument>,
    waveform: Entity<WaveformDisplay<BufferDocument>>,
    transport_state: TransportState,
    looping: bool,
    on_activated: Option<ActivatedFn>,
    last_doc_fingerprint: Option<(u64, String)>,
    app: WeakEntity<AppView>,
}

impl WorkspacePanel {
    pub fn new(
        document_id: DocumentId,
        document: Entity<BufferDocument>,
        waveform: Entity<WaveformDisplay<BufferDocument>>,
        app: WeakEntity<AppView>,
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
            app,
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

    fn tab_name(&self, cx: &App) -> Option<gpui_kit::SharedString> {
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let layout = self
            .app
            .upgrade()
            .and_then(|app| app.update(cx, |this, cx| this.sync_file_drop_layout(cx)));
        v_flex()
            .size_full()
            .child(
                div()
                    .id("workspace-waveform")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .drag_over::<ExternalPaths>(move |style, _, _, cx| {
                        if let Some(app) = app.upgrade() {
                            app.update(cx, |this, cx| this.ensure_file_drop_layout(cx));
                        }
                        style
                    })
                    .child(self.waveform.clone())
                    .when_some(layout, |this, layout| {
                        this.child(file_drop_overlay(layout, self.app.clone()))
                    }),
            )
            .child({
                let playing = self.transport_state == TransportState::Playing;
                let looping = self.looping;
                fn action_factory<A: Action + Clone + 'static>(action: A) -> field_ui_components::TransportAction {
                    Rc::new(move || Box::new(action.clone()) as Box<dyn Action>)
                }
                Transport::new(
                    playing,
                    looping,
                    action_factory(TransportHome),
                    action_factory(TransportPrevious),
                    action_factory(TransportPlayPause),
                    action_factory(TransportNext),
                    action_factory(TransportEnd),
                    action_factory(TransportLoop),
                )
            })
    }
}
