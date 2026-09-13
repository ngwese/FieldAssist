// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::sync::{Arc, RwLock};

use anyhow::Result;
use cpal::Device;

use crate::model::buffer::ChannelScope;
use crate::model::composition::Composition;
use crate::model::document::BufferDocument;
use crate::model::Buffer;
use crate::monitor::{MonitorChain, MonitorHost, MonitorHostProcess};

use super::anchors::{collect_anchors, next_anchor, previous_anchor_near};
use super::provider::SharedCompositionProvider;
use super::{PlaybackDataProvider, PlaybackEngine, Playhead, Transport, TransportState};

/// Matches `INPUT_METER_QUIET_DB` in the playback engine: keep refreshing the
/// monitor UI while envelopes are still decaying after stop/pause.
const MONITOR_UI_METER_QUIET_DB: f32 = -89.0;

pub struct PlaybackSession {
    provider: Arc<SharedCompositionProvider>,
    playhead: Playhead,
    transport: Transport,
    engine: PlaybackEngine,
    monitor: Arc<MonitorHost>,
    anchors: Vec<usize>,
    active_region: Option<(usize, usize)>,
}

impl PlaybackSession {
    pub fn open(device: &Device, composition: Arc<RwLock<Composition>>) -> Result<Self> {
        let provider = Arc::new(SharedCompositionProvider::new(composition));
        let playhead = Playhead::new(provider.clone());
        let engine = PlaybackEngine::open(device, provider.clone())?;
        let monitor = Arc::new(MonitorHost::new(engine.shared.output_rate()));
        let process = Arc::new(MonitorHostProcess::new(monitor.clone()));
        engine
            .shared
            .set_monitor_process(Some(process as Arc<dyn super::MonitorProcess>));
        Ok(Self {
            provider,
            playhead,
            transport: Transport::new(),
            engine,
            monitor,
            anchors: Vec::new(),
            active_region: None,
        })
    }

    pub fn set_output_device(&mut self, device: &Device) -> Result<()> {
        self.stop();
        self.engine.reopen(device)?;
        self.apply_to_engine();
        Ok(())
    }

    pub fn bind_composition(&self, composition: Arc<RwLock<Composition>>) {
        self.provider.bind(composition.clone());
        self.sync_monitor(&composition.read().unwrap());
    }

    pub fn sync_monitor(&self, composition: &Composition) {
        let chain = composition.monitor_chain().and_then(MonitorChain::parse);
        let channels = composition.playback_channels().map(|ch| ch.to_vec());
        let rate = self.engine.shared.output_rate();
        self.monitor.set_config(chain, channels, rate);
    }

    pub fn commit_monitor_for_document(&self, doc: &mut BufferDocument) {
        let snapshot = self.monitor.flush_working();
        if doc.monitor_params_pinned {
            doc.pinned_monitor_params = snapshot;
        } else {
            self.monitor.merge_into_session(&snapshot);
        }
    }

    pub fn commit_monitor_to_session(&self) {
        let snapshot = self.monitor.flush_working();
        self.monitor.merge_into_session(&snapshot);
    }

    pub fn load_monitor_for_document(&self, doc: &BufferDocument) {
        let mut working = self.monitor.session_params();
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
        let rate = self.engine.shared.output_rate();
        self.monitor.replace_working(chain, channels, rate, working);
    }

    pub fn load_session_monitor(&self, composition: &Composition) {
        let chain = composition.monitor_chain().and_then(MonitorChain::parse);
        let channels = composition.playback_channels().map(|ch| ch.to_vec());
        let working = self.monitor.session_params();
        let rate = self.engine.shared.output_rate();
        self.monitor.replace_working(chain, channels, rate, working);
    }

    pub fn set_monitor_param(&self, address: &str, value: f32) {
        self.monitor.set_param(address, value);
    }

    pub fn monitor_param(&self, address: &str) -> Option<f32> {
        self.monitor.get_param(address)
    }

