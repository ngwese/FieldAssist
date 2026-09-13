// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Monitor DSP host with lock-free live parameters.
//!
//! [`MonitorHost::set_param`] / [`MonitorHost::get_param`] / [`MonitorHost::meter`]
//! use [`ParamStore`] atomics and do not take the DSP graph mutex. Audio
//! processing briefly locks that mutex to snapshot params, run Faust, and
//! update meters.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::chain::MonitorChain;
use crate::dsp::{create_dsp, MonitorDsp};
use crate::params::ParamStore;

const FADE_SECS: f32 = 0.02;
const MAX_PLANES: usize = 8;

struct HostInner {
    sample_rate: u32,
    chain: Option<MonitorChain>,
    playback_channels: Option<Vec<usize>>,
    dsp: Option<Box<dyn MonitorDsp>>,
    prev_dsp: Option<Box<dyn MonitorDsp>>,
    fade_remaining: usize,
    fade_len: usize,
    in_planes: Vec<Vec<f32>>,
    out_planes: [Vec<f32>; 2],
    prev_in: Vec<Vec<f32>>,
    prev_out: [Vec<f32>; 2],
    /// Per-chain values for the composition currently being monitored.
    working_params: HashMap<MonitorChain, HashMap<String, f32>>,
    /// Session-wide defaults. Updated only when leaving an unpinned composition.
    session_params: HashMap<MonitorChain, HashMap<String, f32>>,
}

/// Host for monitor DSP chains: routing, crossfades, and param policy.
///
/// Live control get/set is lock-free via [`ParamStore`]. Graph swaps and
/// `process_*` take a short mutex.
pub struct MonitorHost {
    params: Arc<ParamStore>,
    inner: Mutex<HostInner>,
}

