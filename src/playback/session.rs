// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::sync::{Arc, RwLock};

use anyhow::Result;
use cpal::Device;

use crate::model::buffer::ChannelScope;
use crate::model::composition::Composition;
use crate::model::document::BufferDocument;
use crate::model::Buffer;
use crate::monitor::MonitorChain;

use super::anchors::{collect_anchors, next_anchor, previous_anchor_near};
use super::engine::PlaybackEngine;
use super::playhead::Playhead;
use super::provider::{PlaybackDataProvider, SharedCompositionProvider};
use super::transport::{Transport, TransportState};

pub struct PlaybackSession {
    provider: Arc<SharedCompositionProvider>,
    playhead: Playhead,
    transport: Transport,
    engine: PlaybackEngine,
    anchors: Vec<usize>,
    active_region: Option<(usize, usize)>,
}

impl PlaybackSession {
    pub fn open(device: &Device, composition: Arc<RwLock<Composition>>) -> Result<Self> {
        let provider = Arc::new(SharedCompositionProvider::new(composition));
        let playhead = Playhead::new(provider.clone());
        let engine = PlaybackEngine::open(device, provider.clone())?;
        Ok(Self {
            provider,
            playhead,
            transport: Transport::new(),
            engine,
            anchors: Vec::new(),
            active_region: None,
        })
    }

    pub fn bind_composition(&self, composition: Arc<RwLock<Composition>>) {
        self.provider.bind(composition.clone());
        self.sync_monitor(&composition.read().unwrap());
    }

    pub fn sync_monitor(&self, composition: &Composition) {
        let chain = composition.monitor_chain().and_then(MonitorChain::parse);
        let channels = composition.playback_channels().map(|ch| ch.to_vec());
        self.engine.shared.set_monitor(chain, channels);
    }

    pub fn commit_monitor_for_document(&self, doc: &mut BufferDocument) {
        let snapshot = self.engine.shared.flush_monitor_working();
        if doc.monitor_params_pinned {
            doc.pinned_monitor_params = snapshot;
        } else {
            self.engine.shared.merge_monitor_into_session(&snapshot);
        }
    }

    pub fn commit_monitor_to_session(&self) {
        let snapshot = self.engine.shared.flush_monitor_working();
        self.engine.shared.merge_monitor_into_session(&snapshot);
    }

    pub fn load_monitor_for_document(&self, doc: &BufferDocument) {
        let mut working = self.engine.shared.monitor_session_params();
        if doc.monitor_params_pinned {
            for (chain, params) in &doc.pinned_monitor_params {
                working.insert(*chain, params.clone());
            }
        }
        let chain = doc
            .composition
            .read()
            .unwrap()
            .monitor_chain()
            .and_then(MonitorChain::parse);
        let channels = doc
            .composition
            .read()
            .unwrap()
            .playback_channels()
            .map(|ch| ch.to_vec());
        self.engine
            .shared
            .replace_monitor_working(chain, channels, working);
    }

    pub fn load_session_monitor(&self, composition: &Composition) {
        let chain = composition.monitor_chain().and_then(MonitorChain::parse);
        let channels = composition.playback_channels().map(|ch| ch.to_vec());
        let working = self.engine.shared.monitor_session_params();
        self.engine
            .shared
            .replace_monitor_working(chain, channels, working);
    }

    pub fn set_monitor_param(&self, address: &str, value: f32) {
        self.engine.shared.set_monitor_param(address, value);
    }

    pub fn monitor_param(&self, address: &str) -> Option<f32> {
        self.engine.shared.monitor_param(address)
    }

    pub fn monitor_meter(&self, address: &str) -> Option<f32> {
        self.engine.shared.monitor_meter(address)
    }

