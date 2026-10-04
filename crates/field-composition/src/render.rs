// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Multi-output composition render with optional offline DSP.
//!
//! [`render_outputs`] amortizes a single composition read across all N
//! outputs.  An optional shared DSP stage runs on the decoded source before
//! per-output DSP (reserved for future use; leave `None` in v1).  Each output
//! then selects channels, runs its own offline DSP chain if any, applies SRC
//! if needed, and encodes to disk — **in parallel** across outputs after the
//! shared read/DSP stage.
//!
//! The [`OfflineDsp`] trait is defined here so that `field-composition` does
//! **not** acquire a dependency on `field-audio-monitor`.  Implementations
//! live in higher-level crates (e.g. `field-assist`) and are injected by the
//! caller.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Context, Result};
use field_audio_io::{encoder, select_channels, EncodeSpec, FormatEncoder, TagMap};
use field_audio_process::resample_planar;
use field_core::ProgressHandle;

use crate::Composition;

/// Frame-block size for offline DSP / progress updates.
const RENDER_CHUNK_FRAMES: usize = 16_384;

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Offline (non-realtime) DSP transform applied per render output.
///
/// Implementations wrap compiled Faust monitor chains (FOA, M/S, …) or
/// provide test stubs.  `field-composition` owns the trait; higher-level
/// crates supply concrete types so no new crate edge toward
/// `field-audio-monitor` is introduced.
pub trait OfflineDsp: Send {
    /// Stable chain identifier (e.g. `"foa"`, `"ms"`).
    fn chain_id(&self) -> &str;
    /// Number of input channels consumed.
    fn num_inputs(&self) -> usize;
    /// Number of output channels produced.
    fn num_outputs(&self) -> usize;
    /// Apply DSP to `input` planar audio and return output planar audio.
    ///
    /// `input[ch][frame]` — input channels may be fewer than `num_inputs()`;
    /// missing channels are treated as silence.
    ///
    /// May be called repeatedly with consecutive frame blocks; DSP state
    /// carries across calls.
    fn process(&mut self, input: &[Vec<f32>]) -> Result<Vec<Vec<f32>>>;
}

// ── Plan types ─────────────────────────────────────────────────────────────────

/// One output within a [`RenderPlan`].
pub struct RenderOutput {
    /// Destination path.
    pub path: PathBuf,
    /// Encoder id (e.g. `"flac"`, `"wav"`).
    pub encoder_id: String,
    /// Encode spec (rate, format, channel count).
    pub spec: EncodeSpec,
    /// Composition channel indices fed into this output's DSP (or encoded
    /// directly for identity outputs).
    pub channel_indices: Vec<usize>,
    /// Metadata tags to embed.
    pub tags: TagMap,
    /// Optional pre-constructed offline DSP; `None` = identity encode.
    pub dsp: Option<Box<dyn OfflineDsp>>,
}

impl RenderOutput {
    /// Convenience constructor for an identity (no-DSP) output.
    pub fn identity(
        path: impl Into<PathBuf>,
        encoder_id: impl Into<String>,
        spec: EncodeSpec,
        channel_indices: Vec<usize>,
    ) -> Self {
        Self {
            path: path.into(),
            encoder_id: encoder_id.into(),
            spec,
            channel_indices,
            tags: TagMap::new(),
            dsp: None,
        }
    }
}

/// Render plan: optional shared DSP stage plus per-output specs.
///
/// The shared stage is reserved for future use (e.g. a normalisation pass
/// applied before all outputs).  Set `shared_dsp` to `None` for v1 Catalog.
pub struct RenderPlan {
    /// Shared DSP applied to decoded source audio before any per-output DSP.
    /// `None` = noop (no shared stage).
    pub shared_dsp: Option<Box<dyn OfflineDsp>>,
    /// Per-output render specs, processed in parallel after the shared stage.
    pub outputs: Vec<RenderOutput>,
}

impl RenderPlan {
    /// Create a plan with no shared DSP.
    pub fn new(outputs: Vec<RenderOutput>) -> Self {
        Self {
            shared_dsp: None,
            outputs,
        }
    }
}

// ── Results ────────────────────────────────────────────────────────────────────

