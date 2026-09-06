// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use super::chain::MonitorChain;
use super::generated::{
    self, FaustDsp, ParamIndex, MONITOR_FOA_FUMA_JSON, MONITOR_FOA_JSON, MONITOR_MONO_JSON,
    MONITOR_MS_JSON, MONITOR_STEREO_JSON, UI,
};
use super::schema::{collect_addresses, parse_ui_json};

pub trait MonitorDsp: Send {
    fn chain(&self) -> MonitorChain;
    fn num_inputs(&self) -> usize;
    fn num_outputs(&self) -> usize;
    fn compute(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]]);
    fn set_param(&mut self, address: &str, value: f32);
    fn get_param(&self, address: &str) -> Option<f32>;
    fn ui_json(&self) -> &'static str;
    fn meter_addresses(&self) -> &[String];
}

pub fn create_dsp(chain: MonitorChain, sample_rate: u32) -> Box<dyn MonitorDsp> {
    match chain {
        MonitorChain::Mono => Box::new(FaustAdapter::new(
            generated::MonitorMono::new(),
            chain,
            MONITOR_MONO_JSON,
            sample_rate,
        )),
        MonitorChain::Stereo => Box::new(FaustAdapter::new(
            generated::MonitorStereo::new(),
            chain,
            MONITOR_STEREO_JSON,
            sample_rate,
        )),
        MonitorChain::Ms => Box::new(FaustAdapter::new(
            generated::MonitorMs::new(),
            chain,
            MONITOR_MS_JSON,
            sample_rate,
        )),
        MonitorChain::Foa => Box::new(FaustAdapter::new(
            generated::MonitorFoa::new(),
            chain,
            MONITOR_FOA_JSON,
            sample_rate,
        )),
        MonitorChain::FoaFuma => Box::new(FaustAdapter::new(
            generated::MonitorFoaFuma::new(),
            chain,
            MONITOR_FOA_FUMA_JSON,
            sample_rate,
        )),
    }
}

struct FaustAdapter<D> {
    dsp: D,
    chain: MonitorChain,
    json: &'static str,
    index_by_address: HashMap<String, i32>,
    meters: Vec<String>,
}

impl<D: FaustDsp<T = f32> + Send> FaustAdapter<D> {
    fn new(mut dsp: D, chain: MonitorChain, json: &'static str, sample_rate: u32) -> Self {
        dsp.init(sample_rate as i32);
        let n_in = dsp.get_num_inputs().max(0) as usize;
        let n_out = dsp.get_num_outputs().max(0) as usize;
        let prime = 8192usize;
        let zeros = vec![0.0f32; prime];
        let inputs: Vec<&[f32]> = (0..n_in).map(|_| zeros.as_slice()).collect();
        let mut out_bufs: Vec<Vec<f32>> = (0..n_out).map(|_| vec![0.0; prime]).collect();
        let mut output_refs: Vec<&mut [f32]> =
            out_bufs.iter_mut().map(|buf| buf.as_mut_slice()).collect();
        dsp.compute(prime as i32, &inputs, &mut output_refs);
        let addresses = parse_ui_json(json)
            .map(|root| collect_addresses(&root))
            .unwrap_or_default();
        let mut index_by_address = HashMap::new();
        let mut meters = Vec::new();
        dsp.build_user_interface(&mut AddressBinder {
            addresses: &addresses,
            next: 0,
            index_by_address: &mut index_by_address,
            meters: &mut meters,
        });
        Self {
            dsp,
            chain,
            json,
            index_by_address,
            meters,
        }
    }
}

struct AddressBinder<'a> {
    addresses: &'a [(String, bool)],
    next: usize,
    index_by_address: &'a mut HashMap<String, i32>,
    meters: &'a mut Vec<String>,
}

impl AddressBinder<'_> {
    fn bind(&mut self, param: ParamIndex) {
        if let Some((address, passive)) = self.addresses.get(self.next) {
            self.index_by_address.insert(address.clone(), param.0);
            if *passive {
                self.meters.push(address.clone());
            }
            self.next += 1;
        }
    }
}

