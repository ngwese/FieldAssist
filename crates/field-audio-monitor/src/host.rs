// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Monitor DSP host with lock-free live parameters.
//!
//! [`MonitorHost::set_param`] / [`MonitorHost::get_param`] / [`MonitorHost::meter`]
//! use [`ParamStore`] atomics and do not take the DSP graph mutex.
//!
//! [`MonitorHost::process_gathered`] is intended for the CPAL callback: it
//! `try_lock`s a published [`RtSlot`] (no waiting), applies pre-bound control
//! atomics, runs Faust into pre-sized planes, and publishes meters without
//! allocating. Graph rebuilds happen off the realtime thread via [`ArcSwap`].

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;

use crate::chain::MonitorChain;
use crate::dsp::{create_dsp, MonitorDsp};
use crate::params::ParamStore;

const FADE_SECS: f32 = 0.02;
const MAX_PLANES: usize = 8;
/// Pre-sized plane capacity for callback-sized blocks (`BufferSize::Default`).
pub const MAX_CALLBACK_FRAMES: usize = 8192;

struct HostConfig {
    sample_rate: u32,
    chain: Option<MonitorChain>,
    playback_channels: Option<Vec<usize>>,
    fade_len: usize,
    /// Per-chain values for the composition currently being monitored.
    working_params: HashMap<MonitorChain, HashMap<String, f32>>,
    /// Session-wide defaults. Updated only when leaving an unpinned composition.
    session_params: HashMap<MonitorChain, HashMap<String, f32>>,
}

/// Realtime process state published via [`ArcSwapOption`].
struct RtSlot {
    dsp: Option<Box<dyn MonitorDsp>>,
    prev_dsp: Option<Box<dyn MonitorDsp>>,
    fade_remaining: usize,
    fade_len: usize,
    /// Empty means route `0..num_inputs` from the source.
    playback_channels: Vec<usize>,
    max_frames: usize,
    in_planes: [Vec<f32>; MAX_PLANES],
    out_planes: [Vec<f32>; 2],
    prev_in: [Vec<f32>; MAX_PLANES],
    prev_out: [Vec<f32>; 2],
    control_bindings: Vec<(String, Arc<AtomicU32>)>,
    meter_bindings: Vec<(String, Arc<AtomicU32>)>,
}

impl RtSlot {
    fn preallocated(max_frames: usize, fade_len: usize) -> Self {
        let max_frames = max_frames.max(1);
        Self {
            dsp: None,
            prev_dsp: None,
            fade_remaining: 0,
            fade_len,
            playback_channels: Vec::new(),
            max_frames,
            in_planes: std::array::from_fn(|_| vec![0.0; max_frames]),
            out_planes: [vec![0.0; max_frames], vec![0.0; max_frames]],
            prev_in: std::array::from_fn(|_| vec![0.0; max_frames]),
            prev_out: [vec![0.0; max_frames], vec![0.0; max_frames]],
            control_bindings: Vec::new(),
            meter_bindings: Vec::new(),
        }
    }

    fn ensure_capacity(&mut self, frames: usize) {
        if frames <= self.max_frames {
            return;
        }
        // Off-realtime growth only (config / first oversized block in tests).
        self.max_frames = frames;
        for plane in &mut self.in_planes {
            plane.resize(frames, 0.0);
        }
        for plane in &mut self.out_planes {
            plane.resize(frames, 0.0);
        }
        for plane in &mut self.prev_in {
            plane.resize(frames, 0.0);
        }
        for plane in &mut self.prev_out {
            plane.resize(frames, 0.0);
        }
    }
}

/// Host for monitor DSP chains: routing, crossfades, and param policy.
///
/// Live control get/set is lock-free via [`ParamStore`]. `process_gathered`
/// uses `try_lock` on the published RT slot so the callback never waits on
/// graph rebuilds.
pub struct MonitorHost {
    params: Arc<ParamStore>,
    config: Mutex<HostConfig>,
    rt: ArcSwapOption<Mutex<RtSlot>>,
}