/// Result for a single render output.
#[derive(Debug)]
pub enum OutputResult {
    /// File was written successfully.
    Written,
    /// Destination already existed; this output was skipped.
    Skipped(String),
    /// An error occurred; other outputs continue.
    Failed(String),
}

/// Collected results for all outputs in a [`RenderPlan`].
pub struct RenderResults {
    /// One entry per output in plan order: `(destination_path, result)`.
    pub outputs: Vec<(PathBuf, OutputResult)>,
}

impl RenderResults {
    /// `true` when every output was written (no skips or failures).
    pub fn all_written(&self) -> bool {
        self.outputs
            .iter()
            .all(|(_, r)| matches!(r, OutputResult::Written))
    }

    /// Paths of outputs that were skipped because the destination existed.
    pub fn skipped_paths(&self) -> Vec<&PathBuf> {
        self.outputs
            .iter()
            .filter_map(|(p, r)| matches!(r, OutputResult::Skipped(_)).then_some(p))
            .collect()
    }

    /// Paths of outputs that failed with an error.
    pub fn failed_paths(&self) -> Vec<&PathBuf> {
        self.outputs
            .iter()
            .filter_map(|(p, r)| matches!(r, OutputResult::Failed(_)).then_some(p))
            .collect()
    }
}

/// Shared frame-progress counters for a multi-output render.
///
/// [`Self::done`] advances from `0` to [`Self::total`] in **source frames**
/// (composition length).  With N parallel outputs, internal work is
/// `N × frames`; the displayed value is the average so the scale stays
/// `0..=total`.
#[derive(Clone)]
pub struct FrameProgress {
    /// Frames completed on the `0..=total` display scale.
    pub done: Arc<AtomicU64>,
    /// Composition source frame count.
    pub total: u64,
    work: Arc<AtomicU64>,
    n_outputs: u64,
}

impl FrameProgress {
    /// Create progress tracking for `n_outputs` workers over `total` frames.
    pub fn new(total: u64, n_outputs: usize) -> Self {
        Self {
            done: Arc::new(AtomicU64::new(0)),
            total,
            work: Arc::new(AtomicU64::new(0)),
            n_outputs: n_outputs.max(1) as u64,
        }
    }

    fn add_output_frames(&self, frames: u64) {
        if frames == 0 || self.total == 0 {
            return;
        }
        let w = self.work.fetch_add(frames, Ordering::Relaxed) + frames;
        let display = (w / self.n_outputs).min(self.total);
        self.done.store(display, Ordering::Release);
    }

    fn finish(&self) {
        self.done.store(self.total, Ordering::Release);
    }
}

// ── Core function ──────────────────────────────────────────────────────────────

/// Render `composition` to all outputs in `plan` with a single amortized read.
///
/// Steps:
///
/// 1. Read all composition frames into planar buffers once.
/// 2. Apply shared DSP stage (if any).
/// 3. For each output **in parallel**:
///    a. Re-check if the destination already exists → skip if so.
///    b. Select the requested source channels.
///    c. Apply per-output offline DSP (if any), in frame chunks.
///    d. Sample-rate-convert if needed.
///    e. Encode to the destination path.
///
/// Errors in individual outputs are collected into [`OutputResult::Failed`];
/// they do not abort the remaining outputs.  A hard `Err` is returned only
/// for plan-level failures (empty plan, failed composition read).
pub fn render_outputs(
    composition: &Composition,
    plan: &mut RenderPlan,
    progress: Option<&ProgressHandle>,
    epoch: u64,
) -> Result<RenderResults> {
    if plan.outputs.is_empty() {
        bail!("render plan must contain at least one output");
    }

    // ── Amortized source read ──────────────────────────────────────────────
    let frames = composition.frames();
    let ch = composition.channel_count();
    let source_rate = composition.sample_rate();
    let mut planes = vec![vec![0.0f32; frames as usize]; ch];
    {
        let mut refs: Vec<&mut [f32]> = planes.iter_mut().map(|p| p.as_mut_slice()).collect();
        composition
            .read_planar(0, frames, &mut refs)
            .context("read composition for render")?;
    }
    if let Some(p) = progress {
        p.set_fraction(epoch, 0.05);
    }

    render_from_planes(planes, source_rate, plan, progress, epoch, None)
}