impl UI<f32> for AddressBinder<'_> {
    fn open_tab_box(&mut self, _: &str) {}
    fn open_horizontal_box(&mut self, _: &str) {}
    fn open_vertical_box(&mut self, _: &str) {}
    fn close_box(&mut self) {}
    fn add_button(&mut self, _: &str, param: ParamIndex) {
        self.bind(param);
    }
    fn add_check_button(&mut self, _: &str, param: ParamIndex) {
        self.bind(param);
    }
    fn add_vertical_slider(&mut self, _: &str, param: ParamIndex, _: f32, _: f32, _: f32, _: f32) {
        self.bind(param);
    }
    fn add_horizontal_slider(
        &mut self,
        _: &str,
        param: ParamIndex,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
    ) {
        self.bind(param);
    }
    fn add_num_entry(&mut self, _: &str, param: ParamIndex, _: f32, _: f32, _: f32, _: f32) {
        self.bind(param);
    }
    fn add_horizontal_bargraph(&mut self, _: &str, param: ParamIndex, _: f32, _: f32) {
        self.bind(param);
    }
    fn add_vertical_bargraph(&mut self, _: &str, param: ParamIndex, _: f32, _: f32) {
        self.bind(param);
    }
    fn declare(&mut self, _: Option<ParamIndex>, _: &str, _: &str) {}
}

impl<D: FaustDsp<T = f32> + Send> MonitorDsp for FaustAdapter<D> {
    fn chain(&self) -> MonitorChain {
        self.chain
    }

    fn num_inputs(&self) -> usize {
        self.dsp.get_num_inputs().max(0) as usize
    }

    fn num_outputs(&self) -> usize {
        self.dsp.get_num_outputs().max(0) as usize
    }

    fn compute(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        self.dsp.compute(frames as i32, inputs, outputs);
    }

    fn set_param(&mut self, address: &str, value: f32) {
        if let Some(index) = self.index_by_address.get(address).copied() {
            self.dsp.set_param(ParamIndex(index), value);
        }
    }

    fn get_param(&self, address: &str) -> Option<f32> {
        let index = self.index_by_address.get(address).copied()?;
        self.dsp.get_param(ParamIndex(index))
    }

    fn ui_json(&self) -> &'static str {
        self.json
    }

    fn meter_addresses(&self) -> &[String] {
        &self.meters
    }
}

/// Host-side stereo passthrough used by routing tests.
pub struct IdentityDsp {
    chain: MonitorChain,
}

impl IdentityDsp {
    pub fn new(chain: MonitorChain) -> Self {
        Self { chain }
    }
}

impl MonitorDsp for IdentityDsp {
    fn chain(&self) -> MonitorChain {
        self.chain
    }

    fn num_inputs(&self) -> usize {
        self.chain.num_inputs()
    }

    fn num_outputs(&self) -> usize {
        2
    }

    fn compute(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        if outputs.is_empty() || frames == 0 {
            return;
        }
        if inputs.is_empty() {
            for out in outputs.iter_mut() {
                out[..frames].fill(0.0);
            }
            return;
        }
        let src0 = &inputs[0][..frames];
        if inputs.len() == 1 || outputs.len() == 1 {
            outputs[0][..frames].copy_from_slice(src0);
            if outputs.len() > 1 {
                outputs[1][..frames].copy_from_slice(src0);
            }
            return;
        }
        outputs[0][..frames].copy_from_slice(src0);
        outputs[1][..frames].copy_from_slice(&inputs[1][..frames]);
    }

    fn set_param(&mut self, _address: &str, _value: f32) {}

    fn get_param(&self, _address: &str) -> Option<f32> {
        None
    }