impl MonitorHost {
    /// Create a host at `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1);
        Self {
            params: Arc::new(ParamStore::new()),
            config: Mutex::new(HostConfig {
                sample_rate,
                chain: None,
                playback_channels: None,
                fade_len: fade_frames(sample_rate),
                working_params: HashMap::new(),
                session_params: HashMap::new(),
            }),
            rt: ArcSwapOption::from(None::<Arc<Mutex<RtSlot>>>),
        }
    }

    /// Shared lock-free param/meter store.
    pub fn params(&self) -> &Arc<ParamStore> {
        &self.params
    }

    /// Current sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.config
            .lock()
            .expect("monitor config poisoned")
            .sample_rate
    }

    /// Active monitor chain, if any.
    pub fn chain(&self) -> Option<MonitorChain> {
        self.config.lock().expect("monitor config poisoned").chain
    }

    /// Channel indices routed into the DSP inputs.
    pub fn playback_channels(&self) -> Option<Vec<usize>> {
        self.config
            .lock()
            .expect("monitor config poisoned")
            .playback_channels
            .clone()
    }

    /// Faust UI JSON for the active chain.
    pub fn ui_json(&self) -> Option<&'static str> {
        let slot = self.rt.load_full()?;
        let guard = slot.lock().ok()?;
        guard.dsp.as_ref().map(|dsp| dsp.ui_json())
    }

    /// Read a live control (lock-free).
    pub fn get_param(&self, address: &str) -> Option<f32> {
        if let Some(value) = self.params.get(address) {
            return Some(value);
        }
        let config = self.config.lock().expect("monitor config poisoned");
        if let Some(chain) = config.chain {
            if let Some(value) = config
                .working_params
                .get(&chain)
                .and_then(|params| params.get(address))
            {
                return Some(*value);
            }
        }
        drop(config);
        let slot = self.rt.load_full()?;
        let guard = slot.lock().ok()?;
        guard.dsp.as_ref()?.get_param(address)
    }

    /// Read a meter (lock-free).
    pub fn meter(&self, address: &str) -> Option<f32> {
        self.params.meter(address)
    }

    /// Snapshot all meters (lock-free).
    pub fn meters(&self) -> HashMap<String, f32> {
        self.params.meters_snapshot()
    }

    /// Whether any `Meter_Input*` reading is above `quiet_db` (realtime-safe).
    pub fn input_meters_above(&self, quiet_db: f32) -> bool {
        let Some(slot) = self.rt.load_full() else {
            return false;
        };
        let Ok(guard) = slot.try_lock() else {
            return true;
        };
        guard.meter_bindings.iter().any(|(address, slot)| {
            address.contains("Meter_Input") && ParamStore::load_meter_slot(slot) > quiet_db
        })
    }

    /// Preallocated plane capacity of the published RT slot (for tests).
    pub fn rt_plane_capacity(&self) -> Option<usize> {
        let slot = self.rt.load_full()?;
        let guard = slot.lock().ok()?;
        Some(guard.max_frames)
    }

    /// Set a live control (lock-free; does not take the DSP graph mutex).
    ///
    /// Working/session snapshots pick up the value on the next
    /// [`Self::flush_working`] or chain stash.
    pub fn set_param(&self, address: impl Into<String>, value: f32) {
        self.params.set(address.into().as_str(), value);
    }

    /// Flush working params for the active chain into a snapshot.
    pub fn flush_working(&self) -> HashMap<MonitorChain, HashMap<String, f32>> {
        let mut config = self.config.lock().expect("monitor config poisoned");
        Self::stash_chain_params(&self.params, &mut config, &self.rt);
        config.working_params.clone()
    }

    /// Session-wide parameter defaults.
    pub fn session_params(&self) -> HashMap<MonitorChain, HashMap<String, f32>> {
        self.config
            .lock()
            .expect("monitor config poisoned")
            .session_params
            .clone()
    }

    /// Merge composition snapshots into session defaults.
    pub fn merge_into_session(&self, params: &HashMap<MonitorChain, HashMap<String, f32>>) {
        let mut config = self.config.lock().expect("monitor config poisoned");
        for (chain, values) in params {
            config.session_params.insert(*chain, values.clone());
        }
    }

    /// Replace working params and optionally recreate the DSP graph.
    pub fn replace_working(
        &self,
        chain: Option<MonitorChain>,
        playback_channels: Option<Vec<usize>>,
        sample_rate: u32,
        working: HashMap<MonitorChain, HashMap<String, f32>>,
    ) {
        let mut config = self.config.lock().expect("monitor config poisoned");
        self.params.clear_controls();
        config.working_params = working;
        let sample_rate = sample_rate.max(1);
        let chain_changed = config.chain != chain;
        let rate_changed = config.sample_rate != sample_rate;
        config.playback_channels = playback_channels;
        config.sample_rate = sample_rate;
        config.fade_len = fade_frames(sample_rate);
        if chain_changed || rate_changed {
            Self::recreate_dsp(&self.params, &mut config, &self.rt, chain);
        } else if let Some(chain) = chain {
            if config.working_params.contains_key(&chain) {
                Self::apply_working_params(&self.params, &mut config, &self.rt, chain);
            } else {
                Self::recreate_dsp(&self.params, &mut config, &self.rt, Some(chain));
            }
        } else {
            Self::publish_channels_only(&config, &self.rt);
        }
    }

    /// Update chain, routing, and/or sample rate.
    pub fn set_config(
        &self,
        chain: Option<MonitorChain>,
        playback_channels: Option<Vec<usize>>,
        sample_rate: u32,
    ) {
        let mut config = self.config.lock().expect("monitor config poisoned");
        let sample_rate = sample_rate.max(1);
        let channels_changed = config.playback_channels != playback_channels;
        let chain_changed = config.chain != chain;
        let rate_changed = config.sample_rate != sample_rate;
        config.playback_channels = playback_channels;
        config.sample_rate = sample_rate;
        config.fade_len = fade_frames(sample_rate);
        if !chain_changed && !rate_changed {
            if channels_changed {
                Self::publish_channels_only(&config, &self.rt);
            }
            return;
        }
        Self::stash_chain_params(&self.params, &mut config, &self.rt);
        Self::recreate_dsp(&self.params, &mut config, &self.rt, chain);
    }

    fn publish_channels_only(config: &HostConfig, rt: &ArcSwapOption<Mutex<RtSlot>>) {
        let Some(slot) = rt.load_full() else {
            return;
        };
        let Ok(mut guard) = slot.lock() else {
            return;
        };
        guard.playback_channels = config.playback_channels.clone().unwrap_or_default();
    }

    fn recreate_dsp(
        params: &ParamStore,
        config: &mut HostConfig,
        rt: &ArcSwapOption<Mutex<RtSlot>>,
        chain: Option<MonitorChain>,
    ) {
        config.chain = chain;
        let mut next = RtSlot::preallocated(MAX_CALLBACK_FRAMES, config.fade_len);
        next.playback_channels = config.playback_channels.clone().unwrap_or_default();

        if let Some(old) = rt.load_full() {
            if let Ok(mut old_guard) = old.try_lock() {
                if let Some(prev) = old_guard.dsp.take() {
                    next.prev_dsp = Some(prev);
                    next.fade_remaining = config.fade_len.max(1);
                }
                next.prev_dsp = next.prev_dsp.or(old_guard.prev_dsp.take());
            }
        }

        match chain {
            Some(chain) => {
                let mut dsp = create_dsp(chain, config.sample_rate);
                params.replace_meters(dsp.meter_addresses());
                if let Some(saved) = config.working_params.get(&chain).cloned() {
                    params.replace_controls(&saved);
                    for (address, value) in &saved {
                        dsp.set_param(address, *value);
                    }
                } else {
                    let mut defaults = HashMap::new();
                    for address in dsp.control_addresses() {
                        if let Some(value) = dsp.get_param(&address) {
                            defaults.insert(address, value);
                        }
                    }
                    params.replace_controls(&defaults);
                }
                next.control_bindings = params.bind_controls();
                next.meter_bindings = params.bind_meters();
                next.dsp = Some(dsp);
            }
            None => {
                params.clear_controls();
                params.clear_meters();
                if next.prev_dsp.is_some() {
                    next.fade_remaining = config.fade_len.max(1);
                }
                next.control_bindings.clear();
                next.meter_bindings.clear();
            }
        }

        rt.store(Some(Arc::new(Mutex::new(next))));
    }

    fn stash_chain_params(
        params: &ParamStore,
        config: &mut HostConfig,
        rt: &ArcSwapOption<Mutex<RtSlot>>,
    ) {
        let Some(chain) = config.chain else {
            params.clear_controls();
            return;
        };
        let mut saved = config.working_params.remove(&chain).unwrap_or_default();
        if let Some(slot) = rt.load_full() {
            if let Ok(guard) = slot.lock() {
                if let Some(dsp) = guard.dsp.as_ref() {
                    for address in dsp.control_addresses() {
                        if let Some(value) =
                            params.get(&address).or_else(|| dsp.get_param(&address))
                        {
                            saved.entry(address).or_insert(value);
                        }
                    }
                }
            }
        }
        for (address, value) in params.snapshot_controls() {
            saved.insert(address, value);
        }
        config.working_params.insert(chain, saved);
    }

    fn apply_working_params(
        params: &ParamStore,
        config: &mut HostConfig,
        rt: &ArcSwapOption<Mutex<RtSlot>>,
        chain: MonitorChain,
    ) {
        let Some(saved) = config.working_params.get(&chain).cloned() else {
            if let Some(slot) = rt.load_full() {
                if let Ok(mut guard) = slot.lock() {
                    if let Some(dsp) = guard.dsp.as_ref() {
                        let mut defaults = HashMap::new();
                        for address in dsp.control_addresses() {
                            if let Some(value) = dsp.get_param(&address) {
                                defaults.insert(address, value);
                            }
                        }
                        params.replace_controls(&defaults);
                        guard.control_bindings = params.bind_controls();
                        guard.meter_bindings = params.bind_meters();
                    }
                }
            }
            return;
        };
        params.replace_controls(&saved);
        if let Some(slot) = rt.load_full() {
            if let Ok(mut guard) = slot.lock() {
                if let Some(dsp) = guard.dsp.as_mut() {
                    for (address, value) in &saved {
                        dsp.set_param(address, *value);
                    }
                }
                guard.control_bindings = params.bind_controls();
                guard.meter_bindings = params.bind_meters();
                guard.playback_channels = config.playback_channels.clone().unwrap_or_default();
            }
        }
    }

    /// Process interleaved gathered source frames into device output.
    ///
    /// Realtime-safe: uses `try_lock` (never waits) and does not allocate when
    /// `frames ≤` the published plane capacity.
    pub fn process_gathered(
        &self,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        if src_ch == 0 || frames == 0 || out_ch == 0 {
            return;
        }

        let Some(slot) = self.rt.load_full() else {
            map_direct(gathered, src_ch, frames, output, out_ch);
            return;
        };
        let Ok(mut inner) = slot.try_lock() else {
            map_direct(gathered, src_ch, frames, output, out_ch);
            return;
        };

        if frames > inner.max_frames {
            // Should not happen on the device callback (planes sized to 8192).
            inner.ensure_capacity(frames);
        }

        Self::apply_live_params_rt(&mut inner);
        Self::process_gathered_inner(&mut inner, gathered, src_ch, frames, output, out_ch);
    }

    fn apply_live_params_rt(inner: &mut RtSlot) {
        if let Some(dsp) = inner.dsp.as_mut() {
            for (address, slot) in &inner.control_bindings {
                dsp.set_param(address, ParamStore::load_control_slot(slot));
            }
        }
    }

    fn process_gathered_inner(
        inner: &mut RtSlot,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        if inner.dsp.is_none() {
            if inner.fade_remaining > 0 {
                if let Some(prev) = inner.prev_dsp.as_deref_mut() {
                    let fade = inner.fade_remaining;
                    let fade_len = inner.fade_len.max(1);
                    render_dsp(
                        prev,
                        gathered,
                        src_ch,
                        frames,
                        &inner.playback_channels,
                        &mut inner.prev_in,
                        &mut inner.prev_out,
                    );
                    mix_stereo_to_device(&inner.prev_out, frames, output, out_ch, |frame| {
                        fade.saturating_sub(frame) as f32 / fade_len as f32
                    });
                    inner.fade_remaining = fade.saturating_sub(frames);
                    if inner.fade_remaining == 0 {
                        inner.prev_dsp = None;
                    }
                    return;
                }
            }
            map_direct(gathered, src_ch, frames, output, out_ch);
            return;
        }

        {
            let dsp = inner.dsp.as_deref_mut().expect("dsp");
            render_dsp(
                dsp,
                gathered,
                src_ch,
                frames,
                &inner.playback_channels,
                &mut inner.in_planes,
                &mut inner.out_planes,
            );
            snapshot_meters_rt(dsp, &inner.meter_bindings);
        }

        if inner.fade_remaining > 0 {
            if let Some(prev) = inner.prev_dsp.as_deref_mut() {
                render_dsp(
                    prev,
                    gathered,
                    src_ch,
                    frames,
                    &inner.playback_channels,
                    &mut inner.prev_in,
                    &mut inner.prev_out,
                );
                let fade = inner.fade_remaining;
                let fade_len = inner.fade_len.max(1);
                mix_crossfade_to_device(
                    &inner.prev_out,
                    &inner.out_planes,
                    frames,
                    output,
                    out_ch,
                    fade,
                    fade_len,
                );
                inner.fade_remaining = fade.saturating_sub(frames);
                if inner.fade_remaining == 0 {
                    inner.prev_dsp = None;
                }
                return;
            }
            inner.fade_remaining = 0;
        }

        mix_stereo_to_device(&inner.out_planes, frames, output, out_ch, |_| 1.0);
    }
}