/// Render from pre-decoded planar audio without re-reading composition media.
///
/// Useful when the caller has already read the source into memory —
/// for example, to hand off audio data to a background thread.
/// `source_rate` is the sample rate of the `planes` data; per-output SRC
/// converts to each output's target rate as needed.
///
/// The empty-plan check and all per-output work proceed identically to
/// [`render_outputs`].  The optional shared-DSP stage is applied first.
pub fn render_outputs_from_planes(
    planes: Vec<Vec<f32>>,
    source_rate: u32,
    plan: &mut RenderPlan,
    progress: Option<&ProgressHandle>,
    epoch: u64,
) -> Result<RenderResults> {
    render_outputs_from_planes_with_progress(planes, source_rate, plan, progress, epoch, None)
}

/// Like [`render_outputs_from_planes`], updating [`FrameProgress`] as source
/// frames are processed across parallel outputs.
pub fn render_outputs_from_planes_with_progress(
    planes: Vec<Vec<f32>>,
    source_rate: u32,
    plan: &mut RenderPlan,
    progress: Option<&ProgressHandle>,
    epoch: u64,
    frame_progress: Option<&FrameProgress>,
) -> Result<RenderResults> {
    if plan.outputs.is_empty() {
        bail!("render plan must contain at least one output");
    }
    render_from_planes(planes, source_rate, plan, progress, epoch, frame_progress)
}

/// Shared inner: apply shared DSP, then process outputs in parallel.
fn render_from_planes(
    mut planes: Vec<Vec<f32>>,
    source_rate: u32,
    plan: &mut RenderPlan,
    progress: Option<&ProgressHandle>,
    epoch: u64,
    frame_progress: Option<&FrameProgress>,
) -> Result<RenderResults> {
    // ── Optional shared DSP stage (noop in v1) ─────────────────────────────
    if let Some(shared) = plan.shared_dsp.as_mut() {
        planes = shared.process(&planes).context("shared DSP stage")?;
    }

    let frames = planes.first().map(|ch| ch.len()).unwrap_or(0) as u64;
    let n = plan.outputs.len();
    let owned_progress;
    let frame_progress = match frame_progress {
        Some(p) => p,
        None => {
            owned_progress = FrameProgress::new(frames, n);
            &owned_progress
        }
    };

    let planes = Arc::new(planes);
    let mut handles = Vec::with_capacity(n);
    let outputs = std::mem::take(&mut plan.outputs);

    for (index, output) in outputs.into_iter().enumerate() {
        let planes = Arc::clone(&planes);
        let fp = frame_progress.clone();
        handles.push(
            thread::Builder::new()
                .name(format!("fa-render-out-{index}"))
                .spawn(move || {
                    let result = render_one_output(&planes, source_rate, output, &fp);
                    (index, result)
                })
                .context("spawn render output thread")?,
        );
    }

    let mut slotted: Vec<Option<(PathBuf, OutputResult)>> = (0..n).map(|_| None).collect();
    for handle in handles {
        match handle.join() {
            Ok((index, entry)) => {
                if let Some(slot) = slotted.get_mut(index) {
                    *slot = Some(entry);
                }
            }
            Err(_) => {
                bail!("render output thread panicked");
            }
        }
        if let Some(p) = progress {
            let done = frame_progress.done.load(Ordering::Acquire);
            let frac = if frames == 0 {
                1.0
            } else {
                0.05 + 0.95 * (done as f64 / frames as f64)
            };
            p.set_fraction(epoch, frac as f32);
        }
    }

    frame_progress.finish();
    if let Some(p) = progress {
        p.set_fraction(epoch, 1.0);
    }

    let results: Vec<(PathBuf, OutputResult)> = slotted
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            slot.unwrap_or_else(|| {
                (
                    PathBuf::from(format!("<missing-{i}>")),
                    OutputResult::Failed("missing output result".into()),
                )
            })
        })
        .collect();

    Ok(RenderResults { outputs: results })
}