    pub fn monitor_ui_json(&self) -> Option<&'static str> {
        self.engine.shared.monitor_ui_json()
    }

    pub fn monitor_meters(&self) -> std::collections::HashMap<String, f32> {
        self.engine.shared.monitor_meters()
    }

    pub fn transport_state(&self) -> TransportState {
        self.transport.state()
    }

    pub fn looping(&self) -> bool {
        self.playhead.looping()
    }

    pub fn position(&self) -> usize {
        if self.transport.is_playing() {
            self.engine.shared.position()
        } else {
            self.playhead.position()
        }
    }

    pub fn refresh_anchors(&mut self, doc: &BufferDocument) {
        self.anchors = collect_anchors(doc);
    }

    fn refresh_anchors_from_doc(&mut self, doc: &BufferDocument) {
        self.refresh_anchors(doc);
    }

    fn apply_to_engine(&self) {
        self.engine.shared.set_position(self.playhead.position());
        self.engine.shared.set_looping(self.playhead.looping());
        self.engine
            .shared
            .set_in_out(self.playhead.in_point(), self.playhead.out_point());
        self.engine.shared.set_transport(self.transport.state());
    }

    fn sync_playhead_from_engine(&mut self) {
        if self.transport.is_playing() {
            self.playhead.set_position(self.engine.shared.position());
        }
    }

    fn region_bounds_from_doc(doc: &BufferDocument) -> Option<(usize, usize)> {
        doc.selection
            .bounding_span()
            .filter(|(start, end)| end > start)
    }

    pub fn sync_from_document(&mut self, doc: &BufferDocument) {
        self.refresh_anchors_from_doc(doc);

        if doc.is_region_drag_active() {
            if !self.transport.is_playing() {
                if let Some((start, end)) = self.active_region {
                    self.playhead.set_in_out(start, end);
                }
                let caret = doc
                    .current_position
                    .as_ref()
                    .map(|pos| pos.sample)
                    .unwrap_or(0);
                self.playhead.set_position(caret);
                self.apply_to_engine();
            }
            return;
        }

        let new_region = Self::region_bounds_from_doc(doc);
        let region_changed = new_region != self.active_region;
        self.active_region = new_region;
        let caret = doc
            .current_position
            .as_ref()
            .map(|pos| pos.sample)
            .unwrap_or(0);

        if let Some((start, end)) = new_region {
            self.playhead.set_in_out(start, end);
            if self.transport.is_playing() {
                if region_changed {
                    self.playhead.set_position(start);
                }
            } else {
                self.playhead.set_position(caret);
            }
        } else {
            self.playhead.clear_in_out();
            if self.transport.is_playing() {
                if doc.current_position.is_some() {
                    self.playhead.set_position(caret);
                }
            } else {
                self.playhead.set_position(caret);
            }
        }
        self.apply_to_engine();
        self.sync_monitor(&doc.composition.read().unwrap());
    }

    pub fn sync_document_from_playback(&mut self, doc: &mut BufferDocument) {
        self.sync_playhead_from_engine();
        let sample = self.playhead.position();
        doc.set_position_from_playback(sample, ChannelScope::all());
        self.apply_to_engine();
    }

    pub fn start(&mut self) {
        self.playhead.set_position(self.playhead.playback_start());
        self.transport.set_state(TransportState::Playing);
        self.engine.shared.bump_epoch();
        self.apply_to_engine();
    }

    pub fn play_from(&mut self, sample: usize) {
        self.playhead.set_position(sample);
        self.transport.set_state(TransportState::Playing);
        self.engine.shared.bump_epoch();
        self.apply_to_engine();
    }

    pub fn play(&mut self) {
        if self.transport.state() == TransportState::Playing {
            return;
        }
        self.playhead.set_position(self.engine.shared.position());
        if should_restart_from_start(
            self.transport.state(),
            self.playhead.is_at_end(),
            self.playhead.in_point().is_some(),
        ) {
            self.playhead.set_position(self.playhead.playback_start());
        }
        self.transport.set_state(TransportState::Playing);
        self.engine.shared.bump_epoch();
        self.apply_to_engine();
    }

    pub fn pause(&mut self) {
        self.sync_playhead_from_engine();
        self.transport.set_state(TransportState::Paused);
        self.apply_to_engine();
    }

    pub fn stop(&mut self) {
        self.sync_playhead_from_engine();
        self.transport.set_state(TransportState::Stopped);
        self.apply_to_engine();
    }

    pub fn home(&mut self) {
        self.seek_playhead(self.playhead.playback_start());
    }

    pub fn end(&mut self) {
        self.seek_playhead(self.playhead.transport_end());
    }

    pub fn previous(&mut self) {
        self.sync_playhead_from_engine();
        let pos = self.playhead.position();
        let near = ((self.provider.frames() as f64) * 0.03).round() as usize;
        if let Some(anchor) = previous_anchor_near(&self.anchors, pos, near) {
            self.seek_playhead(anchor);
        }
    }

    pub fn next(&mut self) {
        self.sync_playhead_from_engine();
        let pos = self.playhead.position();
        if let Some(anchor) = next_anchor(&self.anchors, pos) {
            self.seek_playhead(anchor);
        }
    }

    fn seek_playhead(&mut self, sample: usize) {
        self.playhead.set_position(sample);
        if self.transport.is_playing() {
            self.engine.shared.bump_epoch();
        }
        self.apply_to_engine();
    }

    pub fn toggle_loop(&mut self) {
        self.playhead.toggle_looping();
        self.apply_to_engine();
    }

    pub fn toggle_play_pause(&mut self) {
        match self.transport.state() {
            TransportState::Playing => self.pause(),
            TransportState::Paused | TransportState::Stopped => self.play(),
        }
    }

    pub fn reload(&mut self, _buffer: &Buffer) {
        self.stop();
        self.playhead.set_position(0);
        self.playhead.clear_in_out();
        self.active_region = None;
        self.anchors.clear();
        self.engine.shared.bump_epoch();
        self.apply_to_engine();
    }

    pub fn poll(&mut self, doc: &mut BufferDocument) -> bool {
        let engine_state = self.engine.shared.transport();
        if engine_state == TransportState::Stopped
            && self.transport.state() == TransportState::Playing
        {
            self.transport.set_state(TransportState::Stopped);
            self.playhead.set_position(self.engine.shared.position());
            doc.set_position_from_playback(self.playhead.position(), ChannelScope::all());
            return true;
        }
        if self.transport.is_playing() {
            self.sync_document_from_playback(doc);
            return true;
        }
        false
    }
}

fn should_restart_from_start(state: TransportState, at_end: bool, has_region: bool) -> bool {
    match state {
        TransportState::Stopped => has_region || at_end,
        TransportState::Paused => at_end,
        TransportState::Playing => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_from_stopped_at_end_restarts_from_start() {
        assert!(should_restart_from_start(
            TransportState::Stopped,
            true,
            false
        ));
    }

    #[test]
    fn play_from_stopped_mid_buffer_keeps_position() {
        assert!(!should_restart_from_start(
            TransportState::Stopped,
            false,
            false
        ));
    }

    #[test]
    fn play_from_stopped_with_region_restarts_from_in_point() {
        assert!(should_restart_from_start(
            TransportState::Stopped,
            false,
            true
        ));
    }

    #[test]
    fn play_from_paused_at_end_restarts_from_start() {
        assert!(should_restart_from_start(
            TransportState::Paused,
            true,
            false
        ));
    }

    #[test]
    fn play_from_paused_mid_buffer_keeps_position() {
        assert!(!should_restart_from_start(
            TransportState::Paused,
            false,
            false
        ));
    }
}