fn fade_frames(sample_rate: u32) -> usize {
    ((sample_rate as f32) * FADE_SECS).round() as usize
}

fn snapshot_meters_rt(dsp: &dyn MonitorDsp, bindings: &[(String, Arc<AtomicU32>)]) {
    for (address, slot) in bindings {
        if let Some(value) = dsp.get_param(address) {
            ParamStore::store_meter_slot(slot, value);
        }
    }
}

fn render_dsp(
    dsp: &mut dyn MonitorDsp,
    gathered: &[f32],
    src_ch: usize,
    frames: usize,
    playback_channels: &[usize],
    in_planes: &mut [Vec<f32>; MAX_PLANES],
    out_planes: &mut [Vec<f32>; 2],
) {
    let n_in = dsp.num_inputs().min(MAX_PLANES);
    for ch in 0..n_in {
        let plane = &mut in_planes[ch][..frames];
        let src = if playback_channels.is_empty() {
            Some(ch)
        } else {
            playback_channels.get(ch).copied()
        };
        if let Some(src) = src {
            if src < src_ch {
                for frame in 0..frames {
                    plane[frame] = gathered[frame * src_ch + src];
                }
                continue;
            }
        }
        plane.fill(0.0);
    }

    let mut input_refs: [&[f32]; MAX_PLANES] = [&[]; MAX_PLANES];
    for ch in 0..n_in {
        input_refs[ch] = &in_planes[ch][..frames];
    }

    // Take owned planes so we can pass two disjoint `&mut [f32]` without
    // overlapping borrows of `out_planes` (capacity already reserved).
    let mut left = std::mem::take(&mut out_planes[0]);
    let mut right = std::mem::take(&mut out_planes[1]);
    if left.len() < frames {
        left.resize(frames, 0.0);
    }
    if right.len() < frames {
        right.resize(frames, 0.0);
    }
    {
        let mut output_refs: [&mut [f32]; 2] = [&mut left[..frames], &mut right[..frames]];
        dsp.compute(frames, &input_refs[..n_in], &mut output_refs);
    }
    out_planes[0] = left;
    out_planes[1] = right;
}