fn render_one_output(
    planes: &[Vec<f32>],
    source_rate: u32,
    mut output: RenderOutput,
    progress: &FrameProgress,
) -> (PathBuf, OutputResult) {
    let path = output.path.clone();
    let source_frames = planes.first().map(|ch| ch.len()).unwrap_or(0) as u64;

    if path.exists() {
        progress.add_output_frames(source_frames);
        return (
            path,
            OutputResult::Skipped(format!(
                "destination already exists: {}",
                output.path.display()
            )),
        );
    }

    let enc = match encoder(&output.encoder_id) {
        Some(e) => e,
        None => {
            progress.add_output_frames(source_frames);
            return (
                path,
                OutputResult::Failed(format!("unknown encoder `{}`", output.encoder_id)),
            );
        }
    };

    let selected = select_channels(planes, &output.channel_indices);

    let post_dsp = if let Some(dsp) = output.dsp.as_mut() {
        let mut credited = 0u64;
        match process_dsp_chunked(dsp.as_mut(), &selected, |n| {
            credited += n;
            progress.add_output_frames(n);
        }) {
            Ok(out) => out,
            Err(e) => {
                progress.add_output_frames(source_frames.saturating_sub(credited));
                return (path, OutputResult::Failed(e.to_string()));
            }
        }
    } else {
        // Identity: attribute frame work up front (encode is the remaining cost).
        progress.add_output_frames(source_frames);
        selected
    };

    let planar = if source_rate == output.spec.sample_rate {
        post_dsp
    } else {
        match resample_planar(&post_dsp, source_rate, output.spec.sample_rate) {
            Ok(r) => r,
            Err(e) => return (path, OutputResult::Failed(e.to_string())),
        }
    };

    let mut encode_spec = output.spec.clone();
    encode_spec.channel_count = planar.len() as u16;
    match write_render_output(enc, &encode_spec, &planar, &path, &output.tags) {
        Ok(()) => (path, OutputResult::Written),
        Err(e) => (path, OutputResult::Failed(e.to_string())),
    }
}

/// Run offline DSP in consecutive frame blocks so progress can advance and
/// Faust state carries across chunks.
fn process_dsp_chunked(
    dsp: &mut dyn OfflineDsp,
    input: &[Vec<f32>],
    mut on_chunk: impl FnMut(u64),
) -> Result<Vec<Vec<f32>>> {
    let frames = input.first().map(|ch| ch.len()).unwrap_or(0);
    if frames == 0 {
        return dsp.process(input);
    }
    let n_out = dsp.num_outputs().max(1);
    let mut out: Vec<Vec<f32>> = (0..n_out).map(|_| Vec::with_capacity(frames)).collect();
    let mut start = 0;
    while start < frames {
        let end = (start + RENDER_CHUNK_FRAMES).min(frames);
        let slice: Vec<Vec<f32>> = input
            .iter()
            .map(|ch| ch.get(start..end).unwrap_or(&[]).to_vec())
            .collect();
        let part = dsp.process(&slice)?;
        if part.len() != out.len() {
            // Allow DSP to report a different channel count on first chunk.
            if start == 0 {
                out = part
                    .iter()
                    .map(|ch| {
                        let mut v = Vec::with_capacity(frames);
                        v.extend_from_slice(ch);
                        v
                    })
                    .collect();
            } else {
                bail!(
                    "DSP channel count changed mid-render ({} → {})",
                    out.len(),
                    part.len()
                );
            }
        } else {
            for (dst, src) in out.iter_mut().zip(part.iter()) {
                dst.extend_from_slice(src);
            }
        }
        on_chunk((end - start) as u64);
        start = end;
    }
    Ok(out)
}

