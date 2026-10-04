// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Faust offline DSP adapter implementing [`field_composition::OfflineDsp`].
//!
//! [`FaustOfflineDsp`] wraps a compiled Faust monitor chain and drives it
//! synchronously (not on the CPAL callback), making it safe to call from any
//! non-realtime thread.

use std::collections::HashMap;

use anyhow::Result;
use field_audio_monitor::{create_dsp, MonitorChain, MonitorDsp};
use field_composition::OfflineDsp;

/// Offline Faust DSP for a single monitor chain.
///
/// Construct via [`FaustOfflineDsp::new`], supplying optional parameter
/// overrides.  Fixed Faust defaults are used for any parameter not present in
/// `params`.  The DSP is run synchronously — never on the CPAL output
/// callback — so callers must not call this from a realtime context.
pub struct FaustOfflineDsp {
    chain: MonitorChain,
    dsp: Box<dyn MonitorDsp>,
}

impl FaustOfflineDsp {
    /// Build an offline DSP for `chain` at `sample_rate`.
    ///
    /// `params` overrides Faust defaults; any address absent from `params`
    /// keeps its compiled default value.
    pub fn new(chain: MonitorChain, sample_rate: u32, params: &HashMap<String, f32>) -> Self {
        let mut dsp = create_dsp(chain, sample_rate);
        for (address, value) in params {
            dsp.set_param(address, *value);
        }
        Self { chain, dsp }
    }
}

impl OfflineDsp for FaustOfflineDsp {
    fn chain_id(&self) -> &str {
        self.chain.id()
    }

    fn num_inputs(&self) -> usize {
        self.dsp.num_inputs()
    }

    fn num_outputs(&self) -> usize {
        self.dsp.num_outputs()
    }

