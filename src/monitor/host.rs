// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use super::chain::MonitorChain;
use super::dsp::{create_dsp, MonitorDsp};

const FADE_SECS: f32 = 0.02;
const MAX_PLANES: usize = 8;

pub struct MonitorHost {
    sample_rate: u32,
    chain: Option<MonitorChain>,
    playback_channels: Option<Vec<usize>>,
    dsp: Option<Box<dyn MonitorDsp>>,
    prev_dsp: Option<Box<dyn MonitorDsp>>,
    fade_remaining: usize,
    fade_len: usize,
    read_buf: Vec<f32>,
    gathered: Vec<f32>,
    in_planes: Vec<Vec<f32>>,
    out_planes: [Vec<f32>; 2],
    prev_in: Vec<Vec<f32>>,
    prev_out: [Vec<f32>; 2],
    pending_params: Vec<(String, f32)>,
    meters: HashMap<String, f32>,
}

impl MonitorHost {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            chain: None,
            playback_channels: None,
            dsp: None,
            prev_dsp: None,
            fade_remaining: 0,
            fade_len: fade_frames(sample_rate),
            read_buf: Vec::new(),
            gathered: Vec::new(),
            in_planes: vec![Vec::new(); MAX_PLANES],
            out_planes: [Vec::new(), Vec::new()],
            prev_in: vec![Vec::new(); MAX_PLANES],
            prev_out: [Vec::new(), Vec::new()],
            pending_params: Vec::new(),
            meters: HashMap::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn chain(&self) -> Option<MonitorChain> {
        self.chain
    }

    pub fn playback_channels(&self) -> Option<&[usize]> {
        self.playback_channels.as_deref()
    }

    pub fn ui_json(&self) -> Option<&'static str> {
        self.dsp.as_ref().map(|dsp| dsp.ui_json())
    }

    pub fn get_param(&self, address: &str) -> Option<f32> {
        self.dsp.as_ref()?.get_param(address)
    }

    pub fn meter(&self, address: &str) -> Option<f32> {
        self.meters.get(address).copied()
    }

    pub fn meters(&self) -> &HashMap<String, f32> {
        &self.meters
    }

    pub fn set_param(&mut self, address: impl Into<String>, value: f32) {
        self.pending_params.push((address.into(), value));
    }

    pub fn set_config(
        &mut self,
        chain: Option<MonitorChain>,
        playback_channels: Option<Vec<usize>>,
        sample_rate: u32,
    ) {
        let sample_rate = sample_rate.max(1);
        let channels_changed = self.playback_channels != playback_channels;
        let chain_changed = self.chain != chain;
        let rate_changed = self.sample_rate != sample_rate;
        self.playback_channels = playback_channels;
        self.sample_rate = sample_rate;
        self.fade_len = fade_frames(sample_rate);
        if !chain_changed && !rate_changed {
            if channels_changed {
                // Routing only; keep the live DSP.
            }
            return;
        }
        self.chain = chain;
        match chain {
            Some(chain) => {
                let next = create_dsp(chain, sample_rate);
                if let Some(old) = self.dsp.replace(next) {
                    self.prev_dsp = Some(old);
                    self.fade_remaining = self.fade_len.max(1);
                }
            }
            None => {
                self.prev_dsp = self.dsp.take();
                if self.prev_dsp.is_some() {
                    self.fade_remaining = self.fade_len.max(1);
                }
            }
        }
    }

    pub fn read_buf_mut(&mut self, len: usize) -> &mut [f32] {
        if self.read_buf.len() < len {
            self.read_buf.resize(len, 0.0);
        }
        &mut self.read_buf[..len]
    }

    pub fn gathered_mut(&mut self, len: usize) -> &mut [f32] {
        if self.gathered.len() < len {
            self.gathered.resize(len, 0.0);
        }
        &mut self.gathered[..len]
    }

    pub fn copy_read_frame(&mut self, src_base: usize, src_ch: usize, dest_base: usize) {
        let needed = dest_base + src_ch;
        if self.gathered.len() < needed {
            self.gathered.resize(needed, 0.0);
        }
        let mut frame = [0.0f32; 32];
        let n = src_ch.min(32);
        {
            let read_buf = &self.read_buf;
            if src_base + n <= read_buf.len() {
                frame[..n].copy_from_slice(&read_buf[src_base..src_base + n]);
            }
        }
        self.gathered[dest_base..dest_base + n].copy_from_slice(&frame[..n]);
        if src_ch > 32 {
            let extra = src_ch - 32;
            let read_buf = &self.read_buf;
            for i in 0..extra {
                self.gathered[dest_base + 32 + i] =
                    read_buf.get(src_base + 32 + i).copied().unwrap_or(0.0);
            }
        }
    }

    pub fn process_produced(
        &mut self,
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        let gathered = std::mem::take(&mut self.gathered);
        let len = frames.saturating_mul(src_ch);
        self.process_gathered(
            &gathered[..len.min(gathered.len())],
            src_ch,
            frames,
            output,
            out_ch,
        );
        self.gathered = gathered;
    }

    pub fn process_gathered(
        &mut self,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        self.drain_params();
        if src_ch == 0 || frames == 0 || out_ch == 0 {
            return;
        }

        if self.dsp.is_none() {
            if self.fade_remaining > 0 {
                if let Some(prev) = self.prev_dsp.as_deref_mut() {
                    let fade = self.fade_remaining;
                    let fade_len = self.fade_len.max(1);
                    render_dsp(
                        prev,
                        gathered,
                        src_ch,
                        frames,
                        &self.playback_channels,
                        &mut self.prev_in,
                        &mut self.prev_out,
                    );
                    mix_stereo_to_device(&self.prev_out, frames, output, out_ch, |frame| {
                        fade.saturating_sub(frame) as f32 / fade_len as f32
                    });
                    self.fade_remaining = fade.saturating_sub(frames);
                    if self.fade_remaining == 0 {
                        self.prev_dsp = None;
                    }
                    return;
                }
            }
            map_direct(gathered, src_ch, frames, output, out_ch);
            return;
        }

        {
            let dsp = self.dsp.as_deref_mut().expect("dsp");
            render_dsp(
                dsp,
                gathered,
                src_ch,
                frames,
                &self.playback_channels,
                &mut self.in_planes,
                &mut self.out_planes,
            );
            snapshot_meters(dsp, &mut self.meters);
        }

        if self.fade_remaining > 0 {
            if let Some(prev) = self.prev_dsp.as_deref_mut() {
                render_dsp(
                    prev,
                    gathered,
                    src_ch,
                    frames,
                    &self.playback_channels,
                    &mut self.prev_in,
                    &mut self.prev_out,
                );
                let fade = self.fade_remaining;
                let fade_len = self.fade_len.max(1);
                mix_crossfade_to_device(
                    &self.prev_out,
                    &self.out_planes,
                    frames,
                    output,
                    out_ch,
                    fade,
                    fade_len,
                );
                self.fade_remaining = fade.saturating_sub(frames);
                if self.fade_remaining == 0 {
                    self.prev_dsp = None;
                }
                return;
            }
            self.fade_remaining = 0;
        }

        mix_stereo_to_device(&self.out_planes, frames, output, out_ch, |_| 1.0);
    }

    fn drain_params(&mut self) {
        let pending = std::mem::take(&mut self.pending_params);
        if let Some(dsp) = self.dsp.as_mut() {
            for (address, value) in pending {
                dsp.set_param(&address, value);
            }
        }
    }
}