fn write_render_output(
    encoder: &dyn FormatEncoder,
    spec: &EncodeSpec,
    planar: &[Vec<f32>],
    dest: &Path,
    tags: &TagMap,
) -> Result<()> {
    // Write to a sibling temp file then rename so encode failures never leave
    // an empty/truncated destination (Sononym and other browsers reject those).
    let tmp = dest.with_extension(format!(
        "{}.partial",
        dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
    ));
    let _ = std::fs::remove_file(&tmp);
    let write_result = (|| -> Result<()> {
        let file =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        let mut writer = BufWriter::new(file);
        encoder.encode_with_tags(spec, planar, &mut writer, tags)?;
        writer.flush().context("flush render output")?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(dest);
        return Err(err);
    }
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("rename {} → {}", tmp.display(), dest.display()))?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use field_audio_io::{decode, EncodeSpec, PcmFormat};
    use field_audio_model::MediaRef;

    use super::*;

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

    fn stereo_spec(rate: u32) -> EncodeSpec {
        EncodeSpec {
            sample_rate: rate,
            sample_format: Some(PcmFormat::S16),
            channel_count: 2,
        }
    }

    fn mono_spec(rate: u32) -> EncodeSpec {
        EncodeSpec {
            sample_rate: rate,
            sample_format: Some(PcmFormat::S16),
            channel_count: 1,
        }
    }

    // ── 2.2: Identity encode ──────────────────────────────────────────────────

    #[test]
    fn identity_export_writes_one_flac() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(4096, 2, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-render-identity.flac");
        let _ = std::fs::remove_file(&dest);

        let mut plan = RenderPlan::new(vec![RenderOutput::identity(
            dest.clone(),
            "flac",
            stereo_spec(48_000),
            vec![0, 1],
        )]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();

        assert!(results.all_written(), "{:?}", results.outputs);
        let decoded = decode(&dest).unwrap();
        assert_eq!(decoded.channel_count(), 2);
        assert_eq!(decoded.sample_rate, 48_000);
        let _ = std::fs::remove_file(&dest);
    }

    // ── 2.5: Skip on existing dest ───────────────────────────────────────────

    #[test]
    fn existing_dest_is_skipped_not_overwritten() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(512, 1, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-render-skip-existing.wav");

        // Pre-create the destination.
        std::fs::write(&dest, b"placeholder").unwrap();

        let mut plan = RenderPlan::new(vec![RenderOutput::identity(
            dest.clone(),
            "wav",
            mono_spec(48_000),
            vec![0],
        )]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();

        assert_eq!(results.outputs.len(), 1);
        assert!(
            matches!(results.outputs[0].1, OutputResult::Skipped(_)),
            "expected Skipped, got {:?}",
            results.outputs[0].1
        );
        // Original placeholder must still be intact.
        assert_eq!(std::fs::read(&dest).unwrap(), b"placeholder");
        let _ = std::fs::remove_file(&dest);
    }

    // ── 2.5: Multi-output, partial skip ──────────────────────────────────────

    #[test]
    fn multi_output_partial_skip_continues_remaining() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(1024, 2, 48_000)).unwrap();
        let dir = std::env::temp_dir();
        let dest_a = dir.join("fa-render-multi-a.wav");
        let dest_b = dir.join("fa-render-multi-b.wav");

        // Pre-create dest_a to trigger a skip.
        std::fs::write(&dest_a, b"old").unwrap();
        let _ = std::fs::remove_file(&dest_b);

        let mut plan = RenderPlan::new(vec![
            RenderOutput::identity(dest_a.clone(), "wav", stereo_spec(48_000), vec![0, 1]),
            RenderOutput::identity(dest_b.clone(), "wav", stereo_spec(48_000), vec![0, 1]),
        ]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();

        assert_eq!(results.outputs.len(), 2);
        assert!(matches!(results.outputs[0].1, OutputResult::Skipped(_)));
        assert!(matches!(results.outputs[1].1, OutputResult::Written));
        assert_eq!(std::fs::read(&dest_a).unwrap(), b"old");
        let _ = std::fs::remove_file(&dest_a);
        let _ = std::fs::remove_file(&dest_b);
    }

    // ── 2.3: OfflineDsp stub → stereo channel count ───────────────────────────

    /// Minimal stub that down-mixes N→2 by averaging pairs.
    struct StereoSumDsp {
        n_in: usize,
    }

    impl OfflineDsp for StereoSumDsp {
        fn chain_id(&self) -> &str {
            "test-stereo-sum"
        }
        fn num_inputs(&self) -> usize {
            self.n_in
        }
        fn num_outputs(&self) -> usize {
            2
        }
        fn process(&mut self, input: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
            let frames = input.first().map(|ch| ch.len()).unwrap_or(0);
            let empty = vec![0.0f32; frames];
            let left = input.first().unwrap_or(&empty);
            let right = if input.len() > 1 { &input[1] } else { left };
            Ok(vec![left.to_vec(), right.to_vec()])
        }
    }

    #[test]
    fn offline_dsp_stub_produces_stereo_from_4ch_source() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(2048, 4, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-render-dsp-stereo.wav");
        let _ = std::fs::remove_file(&dest);

        let dsp = Box::new(StereoSumDsp { n_in: 4 });
        let output = RenderOutput {
            path: dest.clone(),
            encoder_id: "wav".into(),
            spec: stereo_spec(48_000),
            channel_indices: vec![0, 1, 2, 3],
            tags: TagMap::new(),
            dsp: Some(dsp),
        };
        let mut plan = RenderPlan::new(vec![output]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();

        assert!(results.all_written(), "{:?}", results.outputs);
        let decoded = decode(&dest).unwrap();
        assert_eq!(decoded.channel_count(), 2, "DSP output must be stereo");
        let _ = std::fs::remove_file(&dest);
    }

    /// Catalog path: export job keeps pre-DSP channel_count (4 for Ambix) while
    /// FOA/M/S DSP emits stereo. Encode must follow planar length, not the
    /// stale profile count — otherwise FLAC fails and left 0-byte files.
    #[test]
    fn offline_dsp_flac_tolerates_stale_profile_channel_count() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(2048, 4, 48_000)).unwrap();
        let dest = std::env::temp_dir().join("fa-render-dsp-stale-ch.flac");
        let _ = std::fs::remove_file(&dest);

        let dsp = Box::new(StereoSumDsp { n_in: 4 });
        let output = RenderOutput {
            path: dest.clone(),
            encoder_id: "flac".into(),
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S16),
                // Intentionally wrong: composition/profile channel count.
                channel_count: 4,
            },
            channel_indices: vec![0, 1, 2, 3],
            tags: TagMap::new(),
            dsp: Some(dsp),
        };
        let mut plan = RenderPlan::new(vec![output]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();

        assert!(results.all_written(), "{:?}", results.outputs);
        let meta = std::fs::metadata(&dest).unwrap();
        assert!(meta.len() > 0, "mixdown FLAC must not be empty");
        let decoded = decode(&dest).unwrap();
        assert_eq!(decoded.channel_count(), 2);
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn parallel_identity_and_dsp_outputs_both_write() {
        use crate::Composition;

        let frames = 8192usize;
        let comp = Composition::from_media(sine_media(frames, 4, 48_000)).unwrap();
        let dir = std::env::temp_dir();
        let dest_id = dir.join("fa-render-par-id.wav");
        let dest_st = dir.join("fa-render-par-st.wav");
        let _ = std::fs::remove_file(&dest_id);
        let _ = std::fs::remove_file(&dest_st);

        let fp = FrameProgress::new(frames as u64, 2);
        let mut plan = RenderPlan::new(vec![
            RenderOutput::identity(dest_id.clone(), "wav", stereo_spec(48_000), vec![0, 1]),
            RenderOutput {
                path: dest_st.clone(),
                encoder_id: "wav".into(),
                spec: stereo_spec(48_000),
                channel_indices: vec![0, 1, 2, 3],
                tags: TagMap::new(),
                dsp: Some(Box::new(StereoSumDsp { n_in: 4 })),
            },
        ]);
        let results = render_outputs_from_planes_with_progress(
            {
                let ch = comp.channel_count();
                let mut planes = vec![vec![0.0f32; frames]; ch];
                let mut refs: Vec<&mut [f32]> =
                    planes.iter_mut().map(|p| p.as_mut_slice()).collect();
                comp.read_planar(0, frames as u64, &mut refs).unwrap();
                planes
            },
            48_000,
            &mut plan,
            None,
            0,
            Some(&fp),
        )
        .unwrap();

        assert!(results.all_written(), "{:?}", results.outputs);
        assert_eq!(fp.done.load(Ordering::Acquire), frames as u64);
        assert_eq!(decode(&dest_id).unwrap().channel_count(), 2);
        assert_eq!(decode(&dest_st).unwrap().channel_count(), 2);
        let _ = std::fs::remove_file(&dest_id);
        let _ = std::fs::remove_file(&dest_st);
    }

    // ── 2.4: DSP params override ─────────────────────────────────────────────

    /// DSP stub that records whether a custom param was applied.
    struct ParamCaptureDsp {
        gain: f32,
        processed_with_gain: f32,
    }

    impl ParamCaptureDsp {
        const DEFAULT_GAIN: f32 = 1.0;

        fn new(params: &HashMap<String, f32>) -> Self {
            let gain = params.get("gain").copied().unwrap_or(Self::DEFAULT_GAIN);
            Self {
                gain,
                processed_with_gain: 0.0,
            }
        }
    }

    impl OfflineDsp for ParamCaptureDsp {
        fn chain_id(&self) -> &str {
            "test-param-capture"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            2
        }
        fn process(&mut self, input: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
            let frames = input.first().map(|ch| ch.len()).unwrap_or(0);
            self.processed_with_gain = self.gain;
            let plane: Vec<f32> = input
                .first()
                .map(|ch| ch.iter().map(|&s| s * self.gain).collect())
                .unwrap_or_else(|| vec![0.0; frames]);
            Ok(vec![plane.clone(), plane])
        }
    }

    #[test]
    fn per_output_params_override_dsp_defaults() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(512, 1, 48_000)).unwrap();
        let dir = std::env::temp_dir();
        let dest_default = dir.join("fa-render-param-default.wav");
        let dest_override = dir.join("fa-render-param-override.wav");
        let _ = std::fs::remove_file(&dest_default);
        let _ = std::fs::remove_file(&dest_override);

        let default_params: HashMap<String, f32> = HashMap::new();
        let override_params: HashMap<String, f32> =
            [("gain".to_string(), 0.5f32)].into_iter().collect();

        let dsp_default = Box::new(ParamCaptureDsp::new(&default_params));
        let dsp_override = Box::new(ParamCaptureDsp::new(&override_params));

        assert!(
            (dsp_default.gain - ParamCaptureDsp::DEFAULT_GAIN).abs() < 1e-6,
            "default gain should be 1.0"
        );
        assert!(
            (dsp_override.gain - 0.5).abs() < 1e-6,
            "override gain should be 0.5"
        );

        let mut plan = RenderPlan::new(vec![
            RenderOutput {
                path: dest_default.clone(),
                encoder_id: "wav".into(),
                spec: stereo_spec(48_000),
                channel_indices: vec![0],
                tags: TagMap::new(),
                dsp: Some(dsp_default),
            },
            RenderOutput {
                path: dest_override.clone(),
                encoder_id: "wav".into(),
                spec: stereo_spec(48_000),
                channel_indices: vec![0],
                tags: TagMap::new(),
                dsp: Some(dsp_override),
            },
        ]);
        let results = render_outputs(&comp, &mut plan, None, 0).unwrap();
        assert!(results.all_written(), "{:?}", results.outputs);

        // The override output should have been rendered at half gain —
        // verify by checking that the peak of the override is ≈ 0.5× default.
        let decoded_default = decode(&dest_default).unwrap();
        let decoded_override = decode(&dest_override).unwrap();
        let peak_default = decoded_default.channels[0]
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        let peak_override = decoded_override.channels[0]
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        assert!(
            (peak_override / peak_default - 0.5).abs() < 0.05,
            "override peak {peak_override:.4} should be ≈ 0.5 × default {peak_default:.4}"
        );

        let _ = std::fs::remove_file(&dest_default);
        let _ = std::fs::remove_file(&dest_override);
    }

    // ── Empty plan is an error ────────────────────────────────────────────────

    #[test]
    fn empty_plan_returns_error() {
        use crate::Composition;

        let comp = Composition::from_media(sine_media(64, 1, 48_000)).unwrap();
        let mut plan = RenderPlan::new(vec![]);
        assert!(render_outputs(&comp, &mut plan, None, 0).is_err());
    }
}