fn mix_stereo_to_device(
    planes: &[Vec<f32>; 2],
    frames: usize,
    output: &mut [f32],
    out_ch: usize,
    gain_at: impl Fn(usize) -> f32,
) {
    for frame in 0..frames {
        let gain = gain_at(frame);
        let l = planes[0].get(frame).copied().unwrap_or(0.0) * gain;
        let r = planes[1].get(frame).copied().unwrap_or(0.0) * gain;
        let base = frame * out_ch;
        if out_ch == 1 {
            output[base] = 0.5 * (l + r);
        } else {
            output[base] = l;
            output[base + 1] = r;
            for ch in 2..out_ch {
                output[base + ch] = 0.0;
            }
        }
    }
}

fn mix_crossfade_to_device(
    old: &[Vec<f32>; 2],
    new: &[Vec<f32>; 2],
    frames: usize,
    output: &mut [f32],
    out_ch: usize,
    fade_remaining: usize,
    fade_len: usize,
) {
    let fade_len = fade_len.max(1) as f32;
    for frame in 0..frames {
        let old_gain = fade_remaining.saturating_sub(frame) as f32 / fade_len;
        let new_gain = 1.0 - old_gain;
        let l = old[0].get(frame).copied().unwrap_or(0.0) * old_gain
            + new[0].get(frame).copied().unwrap_or(0.0) * new_gain;
        let r = old[1].get(frame).copied().unwrap_or(0.0) * old_gain
            + new[1].get(frame).copied().unwrap_or(0.0) * new_gain;
        let base = frame * out_ch;
        if out_ch == 1 {
            output[base] = 0.5 * (l + r);
        } else {
            output[base] = l;
            output[base + 1] = r;
            for ch in 2..out_ch {
                output[base + ch] = 0.0;
            }
        }
    }
}