    /// Process `input` planar audio through the Faust chain.
    ///
    /// Input channels beyond [`num_inputs`] are ignored.  Missing input
    /// channels (fewer than `num_inputs`) are padded with silence.  Output
    /// always has exactly [`num_outputs`] channels (typically 2 for stereo).
    fn process(&mut self, input: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        let n_in = self.dsp.num_inputs();
        let n_out = self.dsp.num_outputs();
        let frames = input.first().map(|ch| ch.len()).unwrap_or(0);

        let silence = vec![0.0f32; frames];
        let input_refs: Vec<&[f32]> = (0..n_in)
            .map(|i| {
                input
                    .get(i)
                    .map(|ch| ch.as_slice())
                    .unwrap_or(silence.as_slice())
            })
            .collect();

        let mut out_bufs: Vec<Vec<f32>> = (0..n_out).map(|_| vec![0.0f32; frames]).collect();
        let mut out_refs: Vec<&mut [f32]> = out_bufs.iter_mut().map(|b| b.as_mut_slice()).collect();

        self.dsp.compute(frames, &input_refs, &mut out_refs);

        Ok(out_bufs)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use field_audio_io::{decode, EncodeSpec, PcmFormat};
    use field_audio_model::MediaRef;
    use field_audio_monitor::MonitorChain;
    use field_composition::Composition;
    use field_composition::{render_outputs, OfflineDsp, RenderOutput, RenderPlan};

    fn sine_media(frames: usize, channels: usize, rate: u32) -> MediaRef {
        let samples = (0..channels)
            .map(|ch| {
                (0..frames)
                    .map(|i| {
                        let t = i as f32 / rate as f32;
                        (t * (440.0 + ch as f32 * 110.0) * std::f32::consts::TAU).sin() * 0.5
                    })
                    .collect()
            })
            .collect();
        MediaRef::from_memory_samples(rate, samples)
    }

    // ── 2.3: FOA offline → stereo channel count ───────────────────────────────

    #[test]
    fn foa_offline_produces_stereo_from_4ch_composition() {
        let comp = Composition::from_media(sine_media(8192, 4, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-faust-offline-foa.wav");
        let _ = std::fs::remove_file(&dest);

        let dsp = Box::new(FaustOfflineDsp::new(
            MonitorChain::Foa,
            48_000,
            &HashMap::new(),
        ));
        assert_eq!(dsp.num_inputs(), 4);
        assert_eq!(dsp.num_outputs(), 2);

        let output = RenderOutput {
            path: dest.clone(),
            encoder_id: "wav".into(),
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::F32),
                channel_count: 2,
            },
            channel_indices: vec![0, 1, 2, 3],
            tags: Default::default(),
            dsp: Some(dsp),
        };
        let mut plan = RenderPlan::new(vec![output]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();
        assert!(results.all_written(), "{:?}", results.outputs);

        let decoded = decode(&dest).unwrap();
        assert_eq!(
            decoded.channel_count(),
            2,
            "FOA offline must produce stereo"
        );
        assert_eq!(decoded.sample_rate, 48_000);
        let _ = std::fs::remove_file(&dest);
    }

    /// Catalog Ambix→`-st.flac`: profile channel_count stays 4 while FOA DSP
    /// emits stereo. Encode must succeed with a non-empty FLAC.
    #[test]
    fn foa_offline_flac_with_stale_profile_channel_count() {
        let comp = Composition::from_media(sine_media(8192, 4, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-faust-offline-foa-st.flac");
        let _ = std::fs::remove_file(&dest);

        let dsp = Box::new(FaustOfflineDsp::new(
            MonitorChain::Foa,
            48_000,
            &HashMap::new(),
        ));
        let output = RenderOutput {
            path: dest.clone(),
            encoder_id: "flac".into(),
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S24),
                channel_count: 4, // pre-DSP Ambix count from build_job
            },
            channel_indices: vec![0, 1, 2, 3],
            tags: Default::default(),
            dsp: Some(dsp),
        };
        let mut plan = RenderPlan::new(vec![output]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();
        assert!(results.all_written(), "{:?}", results.outputs);
        assert!(std::fs::metadata(&dest).unwrap().len() > 0);
        let decoded = decode(&dest).unwrap();
        assert_eq!(decoded.channel_count(), 2);
        let _ = std::fs::remove_file(&dest);
    }

    // ── 2.3: MS offline → stereo channel count ───────────────────────────────

    #[test]
    fn ms_offline_produces_stereo_from_2ch_composition() {
        let comp = Composition::from_media(sine_media(8192, 2, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-faust-offline-ms.wav");
        let _ = std::fs::remove_file(&dest);

        let dsp = Box::new(FaustOfflineDsp::new(
            MonitorChain::Ms,
            48_000,
            &HashMap::new(),
        ));
        assert_eq!(dsp.num_inputs(), 2);
        assert_eq!(dsp.num_outputs(), 2);

        let output = RenderOutput {
            path: dest.clone(),
            encoder_id: "wav".into(),
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::F32),
                channel_count: 2,
            },
            channel_indices: vec![0, 1],
            tags: Default::default(),
            dsp: Some(dsp),
        };
        let mut plan = RenderPlan::new(vec![output]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();
        assert!(results.all_written(), "{:?}", results.outputs);

        let decoded = decode(&dest).unwrap();
        assert_eq!(decoded.channel_count(), 2, "MS offline must produce stereo");
        let _ = std::fs::remove_file(&dest);
    }

    // ── 2.4: Param override changes output amplitude ─────────────────────────

    #[test]
    fn foa_fuma_offline_accepts_param_overrides() {
        // Verify that FaustOfflineDsp::new correctly applies supplied params
        // by inspecting the DSP's stored value vs the Faust default.
        let chain = MonitorChain::FoaFuma;
        let params_default: HashMap<String, f32> = HashMap::new();
        let dsp_default = FaustOfflineDsp::new(chain, 48_000, &params_default);

        // Read the Gain control address.
        let gain_address = dsp_default
            .dsp
            .control_addresses()
            .into_iter()
            .find(|a| a.contains("Output_Gain"))
            .expect("FoaFuma DSP should have Output_Gain control");

        let default_gain = dsp_default
            .dsp
            .get_param(&gain_address)
            .expect("default gain readable");

        // Override with a different value.
        let override_val = default_gain - 6.0;
        let params_override: HashMap<String, f32> =
            [(gain_address.clone(), override_val)].into_iter().collect();
        let dsp_override = FaustOfflineDsp::new(chain, 48_000, &params_override);

        let got = dsp_override
            .dsp
            .get_param(&gain_address)
            .expect("override gain readable");
        assert!(
            (got - override_val).abs() < 0.5,
            "override {override_val} should be reflected in DSP, got {got}"
        );
    }
}