    pub fn monitor_ui_json(&self) -> Option<&'static str> {
        self.monitor.ui_json()
    }

    pub fn monitor_meters(&self) -> std::collections::HashMap<String, f32> {
        self.monitor.meters()
    }

    /// True while any input meter is still above the quiet floor (e.g. decay).
    pub fn input_meters_active(&self) -> bool {
        self.monitor.input_meters_above(MONITOR_UI_METER_QUIET_DB)
    }

    pub fn transport_state(&self) -> TransportState {
        self.transport.state()
    }

    pub fn looping(&self) -> bool {
        self.playhead.looping()
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

    pub fn sync_from_document(&mut self, doc: &mut BufferDocument) {
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
        let engine_pos = self.engine.shared.position();
        let mut seek_while_playing = false;

        if let Some((start, end)) = new_region {
            self.playhead.set_in_out(start, end);
            if self.transport.is_playing() {
                // On region create/change during play, always seek to the in-point
                // (finish_region_drag leaves the caret at the out-point).
                if let Some(target) =
                    seek_target_after_region_change(true, region_changed, Some((start, end)))
                {
                    self.playhead.set_position(target);
                    seek_while_playing = true;
                    doc.set_position_from_playback(target, ChannelScope::all());
                }
            } else {
                self.playhead.set_position(caret);
            }
        } else {
            self.playhead.clear_in_out();
            if self.transport.is_playing() {
                if doc.current_position.is_some() && caret != engine_pos {
                    self.playhead.set_position(caret);
                    seek_while_playing = true;
                }
            } else {
                self.playhead.set_position(caret);
            }
        }
        self.apply_to_engine();
        if seek_while_playing {
            // Position is already on the engine; bump so prefetch retargets.
            self.engine.shared.bump_epoch();
        }
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
        self.apply_to_engine();
        self.engine.shared.bump_epoch();
    }

    pub fn play_from(&mut self, sample: usize) {
        self.playhead.set_position(sample);
        self.transport.set_state(TransportState::Playing);
        self.apply_to_engine();
        self.engine.shared.bump_epoch();
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
        self.apply_to_engine();
        self.engine.shared.bump_epoch();
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
            self.engine.shared.seek_to(sample);
            self.apply_to_engine();
        } else {
            self.apply_to_engine();
        }
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
        self.apply_to_engine();
        self.engine.shared.bump_epoch();
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

/// While playing, a newly applied selection region seeks to its start (in-point).
fn seek_target_after_region_change(
    playing: bool,
    region_changed: bool,
    new_region: Option<(usize, usize)>,
) -> Option<usize> {
    if playing && region_changed {
        new_region.map(|(start, _end)| start)
    } else {
        None
    }
}

/// Gate for the ~30 Hz monitor UI timer ([issue #14](https://github.com/ngwese/FieldAssist/issues/14)).
///
/// Skip repaints when the Monitor tab is hidden, or when transport is idle and
/// meters have already settled — VU bars are static then and do not need paint.
pub(crate) fn should_refresh_monitor_ui(
    tab_visible: bool,
    playing: bool,
    meters_active: bool,
) -> bool {
    tab_visible && (playing || meters_active)
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

    #[test]
    fn region_change_while_playing_seeks_to_region_start() {
        assert_eq!(
            seek_target_after_region_change(true, true, Some((100, 500))),
            Some(100),
            "finishing a region drag during play must seek to the in-point"
        );
        // Even when the engine is already at the in-point (drag started there),
        // still seek so the caret leaves the out-point and prefetch resyncs.
        assert_eq!(
            seek_target_after_region_change(true, true, Some((200, 200))),
            Some(200)
        );
    }

    #[test]
    fn region_unchanged_while_playing_does_not_seek() {
        assert_eq!(
            seek_target_after_region_change(true, false, Some((100, 500))),
            None
        );
    }

    #[test]
    fn region_change_while_stopped_does_not_seek() {
        assert_eq!(
            seek_target_after_region_change(false, true, Some((100, 500))),
            None,
            "when stopped, finish_region_drag keeps the caret at the out-point"
        );
    }

    #[test]
    fn clearing_region_while_playing_does_not_seek_via_region_helper() {
        assert_eq!(seek_target_after_region_change(true, true, None), None);
    }

    #[test]
    fn monitor_ui_refreshes_only_when_visible_and_live() {
        assert!(
            should_refresh_monitor_ui(true, true, false),
            "playing with monitor tab visible must refresh meters"
        );
        assert!(
            should_refresh_monitor_ui(true, false, true),
            "post-stop meter decay must keep refreshing while visible"
        );
        assert!(
            !should_refresh_monitor_ui(true, false, false),
            "idle quiet meters must not spin the monitor UI"
        );
        assert!(
            !should_refresh_monitor_ui(false, true, true),
            "hidden monitor tab must not refresh even while playing"
        );
    }
}
