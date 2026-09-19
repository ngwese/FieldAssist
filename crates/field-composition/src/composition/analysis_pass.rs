// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Shared multi-kind analysis pass: one planar read, many op consumes.

use std::sync::RwLock;
use std::time::Instant;

use anyhow::Result;
use field_audio_model::BLOCK_FRAMES;
use field_audio_process::{
    AnalysisKind, AnalysisSink, EnvelopePeakOp, MinMaxOp, RecomputeScope, SpectralOp,
    TransientDetectOp, SPECTRAL_BAND_COUNT,
};
use field_core::ProgressHandle;

use super::{
    advance_read_pos, next_chunk_in_ranges, peak_covered_samples, AnalysisBlockOutcome, Composition,
};
use crate::analysis_store::{expand_frame_ranges, merge_frame_ranges, AnalysisStreams, FrameRange};

/// Wall-time counters for one shared analysis pass (read vs consume).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnalysisPassStats {
    /// Nanoseconds spent in `read_planar` / warmup reads.
    pub read_ns: u64,
    /// Nanoseconds spent in op `consume` / flush / write-back.
    pub consume_ns: u64,
    /// Wall time spent in [`field_audio_model::BlockSource::decode_range`].
    pub decode_ns: u64,
    /// Blocks decoded via the block source during the pass.
    pub decodes: u64,
    /// Blocks served from the pager RAM LRU.
    pub ram_hits: u64,
    /// Blocks reloaded from disk spill.
    pub spill_hits: u64,
    /// Blocks written to spill when evicted from RAM.
    pub spill_writes: u64,
}

impl AnalysisPassStats {
    /// True when decode wall time dominates consume (I/O-bound pass).
    pub fn decode_dominates(self) -> bool {
        self.decode_ns >= self.consume_ns && self.decode_ns > 0
    }

    fn ms(ns: u64) -> f64 {
        ns as f64 / 1_000_000.0
    }

    /// One-line summary for the Messages panel / logs.
    pub fn info_message(self, kinds: &[AnalysisKind]) -> String {
        let label = AnalysisKind::progress_label_for_kinds(kinds);
        let bound = if self.decode_dominates() {
            "I/O-bound"
        } else if self.consume_ns >= self.read_ns && self.consume_ns >= self.decode_ns {
            "CPU-bound"
        } else {
            "mixed"
        };
        format!(
            "{label}: read {:.1}ms, consume {:.1}ms, decode {:.1}ms ({} blocks); \
             pager ram_hits={} spill_hits={} spill_writes={} — {bound}",
            Self::ms(self.read_ns),
            Self::ms(self.consume_ns),
            Self::ms(self.decode_ns),
            self.decodes,
            self.ram_hits,
            self.spill_hits,
            self.spill_writes,
        )
    }
}

impl Composition {
    /// Snapshot of the last / in-flight shared pass timings.
    pub fn analysis_pass_stats(&self) -> AnalysisPassStats {
        self.analysis_pass_stats
    }