/// Direct channel mapping when no monitor DSP is active.
pub fn map_direct(
    gathered: &[f32],
    src_ch: usize,
    frames: usize,
    output: &mut [f32],
    out_ch: usize,
) {
    for frame in 0..frames {
        let base = frame * src_ch;
        for ch in 0..out_ch {
            let sample = if src_ch == 1 {
                gathered[base]
            } else if ch < src_ch {
                gathered[base + ch]
            } else {
                0.0
            };
            output[frame * out_ch + ch] = sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::IdentityDsp;

    fn with_identity(host: &MonitorHost, chain: MonitorChain, channels: Option<Vec<usize>>) {
        let mut config = host.config.lock().unwrap();
        config.chain = Some(chain);
        config.playback_channels = channels.clone();
        let mut slot = RtSlot::preallocated(MAX_CALLBACK_FRAMES, config.fade_len);
        slot.dsp = Some(Box::new(IdentityDsp::new(chain)));
        slot.playback_channels = channels.unwrap_or_default();
        host.rt.store(Some(Arc::new(Mutex::new(slot))));
    }

    #[test]
    fn subset_routes_last_two_of_six() {
        let host = MonitorHost::new(44100);
        with_identity(&host, MonitorChain::Stereo, Some(vec![4, 5]));
        let mut gathered = vec![0.0f32; 6];
        gathered[4] = 0.3;
        gathered[5] = -0.7;
        let mut out = vec![0.0; 2];
        host.process_gathered(&gathered, 6, 1, &mut out, 2);
        assert!((out[0] - 0.3).abs() < 1e-6);
        assert!((out[1] + 0.7).abs() < 1e-6);
    }

    #[test]
    fn default_all_channels_feed_in_order() {
        let host = MonitorHost::new(44100);
        with_identity(&host, MonitorChain::Stereo, None);
        let gathered = [0.1f32, 0.2, 0.9, 0.8];
        let mut out = vec![0.0; 2];
        host.process_gathered(&gathered, 4, 1, &mut out, 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[1] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn missing_dsp_inputs_are_silence() {
        let host = MonitorHost::new(44100);
        with_identity(&host, MonitorChain::Stereo, None);
        let gathered = [0.42f32];
        let mut out = vec![99.0; 2];
        host.process_gathered(&gathered, 1, 1, &mut out, 2);
        assert!((out[0] - 0.42).abs() < 1e-6);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn extra_source_channels_are_unused() {
        let host = MonitorHost::new(44100);
        with_identity(&host, MonitorChain::Stereo, None);
        let gathered = [0.1f32, 0.2, 0.9, 0.8];
        let mut out = vec![0.0; 2];
        host.process_gathered(&gathered, 4, 1, &mut out, 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[1] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn chain_params_survive_switching_chains() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        host.set_param("/MonitorStereo/Output_Gain", -6.0);
        host.set_config(Some(MonitorChain::Foa), None, 44100);
        assert!(host.get_param("/MonitorStereo/Output_Gain").is_none());
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        let gain = host
            .get_param("/MonitorStereo/Output_Gain")
            .expect("stereo gain restored");
        assert!((gain + 6.0).abs() < 1e-5, "{gain}");
    }

    #[test]
    fn chain_params_survive_sample_rate_change() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        host.set_param("/MonitorStereo/Output_Gain", -6.0);
        host.set_config(Some(MonitorChain::Stereo), None, 48000);
        let gain = host
            .get_param("/MonitorStereo/Output_Gain")
            .expect("gain restored");
        assert!((gain + 6.0).abs() < 1e-5, "{gain}");
    }

    #[test]
    fn live_edits_do_not_change_session_until_commit() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        host.set_param("/MonitorStereo/Output_Gain", 0.0);
        let initial = host.flush_working();
        host.merge_into_session(&initial);

        host.set_param("/MonitorStereo/Output_Gain", -6.0);
        let session_gain = host
            .session_params()
            .get(&MonitorChain::Stereo)
            .and_then(|params| params.get("/MonitorStereo/Output_Gain"))
            .copied()
            .unwrap_or(0.0);
        assert!(session_gain.abs() < 1e-5, "{session_gain}");

        let live = host
            .get_param("/MonitorStereo/Output_Gain")
            .expect("live gain");
        assert!((live + 6.0).abs() < 1e-5, "{live}");
    }

    #[test]
    fn leaving_unpinned_promotes_working_to_session() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        host.set_param("/MonitorStereo/Output_Gain", -3.0);
        let snapshot = host.flush_working();
        host.merge_into_session(&snapshot);
        host.replace_working(Some(MonitorChain::Stereo), None, 44100, HashMap::new());
        host.replace_working(
            Some(MonitorChain::Stereo),
            None,
            44100,
            host.session_params(),
        );
        let restored = host
            .get_param("/MonitorStereo/Output_Gain")
            .expect("session gain");
        assert!((restored + 3.0).abs() < 1e-5, "{restored}");
    }

    #[test]
    fn leaving_pinned_does_not_promote_to_session() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        host.set_param("/MonitorStereo/Output_Gain", 0.0);
        let session = host.flush_working();
        host.merge_into_session(&session);

        host.set_param("/MonitorStereo/Output_Gain", -12.0);
        let pinned = host.flush_working();

        host.replace_working(
            Some(MonitorChain::Stereo),
            None,
            44100,
            host.session_params(),
        );
        let session_live = host
            .get_param("/MonitorStereo/Output_Gain")
            .expect("session after leaving pinned");
        assert!(session_live.abs() < 1e-5, "{session_live}");

        host.replace_working(Some(MonitorChain::Stereo), None, 44100, pinned);
        let pinned_live = host
            .get_param("/MonitorStereo/Output_Gain")
            .expect("pinned restore");
        assert!((pinned_live + 12.0).abs() < 1e-5, "{pinned_live}");
    }

    #[test]
    fn set_param_is_visible_without_process_lock_contention() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        host.set_param("/MonitorStereo/Output_Gain", -9.0);
        assert!((host.get_param("/MonitorStereo/Output_Gain").unwrap() + 9.0).abs() < 1e-5);
        assert!((host.params().get("/MonitorStereo/Output_Gain").unwrap() + 9.0).abs() < 1e-5);
    }

    #[test]
    fn process_gathered_keeps_preallocated_plane_capacity() {
        let host = MonitorHost::new(44100);
        host.set_config(Some(MonitorChain::Stereo), None, 44100);
        let before = host.rt_plane_capacity().expect("rt slot");
        assert_eq!(before, MAX_CALLBACK_FRAMES);
        let gathered = vec![0.1f32; 256 * 2];
        let mut out = vec![0.0; 256 * 2];
        for _ in 0..8 {
            host.process_gathered(&gathered, 2, 256, &mut out, 2);
        }
        assert_eq!(host.rt_plane_capacity(), Some(before));
    }
}