    fn ui_json(&self) -> &'static str {
        r#"{"name":"identity","inputs":0,"outputs":2,"ui":[]}"#
    }

    fn meter_addresses(&self) -> &[String] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_named(dsp: &mut dyn MonitorDsp, needle: &str, value: f32) {
        let root = parse_ui_json(dsp.ui_json()).expect("ui json");
        for (address, passive) in collect_addresses(&root) {
            if !passive && address.contains(needle) {
                dsp.set_param(&address, value);
                return;
            }
        }
        panic!("no live Faust address containing {needle}");
    }

    fn process_block(dsp: &mut dyn MonitorDsp, inputs: &[&[f32]]) -> (Vec<f32>, Vec<f32>) {
        let frames = inputs.first().map(|buf| buf.len()).unwrap_or(0);
        let zeros = vec![0.0f32; frames.max(1)];
        let n_in = dsp.num_inputs();
        let refs: Vec<&[f32]> = (0..n_in)
            .map(|i| {
                if i < inputs.len() {
                    inputs[i]
                } else {
                    &zeros[..frames]
                }
            })
            .collect();
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        dsp.compute(frames, &refs, &mut [&mut left, &mut right]);
        (left, right)
    }

    fn settle(dsp: &mut dyn MonitorDsp) {
        let n_in = dsp.num_inputs();
        let zeros = vec![0.0f32; 4096];
        let refs: Vec<&[f32]> = (0..n_in).map(|_| zeros.as_slice()).collect();
        let mut left = vec![0.0; 4096];
        let mut right = vec![0.0; 4096];
        dsp.compute(4096, &refs, &mut [&mut left, &mut right]);
    }

    fn addresses_of(chain: MonitorChain) -> Vec<String> {
        let dsp = create_dsp(chain, 48000);
        let root = parse_ui_json(dsp.ui_json()).unwrap();
        collect_addresses(&root)
            .into_iter()
            .map(|(address, _)| address)
            .collect()
    }

    #[test]
    fn identity_duplicates_mono() {
        let mut dsp = IdentityDsp::new(MonitorChain::Mono);
        let input = vec![0.5f32, -0.25];
        let mut left = vec![0.0; 2];
        let mut right = vec![0.0; 2];
        dsp.compute(2, &[&input], &mut [&mut left, &mut right]);
        assert_eq!(left, input);
        assert_eq!(right, input);
    }

    #[test]
    fn faust_mono_duplicates_at_unity_gain() {
        let mut dsp = create_dsp(MonitorChain::Mono, 44100);
        set_named(dsp.as_mut(), "Gain", 0.0);
        settle(dsp.as_mut());
        let input = vec![0.25f32, -0.5, 0.125];
        let (left, right) = process_block(dsp.as_mut(), &[&input]);
        for i in 0..input.len() {
            assert!((left[i] - input[i]).abs() < 0.02, "{}", left[i]);
            assert!((right[i] - input[i]).abs() < 0.02, "{}", right[i]);
        }
    }

    #[test]
    fn stereo_width_full_is_dry_and_zero_is_mid() {
        let mut dsp = create_dsp(MonitorChain::Stereo, 48000);
        set_named(dsp.as_mut(), "Crossfeed", 0.0);
        set_named(dsp.as_mut(), "Gain", 0.0);
        set_named(dsp.as_mut(), "Width", 100.0);
        settle(dsp.as_mut());
        let left_in = vec![0.5f32, -0.25];
        let right_in = vec![-0.1f32, 0.8];
        let (left, right) = process_block(dsp.as_mut(), &[&left_in, &right_in]);
        assert!((left[0] - 0.5).abs() < 0.03, "{}", left[0]);
        assert!((right[0] + 0.1).abs() < 0.03, "{}", right[0]);

        set_named(dsp.as_mut(), "Width", 0.0);
        settle(dsp.as_mut());
        let left_in = vec![1.0f32];
        let right_in = vec![-1.0f32];
        let (left, right) = process_block(dsp.as_mut(), &[&left_in, &right_in]);
        assert!(left[0].abs() < 0.05, "{}", left[0]);
        assert!(right[0].abs() < 0.05, "{}", right[0]);
    }

    #[test]
    fn crossfeed_off_is_dry_and_on_leaks_left_into_right() {
        let mut dsp = create_dsp(MonitorChain::Stereo, 48000);
        set_named(dsp.as_mut(), "Crossfeed", 0.0);
        set_named(dsp.as_mut(), "Gain", 0.0);
        set_named(dsp.as_mut(), "Width", 100.0);
        settle(dsp.as_mut());
        let left_in = vec![0.8f32, 0.8, 0.8, 0.8];
        let right_in = vec![0.0f32; 4];
        let (left, right) = process_block(dsp.as_mut(), &[&left_in, &right_in]);
        assert!((left[3] - 0.8).abs() < 0.03);
        assert_eq!(right[3], 0.0);

        set_named(dsp.as_mut(), "Crossfeed", 1.0);
        set_named(dsp.as_mut(), "Amount", 0.5);
        settle(dsp.as_mut());
        let left_in = vec![0.8f32; 64];
        let right_in = vec![0.0f32; 64];
        let (_, right) = process_block(dsp.as_mut(), &[&left_in, &right_in]);
        assert!(
            right[63].abs() > 0.02,
            "crossfeed should leak delayed/lowpassed L into R, got {}",
            right[63]
        );
    }

    #[test]
    fn ms_unscaled_decode() {
        let mut dsp = create_dsp(MonitorChain::Ms, 48000);
        set_named(dsp.as_mut(), "Crossfeed", 0.0);
        set_named(dsp.as_mut(), "Mid_trim", 0.0);
        set_named(dsp.as_mut(), "Side_trim", 0.0);
        settle(dsp.as_mut());
        let mid = vec![0.4f32];
        let side = vec![0.1f32];
        let (left, right) = process_block(dsp.as_mut(), &[&mid, &side]);
        assert!((left[0] - 0.5).abs() < 0.03, "{}", left[0]);
        assert!((right[0] - 0.3).abs() < 0.03, "{}", right[0]);
    }

    #[test]
    fn foa_omni_w_is_equal_and_y_pans() {
        let mut dsp = create_dsp(MonitorChain::Foa, 48000);
        set_named(dsp.as_mut(), "Gain", 0.0);
        set_named(dsp.as_mut(), "Yaw", 0.0);
        set_named(dsp.as_mut(), "Orientation", 0.0);
        settle(dsp.as_mut());
        let one = vec![1.0f32];
        let zero = vec![0.0f32];
        let (left, right) = process_block(dsp.as_mut(), &[&one, &zero, &zero, &zero]);
        assert!((left[0] - 0.7071).abs() < 0.04, "{}", left[0]);
        assert!((right[0] - left[0]).abs() < 0.02);

        let (left, right) = process_block(dsp.as_mut(), &[&zero, &one, &zero, &zero]);
        assert!(left[0] > 0.3, "{}", left[0]);
        assert!(right[0] < -0.3, "{}", right[0]);
    }

    #[test]
    fn foa_fuma_w_matches_ambix_after_sn3d_and_x_is_not_y() {
        let one = vec![1.0f32];
        let zero = vec![0.0f32];
        let w_fuma = vec![1.0 / 2.0f32.sqrt()];

        let mut ambix = create_dsp(MonitorChain::Foa, 48000);
        set_named(ambix.as_mut(), "Gain", 0.0);
        set_named(ambix.as_mut(), "Yaw", 0.0);
        set_named(ambix.as_mut(), "Orientation", 0.0);
        settle(ambix.as_mut());
        let (aw, _) = process_block(ambix.as_mut(), &[&one, &zero, &zero, &zero]);
        let (ax, _) = process_block(ambix.as_mut(), &[&zero, &zero, &zero, &one]);
        let (ay_l, ay_r) = process_block(ambix.as_mut(), &[&zero, &one, &zero, &zero]);

        let mut fuma = create_dsp(MonitorChain::FoaFuma, 48000);
        set_named(fuma.as_mut(), "Gain", 0.0);
        set_named(fuma.as_mut(), "Yaw", 0.0);
        set_named(fuma.as_mut(), "Orientation", 0.0);
        settle(fuma.as_mut());
        let (fw, _) = process_block(fuma.as_mut(), &[&w_fuma, &zero, &zero, &zero]);
        let (fx_l, fx_r) = process_block(fuma.as_mut(), &[&zero, &one, &zero, &zero]);
        let (fy_l, fy_r) = process_block(fuma.as_mut(), &[&zero, &zero, &one, &zero]);

        assert!(
            (fw[0] - aw[0]).abs() < 0.04,
            "fuma W {} ambix W {}",
            fw[0],
            aw[0]
        );
        assert!(
            (fx_l[0] - ax[0]).abs() < 0.04 && (fx_r[0] - fx_l[0]).abs() < 0.02,
            "fuma ch1 should be Ambix X, got L={} R={}",
            fx_l[0],
            fx_r[0]
        );
        assert!(
            (fy_l[0] + ay_l[0]).abs() < 0.04 && (fy_r[0] + ay_r[0]).abs() < 0.04,
            "fuma +Y should pan opposite Ambix +Y, ambix {}/{} fuma {}/{}",
            ay_l[0],
            ay_r[0],
            fy_l[0],
            fy_r[0]
        );
    }

    #[test]
    fn foa_orientation_down_flips_y_and_endfire_maps_z_to_x() {
        let one = vec![1.0f32];
        let zero = vec![0.0f32];

        let mut dsp = create_dsp(MonitorChain::Foa, 48000);
        set_named(dsp.as_mut(), "Gain", 0.0);
        set_named(dsp.as_mut(), "Yaw", 0.0);
        set_named(dsp.as_mut(), "Orientation", 0.0);
        settle(dsp.as_mut());
        let (up_w_l, up_w_r) = process_block(dsp.as_mut(), &[&one, &zero, &zero, &zero]);
        let (up_y_l, up_y_r) = process_block(dsp.as_mut(), &[&zero, &one, &zero, &zero]);
        let (up_neg_x_l, up_neg_x_r) =
            process_block(dsp.as_mut(), &[&zero, &zero, &zero, &vec![-1.0]]);

        set_named(dsp.as_mut(), "Orientation", 1.0);
        settle(dsp.as_mut());
        let (down_y_l, down_y_r) = process_block(dsp.as_mut(), &[&zero, &one, &zero, &zero]);
        assert!(
            (down_y_l[0] + up_y_l[0]).abs() < 0.04 && (down_y_r[0] + up_y_r[0]).abs() < 0.04,
            "Down should flip Y pan vs Up, up {}/{} down {}/{}",
            up_y_l[0],
            up_y_r[0],
            down_y_l[0],
            down_y_r[0]
        );

        set_named(dsp.as_mut(), "Orientation", 2.0);
        settle(dsp.as_mut());
        let (end_w_l, end_w_r) = process_block(dsp.as_mut(), &[&one, &zero, &zero, &zero]);
        let (end_z_l, end_z_r) = process_block(dsp.as_mut(), &[&zero, &zero, &one, &zero]);
        assert!((end_w_l[0] - up_w_l[0]).abs() < 0.04);
        assert!((end_w_r[0] - up_w_r[0]).abs() < 0.04);
        assert!(
            (end_z_l[0] - up_neg_x_l[0]).abs() < 0.04 && (end_z_r[0] - up_neg_x_r[0]).abs() < 0.04,
            "Endfire +Z should match Up -X, got {}/{} vs {}/{}",
            end_z_l[0],
            end_z_r[0],
            up_neg_x_l[0],
            up_neg_x_r[0]
        );
    }

    #[test]
    fn stereo_and_ms_schema_include_headphones() {
        for chain in [MonitorChain::Stereo, MonitorChain::Ms] {
            let addresses = addresses_of(chain);
            assert!(
                addresses
                    .iter()
                    .any(|a| a.contains("Headphones") && a.contains("Crossfeed")),
                "{chain:?} {addresses:?}"
            );
            assert!(addresses.iter().any(|a| a.contains("Amount")));
            assert!(addresses.iter().any(|a| a.contains("Crossover")));
        }
        let mono = addresses_of(MonitorChain::Mono);
        assert!(!mono.iter().any(|a| a.contains("Headphones")));
        for chain in [MonitorChain::Foa, MonitorChain::FoaFuma] {
            let addresses = addresses_of(chain);
            assert!(!addresses.iter().any(|a| a.contains("Headphones")));
            assert!(
                addresses.iter().any(|a| a.contains("Orientation")),
                "{chain:?} {addresses:?}"
            );
            assert!(addresses.iter().any(|a| a.contains("Yaw")));
        }
    }
}