impl MonitorHost {
    /// Create a host at `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            params: Arc::new(ParamStore::new()),
            inner: Mutex::new(HostInner {
                sample_rate: sample_rate.max(1),
                chain: None,
                playback_channels: None,
                dsp: None,
                prev_dsp: None,
                fade_remaining: 0,
                fade_len: fade_frames(sample_rate),
                in_planes: vec![Vec::new(); MAX_PLANES],
                out_planes: [Vec::new(), Vec::new()],
                prev_in: vec![Vec::new(); MAX_PLANES],
                prev_out: [Vec::new(), Vec::new()],
                working_params: HashMap::new(),
                session_params: HashMap::new(),
            }),
        }
    }

    /// Shared lock-free param/meter store.
    pub fn params(&self) -> &Arc<ParamStore> {
        &self.params
    }

    /// Current sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.inner
            .lock()
            .expect("monitor host poisoned")
            .sample_rate
    }

    /// Active monitor chain, if any.
    pub fn chain(&self) -> Option<MonitorChain> {
        self.inner.lock().expect("monitor host poisoned").chain
    }

    /// Channel indices routed into the DSP inputs.
    pub fn playback_channels(&self) -> Option<Vec<usize>> {
        self.inner
            .lock()
            .expect("monitor host poisoned")
            .playback_channels
            .clone()
    }

    /// Faust UI JSON for the active chain.
    pub fn ui_json(&self) -> Option<&'static str> {
        self.inner
            .lock()
            .expect("monitor host poisoned")
            .dsp
            .as_ref()
            .map(|dsp| dsp.ui_json())
    }

    /// Read a live control (lock-free).
    pub fn get_param(&self, address: &str) -> Option<f32> {
        if let Some(value) = self.params.get(address) {
            return Some(value);
        }
        let inner = self.inner.lock().expect("monitor host poisoned");
        if let Some(chain) = inner.chain {
            if let Some(value) = inner
                .working_params
                .get(&chain)
                .and_then(|params| params.get(address))
            {
                return Some(*value);
            }
        }
        inner.dsp.as_ref()?.get_param(address)
    }

    /// Read a meter (lock-free).
    pub fn meter(&self, address: &str) -> Option<f32> {
        self.params.meter(address)
    }

    /// Snapshot all meters (lock-free).
    pub fn meters(&self) -> HashMap<String, f32> {
        self.params.meters_snapshot()
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
        let mut inner = self.inner.lock().expect("monitor host poisoned");
        Self::stash_chain_params(&self.params, &mut inner);
        inner.working_params.clone()
    }

    /// Session-wide parameter defaults.
    pub fn session_params(&self) -> HashMap<MonitorChain, HashMap<String, f32>> {
        self.inner
            .lock()
            .expect("monitor host poisoned")
            .session_params
            .clone()
    }

    /// Merge composition snapshots into session defaults.
    pub fn merge_into_session(&self, params: &HashMap<MonitorChain, HashMap<String, f32>>) {
        let mut inner = self.inner.lock().expect("monitor host poisoned");
        for (chain, values) in params {
            inner.session_params.insert(*chain, values.clone());
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
        let mut inner = self.inner.lock().expect("monitor host poisoned");
        self.params.clear_controls();
        inner.working_params = working;
        let sample_rate = sample_rate.max(1);
        let chain_changed = inner.chain != chain;
        let rate_changed = inner.sample_rate != sample_rate;
        inner.playback_channels = playback_channels;
        inner.sample_rate = sample_rate;
        inner.fade_len = fade_frames(sample_rate);
        if chain_changed || rate_changed {
            Self::recreate_dsp(&self.params, &mut inner, chain);
        } else if let Some(chain) = chain {
            if inner.working_params.contains_key(&chain) {
                Self::apply_working_params(&self.params, &mut inner, chain);
            } else {
                Self::recreate_dsp(&self.params, &mut inner, Some(chain));
            }
        }
    }

    /// Update chain, routing, and/or sample rate.
    pub fn set_config(
        &self,
        chain: Option<MonitorChain>,
        playback_channels: Option<Vec<usize>>,
        sample_rate: u32,
    ) {
        let mut inner = self.inner.lock().expect("monitor host poisoned");
        let sample_rate = sample_rate.max(1);
        let channels_changed = inner.playback_channels != playback_channels;
        let chain_changed = inner.chain != chain;
        let rate_changed = inner.sample_rate != sample_rate;
        inner.playback_channels = playback_channels;
        inner.sample_rate = sample_rate;
        inner.fade_len = fade_frames(sample_rate);
        if !chain_changed && !rate_changed {
            let _ = channels_changed;
            return;
        }
        Self::stash_chain_params(&self.params, &mut inner);
        Self::recreate_dsp(&self.params, &mut inner, chain);
    }

    fn recreate_dsp(params: &ParamStore, inner: &mut HostInner, chain: Option<MonitorChain>) {
        inner.chain = chain;
        match chain {
            Some(chain) => {
                let next = create_dsp(chain, inner.sample_rate);
                params.replace_meters(next.meter_addresses());
                if let Some(old) = inner.dsp.replace(next) {
                    inner.prev_dsp = Some(old);
                    inner.fade_remaining = inner.fade_len.max(1);
                }
                Self::apply_working_params(params, inner, chain);
            }
            None => {
                params.clear_controls();
                params.clear_meters();
                inner.prev_dsp = inner.dsp.take();
                if inner.prev_dsp.is_some() {
                    inner.fade_remaining = inner.fade_len.max(1);
                }
            }
        }
    }

    fn stash_chain_params(params: &ParamStore, inner: &mut HostInner) {
        let Some(chain) = inner.chain else {
            params.clear_controls();
            return;
        };
        let mut saved = inner.working_params.remove(&chain).unwrap_or_default();
        if let Some(dsp) = inner.dsp.as_ref() {
            for address in dsp.control_addresses() {
                if let Some(value) = params.get(&address).or_else(|| dsp.get_param(&address)) {
                    saved.entry(address).or_insert(value);
                }
            }
        }
        for (address, value) in params.snapshot_controls() {
            saved.insert(address, value);
        }
        inner.working_params.insert(chain, saved);
    }

    fn apply_working_params(params: &ParamStore, inner: &mut HostInner, chain: MonitorChain) {
        let Some(saved) = inner.working_params.get(&chain).cloned() else {
            if let Some(dsp) = inner.dsp.as_ref() {
                let mut defaults = HashMap::new();
                for address in dsp.control_addresses() {
                    if let Some(value) = dsp.get_param(&address) {
                        defaults.insert(address, value);
                    }
                }
                params.replace_controls(&defaults);
            }
            return;
        };
        params.replace_controls(&saved);
        if let Some(dsp) = inner.dsp.as_mut() {
            for (address, value) in &saved {
                dsp.set_param(address, *value);
            }
        }
    }

    /// Process interleaved gathered source frames into device output.
    pub fn process_gathered(
        &self,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        let mut inner = self.inner.lock().expect("monitor host poisoned");
        Self::process_gathered_inner(
            &self.params,
            &mut inner,
            gathered,
            src_ch,
            frames,
            output,
            out_ch,
        );
    }

    fn process_gathered_inner(
        params: &ParamStore,
        inner: &mut HostInner,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        Self::apply_live_params(params, inner);
        if src_ch == 0 || frames == 0 || out_ch == 0 {
            return;
        }

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
            snapshot_meters(dsp, params);
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

    fn apply_live_params(params: &ParamStore, inner: &mut HostInner) {
        let snapshot = params.snapshot_controls();
        if let Some(dsp) = inner.dsp.as_mut() {
            for (address, value) in snapshot {
                dsp.set_param(&address, value);
            }
        }
    }
}

fn fade_frames(sample_rate: u32) -> usize {
    ((sample_rate as f32) * FADE_SECS).round() as usize
}

fn snapshot_meters(dsp: &dyn MonitorDsp, params: &ParamStore) {
    for address in dsp.meter_addresses() {
        if let Some(value) = dsp.get_param(address) {
            params.set_meter(address, value);
        }
    }
}

fn render_dsp(
    dsp: &mut dyn MonitorDsp,
    gathered: &[f32],
    src_ch: usize,
    frames: usize,
    playback_channels: &Option<Vec<usize>>,
    in_planes: &mut [Vec<f32>],
    out_planes: &mut [Vec<f32>; 2],
) {
    let n_in = dsp.num_inputs().min(MAX_PLANES);
    let sources = match playback_channels.as_deref() {
        Some(channels) if !channels.is_empty() => channels.to_vec(),
        _ => (0..src_ch).collect(),
    };
    for ch in 0..n_in {
        if in_planes[ch].len() < frames {
            in_planes[ch].resize(frames, 0.0);
        }
        let plane = &mut in_planes[ch][..frames];
        if let Some(&src) = sources.get(ch) {
            if src < src_ch {
                for frame in 0..frames {
                    plane[frame] = gathered[frame * src_ch + src];
                }
                continue;
            }
        }
        plane.fill(0.0);
    }
    for out in out_planes.iter_mut() {
        if out.len() < frames {
            out.resize(frames, 0.0);
        }
    }
    let input_refs: Vec<&[f32]> = (0..n_in)
        .map(|ch| in_planes[ch][..frames].as_ref())
        .collect();
    let mut left = std::mem::take(&mut out_planes[0]);
    let mut right = std::mem::take(&mut out_planes[1]);
    {
        let mut output_refs: [&mut [f32]; 2] = [&mut left[..frames], &mut right[..frames]];
        dsp.compute(frames, &input_refs, &mut output_refs);
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
        let mut inner = host.inner.lock().unwrap();
        inner.dsp = Some(Box::new(IdentityDsp::new(chain)));
        inner.chain = Some(chain);
        inner.playback_channels = channels;
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
}