    /// Run one pager-sized step of a multi-kind analysis pass.
    ///
    /// On the first call, plans a union of needed ranges expanded by each
    /// kind's recompute radius, then each step performs a single
    /// [`Self::read_planar`] and fans the PCM into every participating op.
    pub fn build_next_analysis_pass_block(
        composition: &RwLock<Self>,
        kinds: &[AnalysisKind],
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        let mut kinds = kinds.to_vec();
        kinds.sort_by_key(|k| k.id());
        kinds.dedup();
        if kinds.is_empty() {
            return Ok(AnalysisBlockOutcome::Complete);
        }
        // Always use the shared stepper (including one-kind passes) so
        // AnalysisPassStats is populated for Messages.
        if progress.is_some_and(|p| !p.is_epoch(epoch)) {
            let mut this = composition.write().unwrap();
            this.abandon_analysis_job_scratch();
            return Ok(AnalysisBlockOutcome::Cancelled);
        }

        Self::ensure_pass_started(composition, &kinds)?;

        let warmup_frames = {
            let this = composition.read().unwrap();
            this.analysis_pass_kinds
                .iter()
                .map(|k| match k.recompute_scope(this.sample_rate) {
                    RecomputeScope::Regional(r) => r.warmup_frames.max(r.radius_frames),
                    RecomputeScope::FullTimeline => 0,
                })
                .max()
                .unwrap_or(0)
        };

        let (channel_count, total, done, read_pos, take, range_start) = {
            let this = composition.read().unwrap();
            if this.analysis_target_done >= this.analysis_target_total {
                (
                    this.channel_count,
                    this.analysis_target_total,
                    this.analysis_target_done,
                    0u64,
                    0usize,
                    0u64,
                )
            } else if let Some((start, end, pos)) = next_chunk_in_ranges(
                &this.analysis_target_ranges,
                this.analysis_read_pos,
                BLOCK_FRAMES,
            ) {
                (
                    this.channel_count,
                    this.analysis_target_total,
                    this.analysis_target_done,
                    pos,
                    (end - pos) as usize,
                    start,
                )
            } else {
                (
                    this.channel_count,
                    this.analysis_target_total,
                    this.analysis_target_total,
                    0u64,
                    0usize,
                    0u64,
                )
            }
        };

        if take == 0 {
            return Self::finish_pass(composition, progress, epoch);
        }
        if let Some(progress) = progress {
            progress.set_ratio(epoch, done, total.max(1));
        }

        let sample_rate = composition.read().unwrap().sample_rate;
        let kinds_now = composition.read().unwrap().analysis_pass_kinds.clone();

        let mut prime_planar: Option<Vec<Vec<f32>>> = None;
        if read_pos == range_start && warmup_frames > 0 && range_start > 0 {
            let prime_start = range_start.saturating_sub(warmup_frames);
            let prime_len = (range_start - prime_start) as usize;
            let mut planar = vec![vec![0.0; prime_len]; channel_count];
            let t0 = Instant::now();
            {
                let this = composition.read().unwrap();
                let mut dests: Vec<&mut [f32]> =
                    planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
                this.read_planar(prime_start, prime_len as u64, &mut dests)?;
            }
            composition.write().unwrap().analysis_pass_stats.read_ns +=
                t0.elapsed().as_nanos() as u64;
            prime_planar = Some(planar);
        }

        // Transient pre-roll uses its own warmup when starting mid-timeline.
        let transient_warmup = if kinds_now.contains(&AnalysisKind::Transients)
            && read_pos == range_start
            && read_pos > 0
        {
            TransientDetectOp::warmup_frames(sample_rate).min(read_pos)
        } else {
            0
        };
        let mut transient_warm: Option<Vec<Vec<f32>>> = None;
        if transient_warmup > 0 {
            let mut planes = vec![vec![0.0; transient_warmup as usize]; channel_count];
            let t0 = Instant::now();
            {
                let this = composition.read().unwrap();
                let mut dests: Vec<&mut [f32]> =
                    planes.iter_mut().map(|ch| ch.as_mut_slice()).collect();
                this.read_planar(read_pos - transient_warmup, transient_warmup, &mut dests)?;
            }
            composition.write().unwrap().analysis_pass_stats.read_ns +=
                t0.elapsed().as_nanos() as u64;
            transient_warm = Some(planes);
        }

        let mut planar = vec![vec![0.0; take]; channel_count];
        {
            let t0 = Instant::now();
            let this = composition.read().unwrap();
            let mut dests: Vec<&mut [f32]> =
                planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
            this.read_planar(read_pos, take as u64, &mut dests)?;
            drop(this);
            composition.write().unwrap().analysis_pass_stats.read_ns +=
                t0.elapsed().as_nanos() as u64;
        }

        {
            let mut this = composition.write().unwrap();
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                this.abandon_analysis_job_scratch();
                return Ok(AnalysisBlockOutcome::Cancelled);
            }
            let t0 = Instant::now();
            let range_end = this
                .analysis_target_ranges
                .iter()
                .find(|(s, e)| read_pos >= *s && read_pos < *e)
                .map(|(_, e)| *e)
                .unwrap_or(read_pos + take as u64);

            if read_pos == range_start {
                if kinds_now.contains(&AnalysisKind::Spectral) {
                    this.spectral_op = Some(SpectralOp::new(sample_rate, channel_count));
                    if let Some(prime) = prime_planar.as_ref() {
                        let op = this.spectral_op.as_mut().expect("spectral");
                        for (ch, plane) in prime.iter().enumerate() {
                            op.prime_channel(ch, plane);
                        }
                    }
                }
                if kinds_now.contains(&AnalysisKind::EnvelopePeak) {
                    this.envelope_op = Some(EnvelopePeakOp::new(sample_rate, channel_count));
                    if let Some(prime) = prime_planar.as_ref() {
                        let op = this.envelope_op.as_mut().expect("envelope");
                        for (ch, plane) in prime.iter().enumerate() {
                            op.prime_channel(ch, plane);
                        }
                    }
                }
            }

            if kinds_now.contains(&AnalysisKind::MinMax) {
                this.consume_minmax_from_planar(read_pos, &planar)?;
            }
            if kinds_now.contains(&AnalysisKind::Spectral) {
                this.consume_spectral_from_planar(read_pos, range_end, &planar);
            }
            if kinds_now.contains(&AnalysisKind::EnvelopePeak) {
                this.consume_envelope_from_planar(read_pos, range_end, &planar);
            }
            if kinds_now.contains(&AnalysisKind::Transients) {
                this.consume_transient_from_planar(
                    read_pos,
                    range_start,
                    &planar,
                    transient_warm.as_deref(),
                );
            }

            this.analysis_pass_stats.consume_ns += t0.elapsed().as_nanos() as u64;
            {
                let pager = this.pager_arc().lock().unwrap().stats();
                this.analysis_pass_stats.decode_ns = pager.decode_ns;
                this.analysis_pass_stats.decodes = pager.decodes;
                this.analysis_pass_stats.ram_hits = pager.ram_hits;
                this.analysis_pass_stats.spill_hits = pager.spill_hits;
                this.analysis_pass_stats.spill_writes = pager.spill_writes;
            }

            let next_pos = advance_read_pos(&this.analysis_target_ranges, read_pos + take as u64);
            this.analysis_target_done += take as u64;
            this.analysis_read_pos = next_pos;
            let done_now = this.analysis_target_done;
            let complete = done_now >= this.analysis_target_total;
            if let Some(progress) = progress {
                progress.set_ratio(epoch, done_now, total.max(1));
            }
            if complete {
                drop(this);
                return Self::finish_pass(composition, progress, epoch);
            }
        }
        Ok(AnalysisBlockOutcome::Progress)
    }

    fn ensure_pass_started(composition: &RwLock<Self>, kinds: &[AnalysisKind]) -> Result<()> {
        let mut this = composition.write().unwrap();
        if this.analysis_job_started {
            return Ok(());
        }
        let sample_rate = this.sample_rate;
        let channel_count = this.channel_count;
        let frames = this.frames();
        let selection_configured = this.analysis_target_configured;

        let mut seeds: Vec<FrameRange> = Vec::new();
        let mut max_radius = 0u64;
        for &kind in kinds {
            let kind_seeds = this.seed_ranges_for_kind(kind, selection_configured);
            let radius = match kind.recompute_scope(sample_rate) {
                RecomputeScope::Regional(r) => r.radius_frames.max(r.warmup_frames),
                RecomputeScope::FullTimeline => 0,
            };
            max_radius = max_radius.max(radius);
            seeds.extend(kind_seeds);
        }
        let planned = if max_radius > 0 {
            expand_frame_ranges(&seeds, max_radius, frames)
        } else {
            merge_frame_ranges(seeds, frames)
        };

        for &kind in kinds {
            this.prepare_kind_streams(kind, &planned, selection_configured);
        }

        this.begin_analysis_target(Some(planned));
        this.analysis_pass_kinds = kinds.to_vec();
        this.analysis_job_started = true;
        this.analysis_minmax_clip = None;

        this.reset_pager_stats();
        this.analysis_pass_stats = AnalysisPassStats::default();

        if kinds.contains(&AnalysisKind::Transients) {
            this.ensure_transient_marker_type();
        }
        let _ = (sample_rate, channel_count);
        Ok(())
    }

    fn seed_ranges_for_kind(
        &self,
        kind: AnalysisKind,
        selection_configured: bool,
    ) -> Vec<FrameRange> {
        let frames = self.frames();
        match kind {
            AnalysisKind::MinMax => {
                let mut out = Vec::new();
                for span in self.tree.spans() {
                    if !span.clip.needs_peak_extend() {
                        continue;
                    }
                    let covered = peak_covered_samples(span.clip.as_ref()).min(span.clip.len);
                    let start = span.start.saturating_add(covered);
                    let end = span.start.saturating_add(span.clip.len);
                    if end > start {
                        out.push((start, end));
                    }
                }
                merge_frame_ranges(out, frames)
            }
            AnalysisKind::Spectral => {
                let hop = AnalysisStreams::spectral_hop();
                let hops_needed = if frames == 0 {
                    0
                } else {
                    ((frames as usize) + hop - 1) / hop
                };
                if let Some(series) = self.analysis_streams.spectral() {
                    let regional = !series.dirty_ranges.is_empty()
                        && series.hop_count(0) == hops_needed
                        && series.band_count == SPECTRAL_BAND_COUNT;
                    if regional {
                        return series.dirty_ranges.clone();
                    }
                }
                if frames == 0 {
                    Vec::new()
                } else {
                    vec![(0, frames)]
                }
            }
            AnalysisKind::EnvelopePeak => {
                let hop = AnalysisStreams::envelope_hop();
                let bins_needed = if frames == 0 {
                    0
                } else {
                    ((frames as usize) + hop - 1) / hop
                };
                if selection_configured {
                    return self.analysis_target_ranges.clone();
                }
                if let Some(series) = self.analysis_streams.float(AnalysisKind::EnvelopePeak) {
                    let regional = !series.dirty_ranges.is_empty()
                        && series.channels.iter().all(|ch| ch.len() == bins_needed);
                    if regional {
                        return series.dirty_ranges.clone();
                    }
                }
                if frames == 0 {
                    Vec::new()
                } else {
                    vec![(0, frames)]
                }
            }
            AnalysisKind::Transients => {
                if selection_configured {
                    self.analysis_target_ranges.clone()
                } else if frames == 0 {
                    Vec::new()
                } else {
                    vec![(0, frames)]
                }
            }
        }
    }

    fn prepare_kind_streams(
        &mut self,
        kind: AnalysisKind,
        planned: &[FrameRange],
        selection_configured: bool,
    ) {
        let frames = self.frames();
        let channel_count = self.channel_count;
        match kind {
            AnalysisKind::MinMax => {}
            AnalysisKind::Spectral => {
                let hop = AnalysisStreams::spectral_hop();
                let band_count = SPECTRAL_BAND_COUNT;
                let hops_needed = if frames == 0 {
                    0
                } else {
                    ((frames as usize) + hop - 1) / hop
                };
                let series = self.analysis_streams.ensure_spectral(channel_count);
                let regional = !series.dirty_ranges.is_empty()
                    && series.hop_count(0) == hops_needed
                    && series.band_count == band_count;
                if !regional {
                    for ch in &mut series.channels {
                        ch.clear();
                        ch.resize(
                            hops_needed * band_count,
                            field_audio_process::SPECTRAL_DB_FLOOR,
                        );
                    }
                    series.covered_frames = frames;
                    series.hop = hop;
                    series.band_count = band_count;
                    series.fft_size = field_audio_process::SPECTRAL_FFT_SIZE;
                    series.dirty_ranges = planned.to_vec();
                }
            }
            AnalysisKind::EnvelopePeak => {
                let hop = AnalysisStreams::envelope_hop();
                let bins_needed = if frames == 0 {
                    0
                } else {
                    ((frames as usize) + hop - 1) / hop
                };
                let series = self.analysis_streams.ensure_float(
                    AnalysisKind::EnvelopePeak,
                    channel_count,
                    hop,
                );
                let regional = !series.dirty_ranges.is_empty()
                    && series.channels.iter().all(|ch| ch.len() == bins_needed);
                if !regional {
                    for ch in &mut series.channels {
                        ch.clear();
                        ch.resize(bins_needed, 0.0);
                    }
                    series.covered_frames = frames;
                    series.dirty_ranges.clear();
                }
                if series.dirty_ranges.is_empty() {
                    series.dirty_ranges = if selection_configured {
                        planned.to_vec()
                    } else {
                        planned.to_vec()
                    };
                }
            }
            AnalysisKind::Transients => {}
        }
    }

    fn consume_minmax_from_planar(&mut self, read_pos: u64, planar: &[Vec<f32>]) -> Result<()> {
        let take = planar.first().map(|c| c.len()).unwrap_or(0) as u64;
        if take == 0 {
            return Ok(());
        }
        let channel_count = self.channel_count;
        let end = read_pos + take;
        let mut frame = read_pos;
        while frame < end {
            let Some(span) = self.tree.at(frame) else {
                break;
            };
            let clip = span.clip.clone();
            let span_end = span.start + clip.len;
            let chunk_end = end.min(span_end);
            if clip.source.is_none() || !clip.needs_peak_extend() {
                frame = chunk_end;
                continue;
            }
            let covered = peak_covered_samples(clip.as_ref()).min(clip.len);
            let local = frame.saturating_sub(span.start);
            if local < covered {
                frame = span.start.saturating_add(covered).min(chunk_end);
                if frame <= local + span.start {
                    frame = chunk_end;
                }
                continue;
            }
            if self.analysis_minmax_clip != Some(clip.id) {
                self.minmax_op = Some(MinMaxOp::new(channel_count));
                self.analysis_minmax_clip = Some(clip.id);
            }
            let n = (chunk_end - frame) as usize;
            let off = (frame - read_pos) as usize;
            let mut bins = vec![Vec::new(); channel_count];
            {
                let op = self
                    .minmax_op
                    .get_or_insert_with(|| MinMaxOp::new(channel_count));
                for (ch, plane) in planar.iter().enumerate() {
                    op.consume_channel(ch, &plane[off..off + n], &mut bins[ch]);
                }
                let clip_done = chunk_end >= span_end;
                if clip_done {
                    for (ch, out) in bins.iter_mut().enumerate() {
                        op.flush_channel(ch, out);
                    }
                }
                let (min, max) = op.combined_global_min_max();
                if !bins.iter().all(|b| b.is_empty()) || clip_done {
                    // Rebuild clip pointer after potential tree updates.
                    let target = clip.as_ref().clone();
                    self.append_peak_chunk(&target, bins, min, max);
                }
                if clip_done {
                    self.minmax_op = None;
                    self.analysis_minmax_clip = None;
                }
            }
            frame = chunk_end;
        }
        Ok(())
    }

    fn consume_spectral_from_planar(&mut self, read_pos: u64, range_end: u64, planar: &[Vec<f32>]) {
        let channel_count = self.channel_count;
        let sample_rate = self.sample_rate;
        let take = planar.first().map(|c| c.len()).unwrap_or(0);
        let hop = AnalysisStreams::spectral_hop();
        let band_count = SPECTRAL_BAND_COUNT;
        let mut hop_frames = vec![Vec::new(); channel_count];
        {
            let op = self
                .spectral_op
                .get_or_insert_with(|| SpectralOp::new(sample_rate, channel_count));
            for (ch, plane) in planar.iter().enumerate() {
                op.consume_channel(ch, plane, &mut hop_frames[ch]);
            }
            if read_pos + take as u64 >= range_end {
                for (ch, bins) in hop_frames.iter_mut().enumerate() {
                    op.flush_channel(ch, bins);
                }
            }
        }
        let frames = self.frames();
        let series = self.analysis_streams.ensure_spectral(channel_count);
        let base_hop = (read_pos as usize) / hop;
        for (ch, packed) in hop_frames.iter().enumerate() {
            let emitted = packed.len() / band_count;
            for i in 0..emitted {
                let dest_hop = base_hop + i;
                let dest_base = dest_hop * band_count;
                let src_base = i * band_count;
                if dest_base + band_count <= series.channels[ch].len() {
                    series.channels[ch][dest_base..dest_base + band_count]
                        .copy_from_slice(&packed[src_base..src_base + band_count]);
                }
            }
        }
        let written_end = read_pos + take as u64;
        series.clear_dirty_completed(read_pos, written_end.min(range_end));
        series.covered_frames = frames;
    }

    fn consume_envelope_from_planar(&mut self, read_pos: u64, range_end: u64, planar: &[Vec<f32>]) {
        let channel_count = self.channel_count;
        let sample_rate = self.sample_rate;
        let take = planar.first().map(|c| c.len()).unwrap_or(0);
        let hop = AnalysisStreams::envelope_hop();
        let mut hop_bins = vec![Vec::new(); channel_count];
        {
            let op = self
                .envelope_op
                .get_or_insert_with(|| EnvelopePeakOp::new(sample_rate, channel_count));
            for (ch, plane) in planar.iter().enumerate() {
                op.consume_channel(ch, plane, &mut hop_bins[ch]);
            }
            if read_pos + take as u64 >= range_end {
                for (ch, bins) in hop_bins.iter_mut().enumerate() {
                    op.flush_channel(ch, bins);
                }
            }
        }
        let frames = self.frames();
        let series =
            self.analysis_streams
                .ensure_float(AnalysisKind::EnvelopePeak, channel_count, hop);
        let base_hop = (read_pos as usize) / hop;
        for (ch, bins) in hop_bins.iter().enumerate() {
            for (i, &value) in bins.iter().enumerate() {
                let idx = base_hop + i;
                if idx < series.channels[ch].len() {
                    series.channels[ch][idx] = value;
                }
            }
        }
        let written_end = read_pos + take as u64;
        series.clear_dirty_completed(read_pos, written_end.min(range_end));
        series.covered_frames = frames;
    }

    fn consume_transient_from_planar(
        &mut self,
        read_pos: u64,
        range_start: u64,
        planar: &[Vec<f32>],
        warm: Option<&[Vec<f32>]>,
    ) {
        let channel_count = self.channel_count;
        let sample_rate = self.sample_rate;
        if read_pos == range_start {
            let mut op = TransientDetectOp::new(sample_rate, channel_count);
            if let Some(warm) = warm {
                let warm_start =
                    read_pos.saturating_sub(warm.first().map(|c| c.len() as u64).unwrap_or(0));
                op.begin(warm_start);
                op.consume_planar(warm);
            }
            op.begin(read_pos);
            self.transient_op = Some(op);
        }
        if let Some(op) = self.transient_op.as_mut() {
            op.consume_planar(planar);
        }
    }

    fn finish_pass(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        let mut this = composition.write().unwrap();
        let kinds = this.analysis_pass_kinds.clone();
        let frames = this.frames();

        if kinds.contains(&AnalysisKind::Spectral) {
            this.spectral_op = None;
            if let Some(series) = this.analysis_streams.spectral_mut() {
                series.covered_frames = frames;
            }
        }
        if kinds.contains(&AnalysisKind::EnvelopePeak) {
            this.envelope_op = None;
            if let Some(series) = this.analysis_streams.float_mut(AnalysisKind::EnvelopePeak) {
                series.covered_frames = frames;
            }
        }
        if kinds.contains(&AnalysisKind::Transients) {
            let mut sink = AnalysisSink::default();
            if let Some(mut op) = this.transient_op.take() {
                op.finish(&mut sink);
            }
            this.apply_transient_markers(sink.markers);
        }
        if kinds.contains(&AnalysisKind::MinMax) {
            this.minmax_op = None;
            this.analysis_minmax_clip = None;
        }

        {
            let pager = this.pager_arc().lock().unwrap().stats();
            this.analysis_pass_stats.decode_ns = pager.decode_ns;
            this.analysis_pass_stats.decodes = pager.decodes;
            this.analysis_pass_stats.ram_hits = pager.ram_hits;
            this.analysis_pass_stats.spill_hits = pager.spill_hits;
            this.analysis_pass_stats.spill_writes = pager.spill_writes;
        }

        this.analysis_job_started = false;
        this.analysis_target_configured = false;
        this.analysis_pass_kinds.clear();
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 1.0);
        }
        Ok(AnalysisBlockOutcome::Complete)
    }

    /// Replace the block pager (tests / specialized hosts).
    pub fn set_pager(&mut self, pager: field_audio_model::BlockPager) {
        *self.pager_arc().lock().unwrap() = pager;
    }

    /// Clear overview peaks and analysis streams so a pass must rebuild.
    pub fn clear_analysis_caches_for_test(&mut self) {
        self.tree = self.tree.map_clips(|clip| {
            let mut clip = clip.clone();
            clip.cache = crate::clip::ClipCache::default();
            clip
        });
        self.analysis_streams.clear();
        self.reset_analysis_job_scratch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field_audio_model::{BlockSource, MediaRef, BLOCK_FRAMES};
    use field_audio_process::{AnalysisKind, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, RwLock};

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

    fn run_kinds_to_complete(lock: &RwLock<Composition>, kinds: &[AnalysisKind]) {
        loop {
            match Composition::build_next_analysis_kinds(lock, kinds, None, 0).unwrap() {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
    }

    #[test]
    fn shared_minmax_spectral_matches_sequential() {
        let rate = 8_000u32;
        let frames = (BLOCK_FRAMES as usize) + 2_048;
        let media = sine_media(frames, 1, rate);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        lock.write().unwrap().clear_analysis_caches_for_test();

        run_kinds_to_complete(&lock, &[AnalysisKind::MinMax, AnalysisKind::Spectral]);
        let shared_peaks = lock
            .read()
            .unwrap()
            .clip_at(0)
            .unwrap()
            .clip
            .cache
            .peaks
            .clone();
        let shared_spectral = lock
            .read()
            .unwrap()
            .analysis_streams()
            .spectral()
            .expect("spectral")
            .channels[0]
            .clone();
        assert!(!shared_peaks.is_empty());
        assert!(!lock.read().unwrap().needs_peak_build());
        assert!(!lock.read().unwrap().needs_spectral_build());

        let media2 = sine_media(frames, 1, rate);
        let lock2 = RwLock::new(Composition::from_media(media2).unwrap());
        lock2.write().unwrap().clear_analysis_caches_for_test();
        run_kinds_to_complete(&lock2, &[AnalysisKind::MinMax]);
        run_kinds_to_complete(&lock2, &[AnalysisKind::Spectral]);
        let seq_peaks = lock2
            .read()
            .unwrap()
            .clip_at(0)
            .unwrap()
            .clip
            .cache
            .peaks
            .clone();
        let seq_spectral = lock2
            .read()
            .unwrap()
            .analysis_streams()
            .spectral()
            .expect("spectral")
            .channels[0]
            .clone();

        assert_eq!(shared_peaks, seq_peaks);
        assert_eq!(shared_spectral.len(), seq_spectral.len());
        for (a, b) in shared_spectral.iter().zip(seq_spectral.iter()) {
            assert!((a - b).abs() < 1e-3, "spectral mismatch {a} vs {b}");
        }
    }

    #[test]
    fn shared_pass_expands_to_max_radius_on_edit() {
        let rate = 8_000u32;
        let frames = 16_384usize;
        let media = sine_media(frames, 1, rate);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        run_kinds_to_complete(&lock, &[AnalysisKind::Spectral, AnalysisKind::EnvelopePeak]);
        {
            let mut comp = lock.write().unwrap();
            comp.delete(4_000, 128);
        }
        assert!(lock.read().unwrap().needs_spectral_build());
        assert!(lock.read().unwrap().needs_envelope_peak_build());
        run_kinds_to_complete(&lock, &[AnalysisKind::Spectral, AnalysisKind::EnvelopePeak]);
        assert!(!lock.read().unwrap().needs_spectral_build());
        assert!(!lock.read().unwrap().needs_envelope_peak_build());
        {
            let comp = lock.read().unwrap();
            let spectral = comp.analysis_streams().spectral().expect("spectral");
            assert!(spectral.dirty_ranges.is_empty());
            let mut columns = vec![0.0f32; 4 * SPECTRAL_BAND_COUNT];
            comp.fill_spectral_columns(0, 0.0, 256.0, &mut columns);
            assert!(columns.iter().any(|&v| v > SPECTRAL_DB_FLOOR));
        }
    }

    struct CountingSource {
        inner: field_audio_io::SymphoniaBlockSource,
        count: Arc<AtomicU64>,
    }

    impl BlockSource for CountingSource {
        fn decode_range(
            &self,
            path: &Path,
            start: u64,
            count: u64,
        ) -> anyhow::Result<Vec<Vec<f32>>> {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.inner.decode_range(path, start, count)
        }
    }

    fn write_sine_wav(path: &Path, channels: u16, frames: u32, sample_rate: u32) {
        use std::io::Write;
        let bits_per_sample: u16 = 16;
        let block_align = channels * bits_per_sample / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len = frames * u32::from(block_align);
        let mut out = std::fs::File::create(path).unwrap();
        out.write_all(b"RIFF").unwrap();
        out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        out.write_all(b"WAVE").unwrap();
        out.write_all(b"fmt ").unwrap();
        out.write_all(&16u32.to_le_bytes()).unwrap();
        out.write_all(&1u16.to_le_bytes()).unwrap();
        out.write_all(&channels.to_le_bytes()).unwrap();
        out.write_all(&sample_rate.to_le_bytes()).unwrap();
        out.write_all(&byte_rate.to_le_bytes()).unwrap();
        out.write_all(&block_align.to_le_bytes()).unwrap();
        out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        out.write_all(b"data").unwrap();
        out.write_all(&data_len.to_le_bytes()).unwrap();
        for i in 0..frames {
            let t = i as f32 / sample_rate as f32;
            let sample = (t * 440.0 * std::f32::consts::TAU).sin();
            let pcm = (sample * 0.6 * i16::MAX as f32) as i16;
            for _ in 0..channels {
                out.write_all(&pcm.to_le_bytes()).unwrap();
            }
        }
    }

    #[test]
    fn shared_pass_decodes_each_block_once() {
        let frames = BLOCK_FRAMES as u32 + 4_096;
        let path = std::env::temp_dir().join("fa-shared-pass-decode.wav");
        write_sine_wav(&path, 1, frames, 8_000);
        let mut comp = Composition::load_from_path(&path).unwrap();
        let count = Arc::new(AtomicU64::new(0));
        let spill = std::env::temp_dir().join("fa-shared-pass-spill");
        let _ = std::fs::remove_dir_all(&spill);
        let pager = field_audio_model::BlockPager::with_cache_bytes(
            spill.clone(),
            1, // force miss → decode every time a block is needed
            Arc::new(CountingSource {
                inner: field_audio_io::SymphoniaBlockSource,
                count: count.clone(),
            }),
        )
        .unwrap();
        comp.set_pager(pager);
        comp.clear_analysis_caches_for_test();
        let lock = RwLock::new(comp);
        count.store(0, Ordering::SeqCst);
        run_kinds_to_complete(&lock, &[AnalysisKind::MinMax, AnalysisKind::Spectral]);
        let shared_decodes = count.load(Ordering::SeqCst);
        assert!(
            shared_decodes >= 2,
            "expected multiple pager blocks, got {shared_decodes}"
        );

        // Sequential jobs with a cold tiny cache would decode twice as often.
        let mut comp2 = Composition::load_from_path(&path).unwrap();
        let count2 = Arc::new(AtomicU64::new(0));
        let spill2 = std::env::temp_dir().join("fa-shared-pass-spill2");
        let _ = std::fs::remove_dir_all(&spill2);
        comp2.set_pager(
            field_audio_model::BlockPager::with_cache_bytes(
                spill2,
                1,
                Arc::new(CountingSource {
                    inner: field_audio_io::SymphoniaBlockSource,
                    count: count2.clone(),
                }),
            )
            .unwrap(),
        );
        comp2.clear_analysis_caches_for_test();
        let lock2 = RwLock::new(comp2);
        count2.store(0, Ordering::SeqCst);
        run_kinds_to_complete(&lock2, &[AnalysisKind::MinMax]);
        let after_minmax = count2.load(Ordering::SeqCst);
        // Drop spill so Spectral cannot reuse decoded blocks from disk.
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("fa-shared-pass-spill2"));
        let _ = std::fs::create_dir_all(std::env::temp_dir().join("fa-shared-pass-spill2"));
        lock2.write().unwrap().reset_pager_stats();
        // Also flush RAM (tiny) by replacing pager with a fresh counting source.
        let count3 = Arc::new(AtomicU64::new(0));
        {
            let mut comp = lock2.write().unwrap();
            comp.set_pager(
                field_audio_model::BlockPager::with_cache_bytes(
                    std::env::temp_dir().join("fa-shared-pass-spill2b"),
                    1,
                    Arc::new(CountingSource {
                        inner: field_audio_io::SymphoniaBlockSource,
                        count: count3.clone(),
                    }),
                )
                .unwrap(),
            );
            // Keep peaks; only spectral still needed.
            comp.analysis_streams.clear();
            comp.reset_analysis_job_scratch();
        }
        run_kinds_to_complete(&lock2, &[AnalysisKind::Spectral]);
        let spectral_alone = count3.load(Ordering::SeqCst);
        let sequential_decodes = after_minmax + spectral_alone;
        assert_eq!(
            shared_decodes, after_minmax,
            "shared pass should decode about as often as MinMax alone over the same media"
        );
        assert!(
            spectral_alone >= 1,
            "spectral alone must decode at least once on a cold pager"
        );
        assert!(
            sequential_decodes > shared_decodes,
            "shared={shared_decodes} sequential={sequential_decodes} (minmax={after_minmax} spectral={spectral_alone})"
        );
        assert!(
            lock.read().unwrap().analysis_pass_stats().read_ns > 0
                || lock.read().unwrap().analysis_pass_stats().decode_ns > 0
                || shared_decodes > 0
        );
        let msg = lock
            .read()
            .unwrap()
            .analysis_pass_stats()
            .info_message(&[AnalysisKind::MinMax, AnalysisKind::Spectral]);
        assert!(msg.contains("decode"), "{msg}");
        assert!(msg.contains("consume"), "{msg}");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&spill);
    }
}