fn fade_frames(sample_rate: u32) -> usize {
    ((sample_rate as f32) * FADE_SECS).round() as usize
}

fn snapshot_meters(dsp: &dyn MonitorDsp, meters: &mut HashMap<String, f32>) {
    meters.clear();
    for address in dsp.meter_addresses() {
        if let Some(value) = dsp.get_param(address) {
            meters.insert(address.clone(), value);
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
    use crate::monitor::dsp::IdentityDsp;

    #[test]
    fn subset_routes_last_two_of_six() {
        let mut host = MonitorHost::new(44100);
        host.dsp = Some(Box::new(IdentityDsp::new(MonitorChain::Stereo)));
        host.chain = Some(MonitorChain::Stereo);
        host.playback_channels = Some(vec![4, 5]);
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
        let mut host = MonitorHost::new(44100);
        host.dsp = Some(Box::new(IdentityDsp::new(MonitorChain::Stereo)));
        host.chain = Some(MonitorChain::Stereo);
        let gathered = [0.1f32, 0.2, 0.9, 0.8];
        let mut out = vec![0.0; 2];
        host.process_gathered(&gathered, 4, 1, &mut out, 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[1] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn missing_dsp_inputs_are_silence() {
        let mut host = MonitorHost::new(44100);
        host.dsp = Some(Box::new(IdentityDsp::new(MonitorChain::Stereo)));
        host.chain = Some(MonitorChain::Stereo);
        let gathered = [0.42f32];
        let mut out = vec![99.0; 2];
        host.process_gathered(&gathered, 1, 1, &mut out, 2);
        assert!((out[0] - 0.42).abs() < 1e-6);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn extra_source_channels_are_unused() {
        let mut host = MonitorHost::new(44100);
        host.dsp = Some(Box::new(IdentityDsp::new(MonitorChain::Stereo)));
        host.chain = Some(MonitorChain::Stereo);
        let gathered = [0.1f32, 0.2, 0.9, 0.8];
        let mut out = vec![0.0; 2];
        host.process_gathered(&gathered, 4, 1, &mut out, 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[1] - 0.2).abs() < 1e-6);
    }
}
