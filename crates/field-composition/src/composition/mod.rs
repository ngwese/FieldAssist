// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{bail, Context, Result};

use field_audio_io::{probe_file, probe_header, ProbedFile, SymphoniaBlockSource};
use field_audio_model::{
    descriptor_mismatch, BlockPager, BlockSource, ChannelScope, DescriptorMismatch, Marker,
    MarkerId, MarkerList, MarkerType, MediaAvailability, MediaDescriptor, MediaId, MediaPool,
    MediaRef, MediaStore, RegionCollection, RegionId, StoredMarker, BLOCK_FRAMES,
    MARKER_TYPE_TRANSIENT, SELECTION_COLLECTION,
};
use field_audio_process::{
    AnalysisKind, AnalysisSink, EnvelopePeakOp, MinMaxOp, RecomputeScope, SpectralOp,
    TransientDetectOp, PEAK_BLOCK, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR,
};
use field_core::{LoadProblem, Location, OpenReport, ProblemCategory, ProgressHandle};

use super::analysis_store::AnalysisStreams;
use super::clip::{Clip, ClipCache, ClipId, ClipSpan};
use super::edit_ranges::{
    analysis_inverse_op, map_inclusive_through_inverse, map_inclusive_through_op,
};
use super::edl::{CompositionId, EditId, EditOp, Edl, InitialState, ProjectEnvelope, ProjectFile};
use super::tree::ClipTree;
use super::{map_point_if_kept, map_point_if_kept_inverse};

/// Result of folding one pager-sized analysis chunk into a live composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisBlockOutcome {
    /// Work progressed and more blocks remain.
    Progress,
    /// The analysis pass finished.
    Complete,
    /// The job epoch was cancelled before or during this block.
    Cancelled,
}

/// Historical name for [`AnalysisBlockOutcome`] (overview peaks).
pub type PeakBlockOutcome = AnalysisBlockOutcome;

#[derive(Debug, Clone, Default)]
/// Clipboard.
pub struct Clipboard {
    /// sample_rate.
    pub sample_rate: u32,
    /// channel_count.
    pub channel_count: usize,
    /// clips.
    pub clips: Vec<Clip>,
}

impl Clipboard {
    /// `is_empty`.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// `frames`.
    pub fn frames(&self) -> u64 {
        self.clips.iter().map(|clip| clip.len).sum()
    }
}

/// Composition.
pub struct Composition {
    id: CompositionId,
    parent: Option<CompositionId>,
    /// True when `id` was minted for a legacy file and must be saved.
    identity_dirty: bool,
    /// In-memory rename; cleared on save once the path basename matches.
    display_title: Option<String>,
    /// Stem of the `.facomp` path this composition was loaded from or last
    /// saved to. Used by [`Self::display_name`] when there is no rename
    /// override; not a dirtying field.
    path_display_name: Option<String>,
    sample_rate: u32,
    channel_count: usize,
    tree: ClipTree,
    store: Arc<Mutex<MediaStore>>,
    edl: Edl,
    next_clip_id: u64,
    clipboard: Clipboard,
    initial: InitialState,
    markers: MarkerList,
    marker_types: Vec<MarkerType>,
    collections: Vec<RegionCollection>,
    next_region_id: u64,
    channel_layout: Option<String>,
    chosen_channel_layout: Option<String>,
    channel_labels: BTreeMap<usize, String>,
    clean_edit_id: EditId,
    clean_markers: Vec<Marker>,
    clean_collections: Vec<RegionCollection>,
    monitor_chain: Option<String>,
    playback_channels: Option<Vec<usize>>,
    /// Dest composition channel → media channel. `None` means identity.
    source_channels: Option<Vec<usize>>,
    /// In-memory derived streams (envelope, …); not saved to `.facomp`.
    analysis_streams: AnalysisStreams,
    /// Scratch min/max fold kept across pager blocks of one clip.
    minmax_op: Option<MinMaxOp>,
    /// Scratch peak-envelope detector kept across pager blocks of one job.
    envelope_op: Option<EnvelopePeakOp>,
    /// Scratch STFT kept across pager blocks of one spectral job.
    spectral_op: Option<SpectralOp>,
    /// Scratch transient detector kept across pager blocks of one job.
    transient_op: Option<TransientDetectOp>,
    /// Half-open timeline ranges for the active envelope/transient job.
    /// Empty with [`Self::analysis_target_configured`] means no samples.
    analysis_target_ranges: Vec<(u64, u64)>,
    /// Absolute timeline frame of the next sample to process in the job.
    analysis_read_pos: u64,
    /// Total frames across [`Self::analysis_target_ranges`] (or full length).
    analysis_target_total: u64,
    /// Frames consumed so far in the active job.
    analysis_target_done: u64,
    /// True once `begin_analysis_target` has been applied for this job.
    analysis_job_started: bool,
    /// True after [`Self::begin_analysis_target`] until the job finishes/clears.
    analysis_target_configured: bool,
    /// Kinds participating in the active shared analysis pass.
    analysis_pass_kinds: Vec<AnalysisKind>,
    /// Clip currently feeding [`Self::minmax_op`] during a shared pass.
    analysis_minmax_clip: Option<ClipId>,
    /// Wall-time counters for the active / last shared pass.
    analysis_pass_stats: AnalysisPassStats,
}

mod analysis_pass;

pub use analysis_pass::AnalysisPassStats;

fn normalize_playback_channels(
    channels: Option<Vec<usize>>,
    channel_count: usize,
) -> Option<Vec<usize>> {
    let mut channels = channels?;
    channels.sort_unstable();
    channels.dedup();
    channels.retain(|index| *index < channel_count);
    if channels.is_empty() {
        return None;
    }
    if channels.len() == channel_count && channels.iter().copied().eq(0..channel_count) {
        return None;
    }
    Some(channels)
}

/// Normalize a dest→media channel map. Identity `0..channel_count` becomes
/// `None`. Length must equal `channel_count` when present.
fn normalize_source_channels(
    channels: Option<Vec<usize>>,
    channel_count: usize,
) -> Option<Vec<usize>> {
    let channels = channels?;
    if channels.len() != channel_count || channel_count == 0 {
        return None;
    }
    if channels.iter().copied().eq(0..channel_count) {
        return None;
    }
    Some(channels)
}

/// Named region collections that contribute to dirty state. The transient
/// `"selection"` collection is ignored, as are empty collections.
fn named_regions(collections: &[RegionCollection]) -> Vec<&RegionCollection> {
    collections
        .iter()
        .filter(|col| col.name != SELECTION_COLLECTION && !col.regions.is_empty())
        .collect()
}

fn private_media_store() -> Arc<Mutex<MediaStore>> {
    Arc::new(Mutex::new(MediaStore::in_memory()))
}

fn resolve_media_store(store: Option<Arc<Mutex<MediaStore>>>) -> Arc<Mutex<MediaStore>> {
    store.unwrap_or_else(private_media_store)
}

/// Ensure a user-facing rename becomes a `.facomp` file name.
pub fn normalize_facomp_file_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "untitled.facomp".into();
    }
    let path = Path::new(trimmed);
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("facomp"))
    {
        return trimmed.to_string();
    }
    format!("{trimmed}.facomp")
}

/// File-stem display label from a `.facomp` (or other) path.
fn path_stem_name(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

impl Composition {
    /// `new`.
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let tree = ClipTree::empty();
        let mut composition = Self {
            id: CompositionId::new(),
            parent: None,
            identity_dirty: false,
            display_title: None,
            path_display_name: None,
            sample_rate,
            channel_count,
            edl: Edl::new(tree.clone()),
            tree,
            store: private_media_store(),
            next_clip_id: 1,
            clipboard: Clipboard {
                sample_rate,
                channel_count,
                clips: Vec::new(),
            },
            initial: InitialState::Empty,
            markers: MarkerList::new(),
            marker_types: MarkerType::defaults(),
            collections: Vec::new(),
            next_region_id: 1,
            channel_layout: None,
            chosen_channel_layout: None,
            channel_labels: BTreeMap::new(),
            clean_edit_id: EditId(0),
            clean_markers: Vec::new(),
            clean_collections: Vec::new(),
            monitor_chain: None,
            playback_channels: None,
            source_channels: None,
            analysis_streams: AnalysisStreams::default(),
            minmax_op: None,
            envelope_op: None,
            spectral_op: None,
            transient_op: None,
            analysis_target_ranges: Vec::new(),
            analysis_read_pos: 0,
            analysis_target_total: 0,
            analysis_target_done: 0,
            analysis_job_started: false,
            analysis_target_configured: false,
            analysis_pass_kinds: Vec::new(),
            analysis_minmax_clip: None,
            analysis_pass_stats: AnalysisPassStats::default(),
        };
        composition.mark_clean();
        composition
    }

    /// `from_media`.
    pub fn from_media(media: MediaRef) -> Result<Self> {
        Self::from_media_with_store(media, None)
    }

    fn from_media_with_store(
        media: MediaRef,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<Self> {
        if media.sample_rate == 0 {
            bail!("media has no sample rate");
        }
        if media.channel_count == 0 {
            bail!("media has no channels");
        }
        let store = resolve_media_store(store);
        let sample_rate = media.sample_rate;
        let channel_count = media.channel_count;
        let frame_count = media.frame_count;
        let media_id = store.lock().unwrap().intern(media).0;
        let mut next_clip_id = 1;
        let clip = Clip::from_media(ClipId(next_clip_id), media_id, 0, frame_count);
        next_clip_id += 1;
        let tree = ClipTree::from_clip(clip);
        let mut composed = Self {
            id: CompositionId::new(),
            parent: None,
            identity_dirty: false,
            display_title: None,
            path_display_name: None,
            sample_rate,
            channel_count,
            edl: Edl::new(tree.clone()),
            tree,
            store,
            next_clip_id,
            clipboard: Clipboard {
                sample_rate,
                channel_count,
                clips: Vec::new(),
            },
            initial: InitialState::FromMedia { media_id },
            markers: MarkerList::new(),
            marker_types: MarkerType::defaults(),
            collections: Vec::new(),
            next_region_id: 1,
            channel_layout: None,
            chosen_channel_layout: None,
            channel_labels: BTreeMap::new(),
            clean_edit_id: EditId(0),
            clean_markers: Vec::new(),
            clean_collections: Vec::new(),
            monitor_chain: None,
            playback_channels: None,
            source_channels: None,
            analysis_streams: AnalysisStreams::default(),
            minmax_op: None,
            envelope_op: None,
            spectral_op: None,
            transient_op: None,
            analysis_target_ranges: Vec::new(),
            analysis_read_pos: 0,
            analysis_target_total: 0,
            analysis_target_done: 0,
            analysis_job_started: false,
            analysis_target_configured: false,
            analysis_pass_kinds: Vec::new(),
            analysis_minmax_clip: None,
            analysis_pass_stats: AnalysisPassStats::default(),
        };
        let peaked = composed
            .store
            .lock()
            .unwrap()
            .pool()
            .first()
            .is_some_and(|media| media.samples.is_some());
        if peaked {
            composed.ensure_clip_peaks().ok();
            composed.edl = Edl::new(composed.tree.clone());
        }
        composed.mark_clean();
        Ok(composed)
    }

    /// Open a media file and build a single-clip composition.
    pub fn from_media_path(path: &Path, store: Option<Arc<Mutex<MediaStore>>>) -> Result<Self> {
        let probed = probe_file(path)?;
        let mut media = media_ref_from_probed(probed);
        media.path = path.to_path_buf();
        media.prepare_url(None);
        Self::from_media_with_store(media, store)
    }

    /// Build a composition from a persisted descriptor, reprobing the source file.
    pub fn from_descriptor(
        descriptor: MediaDescriptor,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<(Self, OpenReport)> {
        Self::from_descriptor_with_base(descriptor, None, store)
    }

    /// Like [`Self::from_descriptor`], resolving relative descriptor URLs against `base`.
    pub fn from_descriptor_with_base(
        descriptor: MediaDescriptor,
        base: Option<&Path>,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<(Self, OpenReport)> {
        let target = descriptor.url.clone();
        let mut report = OpenReport::new(target.clone());
        let store = resolve_media_store(store);
        let mut media = MediaRef::from_descriptor(descriptor.clone(), PathBuf::new());
        if let Err(err) = media.resolve_url(base) {
            report.push(LoadProblem::new(
                ProblemCategory::InvalidLocation,
                descriptor.basename.clone(),
                err.to_string(),
                Some(descriptor.url.clone()),
            ));
            media.availability = MediaAvailability::Missing;
            let composed = Self::from_media_with_store(media, Some(store))?;
            return Ok((composed, report));
        }
        if !media.path.to_string_lossy().starts_with("memory://") {
            if !media.path.is_file() {
                media.availability = MediaAvailability::Missing;
                report.push(LoadProblem::new(
                    ProblemCategory::MissingReference,
                    descriptor.basename.clone(),
                    format!("missing source media {}", media.path.display()),
                    Some(descriptor.url.clone()),
                ));
                let composed = Self::from_media_with_store(media, Some(store))?;
                return Ok((composed, report));
            }
            let probed = match probe_file(&media.path) {
                Ok(probed) => probed,
                Err(err) => {
                    media.availability = MediaAvailability::Missing;
                    report.push(LoadProblem::new(
                        ProblemCategory::Other,
                        descriptor.basename.clone(),
                        format!("failed to probe {}: {err:#}", media.path.display()),
                        Some(descriptor.url.clone()),
                    ));
                    let composed = Self::from_media_with_store(media, Some(store))?;
                    return Ok((composed, report));
                }
            };
            let mut probed_ref = media_ref_from_probed(probed);
            probed_ref.path = media.path.clone();
            probed_ref.url = descriptor.url.clone();
            let mismatch = descriptor_mismatch(&descriptor, &probed_ref.to_descriptor());
            media = probed_ref;
            media.id = descriptor.id;
            media.url = descriptor.url.clone();
            media.basename = descriptor.basename.clone();
            if let Some(kind) = mismatch {
                media.availability = match kind {
                    DescriptorMismatch::Identity => MediaAvailability::IdentityMismatch,
                    DescriptorMismatch::Freshness => MediaAvailability::FreshnessMismatch,
                };
                report.push(LoadProblem::new(
                    ProblemCategory::MediaMismatch,
                    descriptor.basename.clone(),
                    match kind {
                        DescriptorMismatch::Identity => format!(
                            "Media identity changed for {} (keeping recorded media id)",
                            media.path.display()
                        ),
                        DescriptorMismatch::Freshness => format!(
                            "Media file modified on disk for {} (identity unchanged)",
                            media.path.display()
                        ),
                    },
                    Some(descriptor.url),
                ));
            }
            let composed = Self::from_media_with_store(media, Some(store))?;
            return Ok((composed, report));
        }
        media.id = descriptor.id;
        Ok((Self::from_media_with_store(media, Some(store))?, report))
    }

    /// Override composition identity (session reload of media documents).
    pub fn set_id(&mut self, id: CompositionId) {
        self.id = id;
    }

    /// `load_from_path`.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        Ok(Self::load_from_path_with_warnings(path)?.0)
    }

    /// `load_from_path_with_warnings`.
    pub fn load_from_path_with_warnings(path: &Path) -> Result<(Self, OpenReport)> {
        Self::load_from_path_with_progress(path, None, 0)
    }

    /// Load a `.facomp` or media file, optionally interning into a session store.
    pub fn load_from_path_into_store(
        path: &Path,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<(Self, OpenReport)> {
        Self::load_from_path_with_progress_into_store(path, None, 0, store)
    }

    /// `load_from_path_with_progress`.
    pub fn load_from_path_with_progress(
        path: &Path,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<(Self, OpenReport)> {
        Self::load_from_path_with_progress_into_store(path, progress, epoch, None)
    }

    /// Load with optional session media store and progress reporting.
    pub fn load_from_path_with_progress_into_store(
        path: &Path,
        progress: Option<&ProgressHandle>,
        epoch: u64,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<(Self, OpenReport)> {
        if is_facomp_path(path) {
            return Self::load_facomp_with_progress_into_store(path, progress, epoch, store);
        }
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.2);
        }
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.8);
        }
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        std::time::SystemTime::now().hash(&mut hasher);
        let spill = std::env::temp_dir()
            .join("FieldAssist")
            .join("blocks")
            .join(format!("{:x}", hasher.finish()));
        Ok((
            Self::from_media_path(path, store)?.with_spill_dir(spill)?,
            OpenReport::new(Location::from_path(path)),
        ))
    }

    /// `load_facomp`.
    pub fn load_facomp(path: &Path) -> Result<(Self, OpenReport)> {
        Self::load_facomp_with_progress(path, None, 0)
    }

    fn load_facomp_with_progress(
        path: &Path,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<(Self, OpenReport)> {
        Self::load_facomp_with_progress_into_store(path, progress, epoch, None)
    }

    fn load_facomp_with_progress_into_store(
        path: &Path,
        progress: Option<&ProgressHandle>,
        epoch: u64,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<(Self, OpenReport)> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.05);
        }
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.2);
        }
        let (mut composition, mut report) =
            Self::from_json_reprobing_at_into_store(&json, path.parent(), store)?;
        report.target = Location::from_path(path);
        composition.path_display_name = path_stem_name(path);
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.85);
        }
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        std::time::SystemTime::now().hash(&mut hasher);
        let spill = std::env::temp_dir()
            .join("FieldAssist")
            .join("blocks")
            .join(format!("{:x}", hasher.finish()));
        composition = composition.with_spill_dir(spill)?;
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 1.0);
        }
        Ok((composition, report))
    }

    /// `suggested_facomp_name`.
    pub fn suggested_facomp_name(&self) -> String {
        if let Some(title) = self.display_title.as_ref() {
            return normalize_facomp_file_name(title);
        }
        if let Some(name) = self.path_display_name.as_ref() {
            return normalize_facomp_file_name(name);
        }
        let stem = self
            .pool()
            .first()
            .and_then(|media| media.path.file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.media_display_name());
        format!("{stem}.facomp")
    }

    /// `save_to_path`.
    pub fn save_to_path(&mut self, path: &Path) -> Result<()> {
        write_atomic(path, &self.to_json_with_base(path.parent())?)?;
        self.path_display_name = path_stem_name(path);
        self.mark_clean();
        Ok(())
    }

    /// `is_modified`.
    pub fn is_modified(&self) -> bool {
        self.identity_dirty
            || self.display_title.is_some()
            || self.edl.current_id() != self.clean_edit_id
            || self.markers.to_vec() != self.clean_markers
            || named_regions(&self.collections) != named_regions(&self.clean_collections)
    }

    fn mark_clean(&mut self) {
        self.clean_edit_id = self.edl.current_id();
        self.clean_markers = self.markers.to_vec();
        self.clean_collections = self.collections.clone();
        self.identity_dirty = false;
        self.display_title = None;
    }

    /// `with_spill_dir`.
    pub fn with_spill_dir(self, dir: impl AsRef<Path>) -> Result<Self> {
        let source: Arc<dyn BlockSource> = Arc::new(SymphoniaBlockSource);
        self.store.lock().unwrap().with_spill_dir(dir, source)?;
        Ok(self)
    }

    /// Shared session media store (pool + pager).
    pub fn media_store(&self) -> Arc<Mutex<MediaStore>> {
        Arc::clone(&self.store)
    }

    /// Block pager shared with other compositions using the same store.
    pub fn pager_arc(&self) -> Arc<Mutex<BlockPager>> {
        self.store.lock().unwrap().pager_arc()
    }

    /// Stable composition identity.
    pub fn id(&self) -> CompositionId {
        self.id
    }

    /// Parent composition identity when this was broken out from another.
    pub fn parent_id(&self) -> Option<CompositionId> {
        self.parent
    }

    /// Mint a fresh identity while keeping the parent link (Save As).
    pub fn mint_new_id(&mut self) {
        self.id = CompositionId::new();
        self.identity_dirty = true;
    }

    /// Share the parent's decode cache so break-out children do not re-decode.
    pub fn adopt_shared_media(&mut self, parent: &Composition) {
        self.store = Arc::clone(&parent.store);
    }

    /// Whether this composition shares its block pager with `other`.
    pub fn shares_pager_with(&self, other: &Composition) -> bool {
        Arc::ptr_eq(&self.pager_arc(), &other.pager_arc())
    }

    /// Extract `[start, start+len)` into a new child composition that shares
    /// media and the block pager with this composition.
    pub fn break_out(&self, start: u64, len: u64) -> Result<Composition> {
        self.break_out_channels(&[(start, len)], None)
    }

    /// Extract a child composition with an optional time trim and/or channel
    /// subset. Empty `ranges` keeps the full timeline. `channels` are indices
    /// into this composition; `None` or all channels keeps the full topology.
    pub fn break_out_channels(
        &self,
        ranges: &[(u64, u64)],
        channels: Option<&[usize]>,
    ) -> Result<Composition> {
        let ranges: Vec<(u64, u64)> = ranges.iter().copied().filter(|(_, len)| *len > 0).collect();
        for &(start, len) in &ranges {
            if start.saturating_add(len) > self.frames() {
                bail!("break out range is outside the composition");
            }
        }
        let mut selected: Option<Vec<usize>> = channels.map(|chs| {
            let mut v: Vec<usize> = chs
                .iter()
                .copied()
                .filter(|&c| c < self.channel_count)
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        });
        if let Some(chs) = &selected {
            if chs.is_empty() {
                bail!("break out channel set is empty");
            }
            if chs.len() == self.channel_count && chs.iter().copied().eq(0..self.channel_count) {
                selected = None;
            }
        }
        if ranges.is_empty() && selected.is_none() {
            bail!("break out requires a time range or channel subset");
        }
        let mut child = self.clone_at_cursor();
        child.id = CompositionId::new();
        child.parent = Some(self.id);
        child.identity_dirty = false;
        if !ranges.is_empty() {
            if ranges.len() == 1 {
                let (start, len) = ranges[0];
                child.trim(start, len);
            } else {
                child.trim_ranges(&ranges);
            }
        }
        if let Some(chs) = selected {
            child.take_channels(&chs);
        }
        child.edl.set_undo_floor(child.edl.cursor());
        // Leave dirty so the first save writes the standalone child.
        child.clean_edit_id = EditId(u64::MAX);
        child.clean_markers.clear();
        child.clean_collections.clear();
        Ok(child)
    }

    /// Restrict this composition to a subset of its current channels.
    ///
    /// `selected` are indices into the current topology. Composes with any
    /// existing [`Self::source_channels`] map. Clears chosen layout so
    /// `detect_layout` can assign one that matches the new channel count.
    pub fn take_channels(&mut self, selected: &[usize]) {
        let mut selected: Vec<usize> = selected
            .iter()
            .copied()
            .filter(|&c| c < self.channel_count)
            .collect();
        selected.sort_unstable();
        selected.dedup();
        if selected.is_empty() {
            return;
        }
        if selected.len() == self.channel_count
            && selected.iter().copied().eq(0..self.channel_count)
        {
            return;
        }
        let new_map: Vec<usize> = selected
            .iter()
            .map(|&parent_ch| self.media_channel(parent_ch))
            .collect();
        let mut new_labels = BTreeMap::new();
        for (new_i, &old_i) in selected.iter().enumerate() {
            if let Some(label) = self.channel_labels.get(&old_i) {
                new_labels.insert(new_i, label.clone());
            }
        }
        let new_playback = self.playback_channels.as_ref().map(|chs| {
            chs.iter()
                .filter_map(|old| selected.iter().position(|s| s == old))
                .collect::<Vec<_>>()
        });
        for collection in &mut self.collections {
            collection
                .regions
                .retain_mut(|region| region.channels.remap_take(&selected));
        }
        self.tree = self.tree.map_clips(|clip| {
            let mut clip = clip.clone();
            clip.cache = ClipCache::default();
            clip
        });
        self.edl.map_snapshots(|tree| {
            tree.map_clips(|clip| {
                let mut clip = clip.clone();
                clip.cache = ClipCache::default();
                clip
            })
        });
        self.channel_count = selected.len();
        self.source_channels = normalize_source_channels(Some(new_map), self.channel_count);
        self.channel_labels = new_labels;
        self.clipboard.channel_count = self.channel_count;
        self.playback_channels = normalize_playback_channels(new_playback, self.channel_count);
        // Channel topology changed — let detect_layout pick a matching layout.
        self.chosen_channel_layout = None;
        self.channel_layout = None;
        self.analysis_streams = AnalysisStreams::default();
        self.minmax_op = None;
        self.envelope_op = None;
        self.spectral_op = None;
        self.transient_op = None;
    }

    /// Media channel index for a destination composition channel.
    pub fn media_channel(&self, dest: usize) -> usize {
        self.source_channels
            .as_ref()
            .and_then(|map| map.get(dest).copied())
            .unwrap_or(dest)
    }

    /// Dest→media channel map, when not identity.
    pub fn source_channels(&self) -> Option<&[usize]> {
        self.source_channels.as_deref()
    }

    /// Clone reconstruction state at the current cursor, sharing the pager.
    fn clone_at_cursor(&self) -> Composition {
        let mut edl = self.edl.clone();
        edl.truncate_to_cursor();
        Composition {
            id: self.id,
            parent: self.parent,
            identity_dirty: false,
            display_title: None,
            path_display_name: None,
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            tree: self.tree.clone(),
            store: Arc::clone(&self.store),
            edl,
            next_clip_id: self.next_clip_id,
            clipboard: Clipboard {
                sample_rate: self.sample_rate,
                channel_count: self.channel_count,
                clips: Vec::new(),
            },
            initial: self.initial.clone(),
            markers: self.markers.clone(),
            marker_types: self.marker_types.clone(),
            collections: self.collections.clone(),
            next_region_id: self.next_region_id,
            channel_layout: self.channel_layout.clone(),
            chosen_channel_layout: self.chosen_channel_layout.clone(),
            channel_labels: self.channel_labels.clone(),
            clean_edit_id: EditId(0),
            clean_markers: Vec::new(),
            clean_collections: Vec::new(),
            monitor_chain: self.monitor_chain.clone(),
            playback_channels: self.playback_channels.clone(),
            source_channels: self.source_channels.clone(),
            analysis_streams: AnalysisStreams::default(),
            minmax_op: None,
            envelope_op: None,
            spectral_op: None,
            transient_op: None,
            analysis_target_ranges: Vec::new(),
            analysis_read_pos: 0,
            analysis_target_total: 0,
            analysis_target_done: 0,
            analysis_job_started: false,
            analysis_target_configured: false,
            analysis_pass_kinds: Vec::new(),
            analysis_minmax_clip: None,
            analysis_pass_stats: AnalysisPassStats::default(),
        }
    }

    /// In-memory display title override when the composition was renamed.
    pub fn display_title(&self) -> Option<&str> {
        self.display_title.as_deref()
    }

    /// Set an in-memory display title. Empty clears the override. Dirties the
    /// composition until the next successful save.
    pub fn set_display_title(&mut self, name: impl AsRef<str>) {
        let trimmed = name.as_ref().trim();
        let next = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        if self.display_title != next {
            self.display_title = next;
        }
    }

    /// `display_name`.
    pub fn display_name(&self) -> String {
        if let Some(title) = self.display_title.as_ref() {
            return title.clone();
        }
        if let Some(name) = self.path_display_name.as_ref() {
            return name.clone();
        }
        self.media_display_name()
    }

    fn media_display_name(&self) -> String {
        self.primary_media()
            .and_then(|media| {
                media
                    .path
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        (!media.basename.is_empty()).then(|| {
                            Path::new(&media.basename)
                                .file_stem()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| media.basename.clone())
                        })
                    })
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "FieldAssist".into())
    }

    /// Media this composition was opened from, or the first used media entry.
    fn primary_media(&self) -> Option<MediaRef> {
        let store = self.store.lock().unwrap();
        let pool = store.pool();
        if let InitialState::FromMedia { media_id } = self.initial {
            if let Some(media) = pool.get(media_id) {
                return Some(media.clone());
            }
        }
        drop(store);
        self.used_media_refs().into_iter().next()
    }

    /// True when the EDL cursor is past the founding Init edit (user edits applied).
    pub fn has_edits(&self) -> bool {
        self.edl.cursor() > 0
    }

    /// `sample_rate`.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// `channel_count`.
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    /// `frames`.
    pub fn frames(&self) -> u64 {
        self.tree.frames()
    }

    /// `duration_secs`.
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }

    /// `is_empty`.
    pub fn is_empty(&self) -> bool {
        self.tree.frames() == 0
    }

    /// `markers`.
    pub fn markers(&self) -> &MarkerList {
        &self.markers
    }

    /// `add_marker`.
    pub fn add_marker(
        &mut self,
        frame: u64,
        marker_type: impl Into<String>,
        note: Option<String>,
    ) -> Option<MarkerId> {
        self.markers.insert(frame, marker_type, note)
    }

    /// `remove_marker`.
    pub fn remove_marker(&mut self, id: MarkerId) -> bool {
        self.markers.remove(id)
    }

    /// `remove_marker_at`.
    pub fn remove_marker_at(&mut self, frame: u64) -> bool {
        self.markers.remove_at(frame)
    }

    /// `remove_marker_at_type`.
    pub fn remove_marker_at_type(&mut self, frame: u64, marker_type: &str) -> bool {
        self.markers.remove_at_type(frame, marker_type)
    }

    /// Remove every marker whose type name matches `marker_type`.
    ///
    /// Returns how many markers were removed. The type registry entry is kept.
    pub fn remove_marker_by_type(&mut self, marker_type: &str) -> usize {
        self.markers.remove_type(marker_type)
    }

    /// `marker_types`.
    pub fn marker_types(&self) -> &[MarkerType] {
        &self.marker_types
    }

    /// `marker_type_color`.
    pub fn marker_type_color(&self, name: &str) -> Option<[f32; 4]> {
        MarkerType::color_of(&self.marker_types, name)
    }

    /// `resolved_marker_color`.
    pub fn resolved_marker_color(&self, name: &str) -> [f32; 4] {
        MarkerType::resolved_color(&self.marker_types, name)
    }

    /// `add_marker_type`.
    pub fn add_marker_type(&mut self, name: impl Into<String>, color: [f32; 4]) -> bool {
        let name = name.into();
        if name.is_empty() {
            return false;
        }
        if self.marker_types.iter().any(|ty| ty.name == name) {
            return false;
        }
        self.marker_types.push(MarkerType { name, color });
        true
    }

    /// `remove_marker_type`.
    pub fn remove_marker_type(&mut self, name: &str) -> bool {
        let Some(index) = self.marker_types.iter().position(|ty| ty.name == name) else {
            return false;
        };
        self.marker_types.remove(index);
        self.markers.remove_type(name);
        true
    }

    /// `collections`.
    pub fn collections(&self) -> &[RegionCollection] {
        &self.collections
    }

    /// `collection`.
    pub fn collection(&self, name: &str) -> Option<&RegionCollection> {
        self.collections.iter().find(|col| col.name == name)
    }

    /// `collection_mut`.
    pub fn collection_mut(&mut self, name: &str) -> Option<&mut RegionCollection> {
        self.collections.iter_mut().find(|col| col.name == name)
    }

    /// `ensure_collection`.
    pub fn ensure_collection(&mut self, name: &str) -> Option<&mut RegionCollection> {
        if name.is_empty() || name == SELECTION_COLLECTION {
            return None;
        }
        if !self.collections.iter().any(|col| col.name == name) {
            self.collections.push(RegionCollection::new(name));
        }
        self.collection_mut(name)
    }

    /// `add_named_region`.
    pub fn add_named_region(
        &mut self,
        name: &str,
        start: usize,
        end: usize,
        channels: ChannelScope,
        label: Option<String>,
    ) -> Option<RegionId> {
        if name.is_empty() || name == SELECTION_COLLECTION {
            return None;
        }
        let index = match self.collections.iter().position(|col| col.name == name) {
            Some(index) => index,
            None => {
                self.collections.push(RegionCollection::new(name));
                self.collections.len() - 1
            }
        };
        Some(self.collections[index].alloc_push(
            start,
            end,
            channels,
            label,
            &mut self.next_region_id,
        ))
    }

    /// `alloc_region_id`.
    pub fn alloc_region_id(&mut self) -> u64 {
        let id = self.next_region_id;
        self.next_region_id += 1;
        id
    }

    /// `peek_next_region_id`.
    pub fn peek_next_region_id(&self) -> u64 {
        self.next_region_id.max(1)
    }

    /// `set_next_region_id`.
    pub fn set_next_region_id(&mut self, next: u64) {
        self.next_region_id = self.next_region_id.max(next).max(1);
    }

    fn bump_next_region_id_from_collections(&mut self) {
        let max_id = self
            .collections
            .iter()
            .flat_map(|col| col.regions.iter())
            .map(|region| region.id.0)
            .max()
            .unwrap_or(0);
        self.next_region_id = self.next_region_id.max(max_id.saturating_add(1)).max(1);
    }

    fn remap_collections_op(&mut self, op: &EditOp) {
        for collection in &mut self.collections {
            collection.remap(|start, end| map_inclusive_through_op(start, end, op));
        }
    }

    fn remap_collections_inverse(&mut self, op: &EditOp) {
        for collection in &mut self.collections {
            collection.remap(|start, end| map_inclusive_through_inverse(start, end, op));
        }
    }

    /// Media used by this composition (not the full shared session store).
    pub fn pool(&self) -> MediaPool {
        let mut pool = MediaPool::new();
        for media in self.used_media_refs() {
            pool.insert(media);
        }
        pool
    }

    /// `channel_layout`.
    pub fn channel_layout(&self) -> Option<&str> {
        self.channel_layout.as_deref()
    }

    /// `chosen_channel_layout`.
    pub fn chosen_channel_layout(&self) -> Option<&str> {
        self.chosen_channel_layout.as_deref()
    }

    /// `channel_label`.
    pub fn channel_label(&self, channel: usize) -> String {
        self.channel_labels
            .get(&channel)
            .cloned()
            .unwrap_or_else(|| format!("Ch {}", channel + 1))
    }

    /// `apply_channel_layout`.
    pub fn apply_channel_layout(&mut self, name: Option<String>, labels: BTreeMap<usize, String>) {
        self.channel_layout = name;
        self.channel_labels = labels;
    }

    /// `choose_channel_layout`.
    pub fn choose_channel_layout(&mut self, name: Option<String>, labels: BTreeMap<usize, String>) {
        self.chosen_channel_layout = name.clone();
        self.apply_channel_layout(name, labels);
    }

    /// `monitor_chain`.
    pub fn monitor_chain(&self) -> Option<&str> {
        self.monitor_chain.as_deref()
    }

    /// `set_monitor_chain`.
    pub fn set_monitor_chain(&mut self, chain: Option<String>) {
        self.monitor_chain = chain.filter(|name| !name.is_empty());
    }

    /// `playback_channels`.
    pub fn playback_channels(&self) -> Option<&[usize]> {
        self.playback_channels.as_deref()
    }

    /// `set_playback_channels`.
    pub fn set_playback_channels(&mut self, channels: Option<Vec<usize>>) {
        self.playback_channels = normalize_playback_channels(channels, self.channel_count);
    }

    /// `codec`.
    pub fn codec(&self) -> Option<String> {
        self.primary_media().map(|media| media.codec)
    }

    /// `bit_depth`.
    pub fn bit_depth(&self) -> Option<u32> {
        self.primary_media().and_then(|media| media.bits_per_sample)
    }

    /// `clipboard`.
    pub fn clipboard(&self) -> &Clipboard {
        &self.clipboard
    }

    /// `current_edit`.
    pub fn current_edit(&self) -> EditId {
        self.edl.current_id()
    }

    /// `edit_cursor`.
    pub fn edit_cursor(&self) -> usize {
        self.edl.cursor()
    }

    /// `can_undo`.
    pub fn can_undo(&self) -> bool {
        self.edl.can_undo()
    }

    /// `can_redo`.
    pub fn can_redo(&self) -> bool {
        self.edl.can_redo()
    }

    /// `edits`.
    pub fn edits(&self) -> &[super::edl::Edit] {
        self.edl.edits()
    }

    /// `spans`.
    pub fn spans(&self) -> Vec<ClipSpan> {
        self.tree.spans()
    }

    /// `clip_at`.
    pub fn clip_at(&self, frame: u64) -> Option<ClipSpan> {
        self.tree.at(frame)
    }

    /// Number of clips in the timeline tree.
    pub fn clip_count(&self) -> usize {
        self.tree.clip_count()
    }

    /// Snapshot of block-pager cache / decode counters.
    pub fn pager_stats(&self) -> field_audio_model::PagerStats {
        self.pager_arc().lock().unwrap().stats()
    }

    /// Clear pager counters without flushing the cache.
    pub fn reset_pager_stats(&self) {
        self.pager_arc().lock().unwrap().reset_stats();
    }

    fn alloc_clip_id(&mut self) -> ClipId {
        let id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    fn remap_clips(&mut self, clips: &[Clip]) -> Vec<Clip> {
        clips
            .iter()
            .map(|clip| {
                let mut clip = clip.clone();
                clip.id = self.alloc_clip_id();
                clip
            })
            .collect()
    }

    fn commit(&mut self, op: EditOp, tree: ClipTree) {
        self.markers.remap(|frame| map_point_if_kept(frame, &op));
        self.remap_collections_op(&op);
        let post_frames = tree.frames();
        let pre_tree = self.tree.clone();
        self.invalidate_analysis_streams(&op, &pre_tree, post_frames);
        self.tree = tree;
        self.edl.push(op, self.tree.clone());
        self.reset_analysis_job_scratch();
    }

    /// `undo`.
    pub fn undo(&mut self) -> bool {
        if !self.edl.can_undo() {
            return false;
        }
        let op = self.edl.current().op.clone();
        if let Some(tree) = self.edl.undo() {
            self.markers
                .remap(|frame| map_point_if_kept_inverse(frame, &op));
            self.remap_collections_inverse(&op);
            let post_frames = tree.frames();
            let pre_tree = self.tree.clone();
            match analysis_inverse_op(&op) {
                Some(inv) => self.invalidate_analysis_streams(&inv, &pre_tree, post_frames),
                None => {
                    self.analysis_streams.clear();
                }
            }
            self.adopt_tree(tree);
            self.reset_analysis_job_scratch();
            true
        } else {
            false
        }
    }

    /// `redo`.
    pub fn redo(&mut self) -> bool {
        if !self.edl.can_redo() {
            return false;
        }
        let op = self.edl.edits()[self.edl.cursor() + 1].op.clone();
        if let Some(tree) = self.edl.redo() {
            self.markers.remap(|frame| map_point_if_kept(frame, &op));
            self.remap_collections_op(&op);
            let post_frames = tree.frames();
            let pre_tree = self.tree.clone();
            self.invalidate_analysis_streams(&op, &pre_tree, post_frames);
            self.adopt_tree(tree);
            self.reset_analysis_job_scratch();
            true
        } else {
            false
        }
    }

    /// `jump_to_edit`.
    pub fn jump_to_edit(&mut self, id: EditId) -> bool {
        let from = self.edl.cursor();
        if let Some(tree) = self.edl.jump_to(id) {
            let to = self.edl.cursor();
            self.remap_markers_between(from, to);
            self.remap_collections_between(from, to);
            self.invalidate_analysis_streams_between(from, to, tree.frames());
            self.adopt_tree(tree);
            self.reset_analysis_job_scratch();
            true
        } else {
            false
        }
    }

    /// Splice regional analysis streams through `op`; drop job scratch.
    fn invalidate_analysis_streams(&mut self, op: &EditOp, pre_tree: &ClipTree, post_frames: u64) {
        let sample_rate = self.sample_rate;
        self.analysis_streams
            .invalidate_through_op(op, pre_tree, post_frames, sample_rate);
    }

    /// Remap streams across a multi-edit jump by splicing each step.
    ///
    /// Uses EDL snapshots for intermediate tree lengths / Roll landing. Clears
    /// when an inverse cannot be expressed (e.g. undoing Trim) or the hop
    /// buffer length no longer matches the destination timeline.
    fn invalidate_analysis_streams_between(&mut self, from: usize, to: usize, post_frames: u64) {
        if from == to {
            return;
        }
        let sample_rate = self.sample_rate;
        let hop = AnalysisStreams::spectral_hop();
        let edits: Vec<(EditOp, ClipTree, u64)> = self
            .edl
            .edits()
            .iter()
            .map(|edit| {
                (
                    edit.op.clone(),
                    edit.snapshot.clone(),
                    edit.snapshot.frames(),
                )
            })
            .collect();

        if to > from {
            for i in (from + 1)..=to {
                let (ref op, _, step_post) = edits[i];
                let pre_tree = &edits[i - 1].1;
                self.analysis_streams
                    .invalidate_through_op(op, pre_tree, step_post, sample_rate);
            }
        } else {
            for i in (to + 1..=from).rev() {
                let (ref op, ref post_forward_tree, _) = edits[i];
                let step_post = edits[i - 1].2;
                match analysis_inverse_op(op) {
                    Some(inv) => self.analysis_streams.invalidate_through_op(
                        &inv,
                        post_forward_tree,
                        step_post,
                        sample_rate,
                    ),
                    None => {
                        self.analysis_streams.clear();
                        return;
                    }
                }
            }
        }

        let expected_hops = if post_frames == 0 || hop == 0 {
            0
        } else {
            ((post_frames as usize) + hop - 1) / hop
        };
        if let Some(series) = self.analysis_streams.spectral() {
            if series.has_data() && series.hop_count(0) != expected_hops {
                self.analysis_streams.clear();
                return;
            }
        }
        if let Some(series) = self.analysis_streams.float(AnalysisKind::EnvelopePeak) {
            let bins = series.channels.first().map(|ch| ch.len()).unwrap_or(0);
            if series.has_data() && bins != expected_hops {
                self.analysis_streams.clear();
            }
        }
    }

    fn reset_analysis_job_scratch(&mut self) {
        self.minmax_op = None;
        self.envelope_op = None;
        self.spectral_op = None;
        self.transient_op = None;
        self.analysis_target_ranges.clear();
        self.analysis_read_pos = 0;
        self.analysis_target_total = 0;
        self.analysis_target_done = 0;
        self.analysis_job_started = false;
        self.analysis_target_configured = false;
        self.analysis_pass_kinds.clear();
        self.analysis_minmax_clip = None;
    }

    /// Configure the next envelope/transient job.
    ///
    /// `ranges` are half-open `[start, end)` spans. `None` means the whole
    /// timeline. `Some([])` means no samples (Selection Only with an empty
    /// selection).
    pub fn begin_analysis_target(&mut self, ranges: Option<Vec<(u64, u64)>>) {
        let frames = self.frames();
        let ranges = match ranges {
            None => vec![(0, frames)],
            Some(r) if r.is_empty() => Vec::new(),
            Some(r) => normalize_half_open_ranges(r, frames),
        };
        let total: u64 = ranges.iter().map(|(s, e)| e.saturating_sub(*s)).sum();
        self.analysis_target_ranges = ranges;
        self.analysis_read_pos = self
            .analysis_target_ranges
            .first()
            .map(|(s, _)| *s)
            .unwrap_or(0);
        self.analysis_target_total = total;
        self.analysis_target_done = 0;
        self.analysis_job_started = false;
        self.analysis_target_configured = true;
        self.minmax_op = None;
        self.envelope_op = None;
        self.spectral_op = None;
        self.transient_op = None;
        self.analysis_pass_kinds.clear();
        self.analysis_minmax_clip = None;
    }

    fn remap_markers_between(&mut self, from: usize, to: usize) {
        let ops: Vec<EditOp> = self
            .edl
            .edits()
            .iter()
            .map(|edit| edit.op.clone())
            .collect();
        if to > from {
            for op in &ops[from + 1..=to] {
                self.markers.remap(|frame| map_point_if_kept(frame, op));
            }
        } else if to < from {
            for op in ops[to + 1..=from].iter().rev() {
                self.markers
                    .remap(|frame| map_point_if_kept_inverse(frame, op));
            }
        }
    }

    fn remap_collections_between(&mut self, from: usize, to: usize) {
        let ops: Vec<EditOp> = self
            .edl
            .edits()
            .iter()
            .map(|edit| edit.op.clone())
            .collect();
        if to > from {
            for op in &ops[from + 1..=to] {
                self.remap_collections_op(op);
            }
        } else if to < from {
            for op in ops[to + 1..=from].iter().rev() {
                self.remap_collections_inverse(op);
            }
        }
    }

    fn adopt_tree(&mut self, tree: ClipTree) {
        let tree = self.tree_with_initial_media(tree);
        self.tree = tree;
    }

    fn rebuild_from_initial_media(&self) -> Option<ClipTree> {
        let InitialState::FromMedia { media_id } = self.initial else {
            return None;
        };
        let frame_count = self
            .store
            .lock()
            .unwrap()
            .pool()
            .get(media_id)
            .map(|media| media.frame_count)?;
        if frame_count == 0 {
            return None;
        }
        Some(ClipTree::from_clip(Clip::from_media(
            ClipId(1),
            media_id,
            0,
            frame_count,
        )))
    }

    fn tree_with_initial_media(&mut self, tree: ClipTree) -> ClipTree {
        if !tree.is_empty() || self.edl.cursor() != 0 {
            return tree;
        }
        let Some(rebuilt) = self.rebuild_from_initial_media() else {
            return tree;
        };
        self.edl.replace_init_snapshot(rebuilt.clone());
        rebuilt
    }

    /// Regions changed by applied edits above the undo floor, in the current
    /// timeline. Founding break-out history does not contribute change bars.
    ///
    /// Adjacent landings from different edits stay separate so the waveform
    /// can draw a gap between them.
    pub fn modified_ranges(&self) -> Vec<(u64, u64)> {
        super::edit_ranges::modified_ranges(
            self.edl.edits(),
            self.edl.cursor(),
            self.edl.undo_floor(),
        )
    }

    /// Where `id` landed on the current timeline, if that edit is applied and
    /// above the undo floor.
    pub fn ranges_for_edit(&self, id: EditId) -> Vec<(u64, u64)> {
        let edits = self.edl.edits();
        let Some(index) = edits.iter().position(|edit| edit.id == id) else {
            return Vec::new();
        };
        if index <= self.edl.undo_floor() {
            return Vec::new();
        }
        super::edit_ranges::ranges_for_edit(edits, self.edl.cursor(), index)
    }

    /// Lowest edit index Undo may reach (break-out founding Trim).
    pub fn undo_floor(&self) -> usize {
        self.edl.undo_floor()
    }

    fn fill_clipboard(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let clips = self.tree.clips_in_range(start, len, &mut || {
            n += 1;
            ClipId(n)
        });
        self.next_clip_id = n;
        self.clipboard = Clipboard {
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            clips,
        };
    }

    /// `copy`.
    pub fn copy(&mut self, start: u64, len: u64) {
        self.copy_ranges(&[(start, len)]);
    }

    /// `copy_ranges`.
    pub fn copy_ranges(&mut self, ranges: &[(u64, u64)]) {
        if ranges.is_empty() {
            return;
        }
        let mut n = self.next_clip_id;
        let mut clips = Vec::new();
        for &(start, len) in ranges {
            if len == 0 {
                continue;
            }
            clips.extend(self.tree.clips_in_range(start, len, &mut || {
                n += 1;
                ClipId(n)
            }));
        }
        self.next_clip_id = n;
        self.clipboard = Clipboard {
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            clips,
        };
        let (start, len) = ranges[0];
        self.edl
            .push(EditOp::Copy { start, len }, self.tree.clone());
    }

    /// `trim_ranges`.
    pub fn trim_ranges(&mut self, ranges: &[(u64, u64)]) {
        if ranges.is_empty() {
            return;
        }
        let mut n = self.next_clip_id;
        let mut clips = Vec::new();
        for &(start, len) in ranges {
            if len == 0 {
                continue;
            }
            clips.extend(self.tree.clips_in_range(start, len, &mut || {
                n += 1;
                ClipId(n)
            }));
        }
        let kept_len: u64 = clips.iter().map(|clip| clip.len).sum();
        let kept = ClipTree::from_clips(clips);
        let tree = self
            .tree
            .replace_range(0, self.tree.frames(), kept, &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        let (start, _) = ranges[0];
        self.commit(
            EditOp::Trim {
                start,
                len: kept_len,
            },
            tree,
        );
    }

    /// `remove`.
    pub fn remove(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::empty(), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Remove { start, len }, tree);
    }

    /// `cut`.
    pub fn cut(&mut self, start: u64, len: u64) {
        self.fill_clipboard(start, len);
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::empty(), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Cut { start, len }, tree);
    }

    /// Clear a range to silence of the same length (Edit → Clear).
    ///
    /// No-op when `len` is 0 or the range is already entirely silence clips
    /// (`source` is `None`), so repeating Clear does not stack EDL history.
    pub fn clear(&mut self, start: u64, len: u64) {
        if len == 0 {
            return;
        }
        let start = start.min(self.tree.frames());
        let len = len.min(self.tree.frames().saturating_sub(start));
        if len == 0 {
            return;
        }
        if range_is_silence(&self.tree, start, len) {
            return;
        }
        let silence = Clip::silence(self.alloc_clip_id(), len);
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::from_clip(silence), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Clear { start, len }, tree);
    }

    /// `paste`.
    pub fn paste(&mut self, at: u64) -> Result<()> {
        self.paste_replacing(at, 0)
    }

    /// `paste_replacing`.
    pub fn paste_replacing(&mut self, at: u64, replace_len: u64) -> Result<()> {
        if self.clipboard.is_empty() {
            return Ok(());
        }
        if self.clipboard.sample_rate != self.sample_rate
            || self.clipboard.channel_count != self.channel_count
        {
            bail!(
                "clipboard is {} Hz {} ch, composition is {} Hz {} ch",
                self.clipboard.sample_rate,
                self.clipboard.channel_count,
                self.sample_rate,
                self.channel_count
            );
        }
        let clipboard_clips = self.clipboard.clips.clone();
        let clips = self.remap_clips(&clipboard_clips);
        let pasted_len: u64 = clips.iter().map(|c| c.len).sum();
        let mut n = self.next_clip_id;
        let tree =
            self.tree
                .replace_range(at, replace_len, ClipTree::from_clips(clips), &mut || {
                    n += 1;
                    ClipId(n)
                });
        self.next_clip_id = n;
        self.commit(
            EditOp::Paste {
                at,
                len: pasted_len,
            },
            tree,
        );
        Ok(())
    }

    /// `trim`.
    pub fn trim(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let kept = ClipTree::from_clips(self.tree.clips_in_range(start, len, &mut || {
            n += 1;
            ClipId(n)
        }));
        let tree = self
            .tree
            .replace_range(0, self.tree.frames(), kept, &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Trim { start, len }, tree);
    }

    /// `duplicate`.
    pub fn duplicate(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let clips = self.tree.clips_in_range(start, len, &mut || {
            n += 1;
            ClipId(n)
        });
        let copies: Vec<_> = clips
            .into_iter()
            .map(|mut clip| {
                n += 1;
                clip.id = ClipId(n);
                clip
            })
            .collect();
        let tree =
            self.tree
                .replace_range(start + len, 0, ClipTree::from_clips(copies), &mut || {
                    n += 1;
                    ClipId(n)
                });
        self.next_clip_id = n;
        self.commit(EditOp::Duplicate { start, len }, tree);
    }

    /// `move_range`.
    pub fn move_range(&mut self, from: u64, len: u64, dest: u64) {
        if len == 0 || dest == from {
            return;
        }
        let mut n = self.next_clip_id;
        let clips = self.tree.clips_in_range(from, len, &mut || {
            n += 1;
            ClipId(n)
        });
        let extracted = self
            .tree
            .replace_range(from, len, ClipTree::empty(), &mut || {
                n += 1;
                ClipId(n)
            });
        let insert_at = if dest > from {
            dest.saturating_sub(len)
        } else {
            dest
        };
        let tree = extracted.replace_range(
            insert_at.min(extracted.frames()),
            0,
            ClipTree::from_clips(clips),
            &mut || {
                n += 1;
                ClipId(n)
            },
        );
        self.next_clip_id = n;
        self.commit(EditOp::Move { from, len, dest }, tree);
    }

    /// `roll`.
    pub fn roll(&mut self, at: u64, delta: i64) {
        let Some(span) = self.tree.at(at) else {
            return;
        };
        let Some(source) = &span.clip.source else {
            return;
        };
        let frame_count = self
            .store
            .lock()
            .unwrap()
            .pool()
            .get(source.media_id)
            .map(|media| media.frame_count);
        let Some(frame_count) = frame_count else {
            return;
        };
        let rolled = span.clip.with_rolled_offset(delta, frame_count);
        let tree = self.tree.map_clip_at(at, |_| rolled);
        self.commit(EditOp::Roll { at, delta }, tree);
    }

    /// `replace_range`.
    pub fn replace_range(&mut self, start: u64, len: u64, clips: Vec<Clip>) {
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::from_clips(clips), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Remove { start, len }, tree);
    }

    /// `read_planar`.
    pub fn read_planar(&self, start: u64, count: u64, dest: &mut [&mut [f32]]) -> Result<()> {
        for ch in dest.iter_mut() {
            let n = (*ch).len().min(count as usize);
            (*ch)[..n].fill(0.0);
        }
        if count == 0 || self.tree.is_empty() {
            return Ok(());
        }
        let start = start.min(self.frames());
        let count = count.min(self.frames().saturating_sub(start));
        let mut remaining = count;
        let mut pos = start;
        let mut dest_off = 0usize;
        while remaining > 0 {
            let Some(span) = self.tree.at(pos) else {
                break;
            };
            let local = pos - span.start;
            let take = remaining.min(span.clip.len - local);
            self.read_clip(span.clip.as_ref(), local, take, dest, dest_off)?;
            remaining -= take;
            pos += take;
            dest_off += take as usize;
        }
        Ok(())
    }

    /// `read_channel`.
    pub fn read_channel(&self, channel: usize, start: u64, dest: &mut [f32]) -> Result<()> {
        dest.fill(0.0);
        if dest.is_empty() || channel >= self.channel_count || self.tree.is_empty() {
            return Ok(());
        }
        let start = start.min(self.frames());
        let mut remaining = (dest.len() as u64).min(self.frames().saturating_sub(start));
        let mut pos = start;
        let mut dest_off = 0usize;
        while remaining > 0 {
            let Some(span) = self.tree.at(pos) else {
                break;
            };
            let local = pos - span.start;
            let take = remaining.min(span.clip.len - local) as usize;
            self.read_clip_channel(
                span.clip.as_ref(),
                channel,
                local,
                &mut dest[dest_off..dest_off + take],
            )?;
            remaining -= take as u64;
            pos += take as u64;
            dest_off += take;
        }
        Ok(())
    }

    /// `read_interleaved`.
    ///
    /// Fills destination using planar pager reads (one pager lock for the whole
    /// request), then interleaves. Scratch planes are thread-local so
    /// steady-state calls do not allocate.
    pub fn read_interleaved(&self, start: u64, count: u64, dest: &mut [f32]) -> Result<()> {
        let ch = self.channel_count.max(1);
        let frames = count as usize;
        let need = frames * ch;
        if dest.len() < need {
            bail!("destination is shorter than interleaved frame count");
        }
        dest[..need].fill(0.0);
        if count == 0 || self.channel_count == 0 {
            return Ok(());
        }

        thread_local! {
            static PLANAR: std::cell::RefCell<Vec<f32>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }

        PLANAR.with(|cell| -> Result<()> {
            let mut scratch = cell.borrow_mut();
            if scratch.len() < need {
                scratch.resize(need, 0.0);
            }
            scratch[..need].fill(0.0);

            let mut remaining = count;
            let mut pos = start.min(self.frames());
            let mut dest_off = 0usize;
            while remaining > 0 {
                let Some(span) = self.tree.at(pos) else {
                    break;
                };
                let local = pos - span.start;
                let take = remaining.min(span.clip.len - local) as usize;
                if take == 0 {
                    break;
                }
                if let Some(source) = &span.clip.source {
                    let media_id = source.media_id;
                    let offset = source.offset + local;
                    let map = self.source_channels.as_deref();
                    self.store
                        .lock()
                        .unwrap()
                        .with_locked_pager(|pool, pager| {
                            for c in 0..ch {
                                let plane_start = c * frames + dest_off;
                                let plane = &mut scratch[plane_start..plane_start + take];
                                let media_ch = map.and_then(|m| m.get(c).copied()).unwrap_or(c);
                                pager.fill_channel(
                                    pool,
                                    media_id,
                                    offset,
                                    take as u64,
                                    media_ch,
                                    plane,
                                )?;
                            }
                            Ok::<(), anyhow::Error>(())
                        })?;
                }
                if span.clip.fade_in != 0 || span.clip.fade_out != 0 {
                    for frame in 0..take {
                        let gain = span.clip.gain_at(local + frame as u64);
                        if (gain - 1.0).abs() < f32::EPSILON {
                            continue;
                        }
                        for c in 0..ch {
                            scratch[c * frames + dest_off + frame] *= gain;
                        }
                    }
                }
                remaining -= take as u64;
                pos += take as u64;
                dest_off += take;
            }

            for frame in 0..frames {
                for c in 0..ch {
                    dest[frame * ch + c] = scratch[c * frames + frame];
                }
            }
            Ok(())
        })
    }

    /// `fill_minmax_columns`.
    pub fn fill_minmax_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [(f32, f32)],
    ) {
        dest.fill((0.0, 0.0));
        if samples_per_pixel <= 0.0 || dest.is_empty() || channel >= self.channel_count {
            return;
        }
        let frames = self.frames() as f64;
        for (col, slot) in dest.iter_mut().enumerate() {
            let a = start + col as f64 * samples_per_pixel;
            if a >= frames {
                break;
            }
            *slot = self.min_max_in_range(channel, a, a + samples_per_pixel);
        }
    }

    /// `frames_iter`.
    pub fn frames_iter(&self, start: u64) -> FramesIter<'_> {
        FramesIter {
            composition: self,
            pos: start.min(self.frames()),
            buf: vec![0.0; self.channel_count.max(1)],
        }
    }

    /// `min_max_in_range`.
    pub fn min_max_in_range(&self, channel: usize, start: f64, end: f64) -> (f32, f32) {
        if self.frames() == 0 || channel >= self.channel_count {
            return (0.0, 0.0);
        }
        let start_i = start.max(0.0).floor() as u64;
        let end_i = (end.ceil() as u64).min(self.frames()).max(start_i);
        if start_i >= end_i {
            return (0.0, 0.0);
        }
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut pos = start_i;
        while pos < end_i {
            let Some(span) = self.tree.at(pos) else {
                break;
            };
            let local = pos - span.start;
            let take = (end_i - pos).min(span.clip.len - local);
            let (cmin, cmax) = self.clip_min_max(span.clip.as_ref(), channel, local, take);
            min = min.min(cmin);
            max = max.max(cmax);
            pos += take;
        }
        if min > max {
            (0.0, 0.0)
        } else {
            (min, max)
        }
    }

    fn clip_min_max(&self, clip: &Clip, channel: usize, local: u64, len: u64) -> (f32, f32) {
        if clip.source.is_none() || len == 0 {
            return (0.0, 0.0);
        }
        let Some(peaks) = clip.cache.peaks.get(channel) else {
            return (0.0, 0.0);
        };
        if peaks.is_empty() {
            // Missing overview bins: do not decode PCM on the UI thread.
            // `ensure_clip_peaks` / `build_missing_peak_caches` refill these.
            return (0.0, 0.0);
        }
        let peak_start = local as usize / PEAK_BLOCK;
        let peak_end = (((local + len) as usize + PEAK_BLOCK - 1) / PEAK_BLOCK).min(peaks.len());
        if peak_start < peak_end {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            for &(pmin, pmax) in &peaks[peak_start..peak_end] {
                min = min.min(pmin);
                max = max.max(pmax);
            }
            if min <= max {
                return (min, max);
            }
        }
        if clip.needs_peak_extend() {
            // Later bins have not been folded yet; keep the UI off PCM.
            return (0.0, 0.0);
        }
        // Uncovered tail after an unaligned split (at most one peak block).
        let take = (len as usize).min(PEAK_BLOCK);
        let mut buf = vec![0.0; take];
        let _ = self.read_clip_channel(clip, channel, local, &mut buf);
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for sample in buf {
            min = min.min(sample);
            max = max.max(sample);
        }
        if min > max {
            (0.0, 0.0)
        } else {
            (min, max)
        }
    }

    fn read_clip_channel(
        &self,
        clip: &Clip,
        channel: usize,
        local: u64,
        dest: &mut [f32],
    ) -> Result<()> {
        dest.fill(0.0);
        if dest.is_empty() {
            return Ok(());
        }
        if let Some(source) = &clip.source {
            let media_ch = self.media_channel(channel);
            self.store
                .lock()
                .unwrap()
                .with_locked_pager(|pool, pager| {
                    pager.fill_channel(
                        pool,
                        source.media_id,
                        source.offset + local,
                        dest.len() as u64,
                        media_ch,
                        dest,
                    )
                })?;
        }
        if clip.fade_in == 0 && clip.fade_out == 0 {
            return Ok(());
        }
        for (frame, sample) in dest.iter_mut().enumerate() {
            let gain = clip.gain_at(local + frame as u64);
            if (gain - 1.0).abs() >= f32::EPSILON {
                *sample *= gain;
            }
        }
        Ok(())
    }

    fn read_clip(
        &self,
        clip: &Clip,
        local: u64,
        count: u64,
        dest: &mut [&mut [f32]],
        dest_offset: usize,
    ) -> Result<()> {
        if let Some(source) = &clip.source {
            let map = self.source_channels.as_deref();
            self.store
                .lock()
                .unwrap()
                .with_locked_pager(|pool, pager| {
                    pager.fill_planar_mapped(
                        pool,
                        source.media_id,
                        source.offset + local,
                        count,
                        dest,
                        dest_offset,
                        map,
                    )
                })?;
        }
        if clip.fade_in == 0 && clip.fade_out == 0 {
            return Ok(());
        }
        for frame in 0..count as usize {
            let gain = clip.gain_at(local + frame as u64);
            if (gain - 1.0).abs() < f32::EPSILON {
                continue;
            }
            for plane in dest.iter_mut() {
                let i = dest_offset + frame;
                if i < plane.len() {
                    plane[i] *= gain;
                }
            }
        }
        Ok(())
    }

    /// `needs_peak_build`.
    pub fn needs_peak_build(&self) -> bool {
        self.spans()
            .iter()
            .any(|span| span.clip.needs_peak_extend())
    }

    /// Overview paint can proceed if some clips already have bins, even while
    /// others are still rebuilding after a split.
    pub fn can_paint_overview(&self) -> bool {
        !self.needs_peak_build()
            || self
                .spans()
                .iter()
                .any(|span| !span.clip.cache.is_missing_peaks())
    }

    /// `ensure_clip_peaks`.
    pub fn ensure_clip_peaks(&mut self) -> Result<()> {
        let updates = self.build_missing_peak_caches(None, 0)?;
        self.apply_peak_caches(updates);
        Ok(())
    }

    /// `apply_peak_caches`.
    pub fn apply_peak_caches(&mut self, updates: Vec<(u64, Clip)>) {
        for (_, peaked) in updates {
            self.tree = apply_clip_cache(&self.tree, &peaked);
            self.edl
                .map_snapshots(|tree| apply_clip_cache(tree, &peaked));
        }
    }

    fn append_peak_chunk(
        &mut self,
        target: &Clip,
        bins: Vec<Vec<(f32, f32)>>,
        chunk_min: f32,
        chunk_max: f32,
    ) {
        let key = clip_cache_key(target);
        let id = target.id;
        self.tree = extend_clip_cache(&self.tree, id, key, &bins, chunk_min, chunk_max);
        self.edl
            .map_snapshots(|tree| extend_clip_cache(tree, id, key, &bins, chunk_min, chunk_max));
    }

    /// `build_missing_peak_caches`.
    pub fn build_missing_peak_caches(
        &self,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<Vec<(u64, Clip)>> {
        let spans = self.tree.spans();
        let total: u64 = spans
            .iter()
            .filter(|span| span.clip.needs_peak_cache())
            .map(|span| span.clip.len.saturating_mul(self.channel_count as u64))
            .sum();
        let mut done = 0u64;
        let mut updated = Vec::new();
        if let Some(progress) = progress {
            progress.set_ratio(epoch, 0, total.max(1));
        }
        // Read pager-sized chunks, then fold into PEAK_BLOCK bins. Avoids
        // thousands of lock/decode round-trips for small files.
        for span in &spans {
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                break;
            }
            if !span.clip.needs_peak_cache() {
                continue;
            }
            let mut peaks = vec![Vec::new(); self.channel_count];
            let mut op = MinMaxOp::new(self.channel_count);
            let mut pos = 0u64;
            while pos < span.clip.len {
                if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                    return Ok(updated);
                }
                let take = ((span.clip.len - pos) as usize).min(BLOCK_FRAMES as usize);
                let mut planar = vec![vec![0.0; take]; self.channel_count];
                {
                    let mut dests: Vec<&mut [f32]> =
                        planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
                    self.read_clip(span.clip.as_ref(), pos, take as u64, &mut dests, 0)?;
                }
                for (ch, dest) in planar.iter().enumerate() {
                    op.consume_channel(ch, dest, &mut peaks[ch]);
                    done += take as u64;
                }
                if let Some(progress) = progress {
                    progress.set_ratio(epoch, done, total.max(1));
                }
                pos += take as u64;
            }
            for (ch, bins) in peaks.iter_mut().enumerate() {
                op.flush_channel(ch, bins);
            }
            let (min, max) = op.combined_global_min_max();
            let mut clip = (*span.clip).clone();
            clip.cache = super::clip::ClipCache {
                min: if min <= max { Some(min) } else { None },
                max: if min <= max { Some(max) } else { None },
                peaks,
            };
            updated.push((span.start, clip));
        }
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 1.0);
        }
        Ok(updated)
    }

    /// Fold one pager-sized chunk of overview bins into `composition`.
    ///
    /// The live tree is updated before this returns so the UI can paint
    /// leading peaks while later blocks are still decoding.
    pub fn build_next_peak_block(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        Self::build_next_analysis_block(composition, AnalysisKind::MinMax, progress, epoch)
    }

    /// Run one pager-sized step of `kind` against `composition`.
    pub fn build_next_analysis_block(
        composition: &RwLock<Self>,
        kind: AnalysisKind,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        match kind {
            AnalysisKind::MinMax => Self::build_next_minmax_block(composition, progress, epoch),
            AnalysisKind::EnvelopePeak => {
                Self::build_next_envelope_block(composition, progress, epoch)
            }
            AnalysisKind::Spectral => Self::build_next_spectral_block(composition, progress, epoch),
            AnalysisKind::Transients => {
                Self::build_next_transient_block(composition, progress, epoch)
            }
        }
    }

    /// Run one pager-sized step of a multi-kind pass (see [`Self::build_next_analysis_pass_block`]).
    pub fn build_next_analysis_kinds(
        composition: &RwLock<Self>,
        kinds: &[AnalysisKind],
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        Self::build_next_analysis_pass_block(composition, kinds, progress, epoch)
    }

    fn build_next_minmax_block(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        if progress.is_some_and(|p| !p.is_epoch(epoch)) {
            return Ok(AnalysisBlockOutcome::Cancelled);
        }
        let (clip, channel_count, pos, total_samples) = {
            let this = composition.read().unwrap();
            let channel_count = this.channel_count;
            let mut next = None;
            let mut total = 0u64;
            let mut covered = 0u64;
            for span in this.tree.spans() {
                if span.clip.source.is_none() {
                    continue;
                }
                let clip_total = span.clip.len.saturating_mul(channel_count as u64);
                total += clip_total;
                let clip_covered = peak_covered_samples(span.clip.as_ref())
                    .min(span.clip.len)
                    .saturating_mul(channel_count as u64);
                covered += clip_covered;
                if next.is_none() && span.clip.needs_peak_extend() {
                    next = Some((span.clip.clone(), peak_covered_samples(span.clip.as_ref())));
                }
            }
            let Some((clip, pos)) = next else {
                if let Some(progress) = progress {
                    progress.set_fraction(epoch, 1.0);
                }
                return Ok(AnalysisBlockOutcome::Complete);
            };
            if let Some(progress) = progress {
                progress.set_ratio(epoch, covered, total.max(1));
            }
            (clip, channel_count, pos, total)
        };
        if pos >= clip.len {
            return Ok(AnalysisBlockOutcome::Complete);
        }
        if progress.is_some_and(|p| !p.is_epoch(epoch)) {
            return Ok(AnalysisBlockOutcome::Cancelled);
        }
        let take = ((clip.len - pos) as usize).min(BLOCK_FRAMES as usize);
        let mut bins = vec![Vec::new(); channel_count];
        let mut planar = vec![vec![0.0; take]; channel_count];
        {
            let this = composition.read().unwrap();
            let mut dests: Vec<&mut [f32]> =
                planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
            this.read_clip(clip.as_ref(), pos, take as u64, &mut dests, 0)?;
        }
        {
            let mut this = composition.write().unwrap();
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                return Ok(AnalysisBlockOutcome::Cancelled);
            }
            let clip_done = pos + take as u64 >= clip.len;
            if pos == 0 {
                this.minmax_op = Some(MinMaxOp::new(channel_count));
            }
            let (min, max) = {
                let op = this
                    .minmax_op
                    .get_or_insert_with(|| MinMaxOp::new(channel_count));
                for (ch, dest) in planar.iter().enumerate() {
                    op.consume_channel(ch, dest, &mut bins[ch]);
                }
                if clip_done {
                    for (ch, out) in bins.iter_mut().enumerate() {
                        op.flush_channel(ch, out);
                    }
                }
                op.combined_global_min_max()
            };
            if clip_done {
                this.minmax_op = None;
            }
            this.append_peak_chunk(&clip, bins, min, max);
            if let Some(progress) = progress {
                let covered: u64 = this
                    .tree
                    .spans()
                    .into_iter()
                    .filter(|span| span.clip.source.is_some())
                    .map(|span| {
                        peak_covered_samples(span.clip.as_ref())
                            .min(span.clip.len)
                            .saturating_mul(this.channel_count as u64)
                    })
                    .sum();
                progress.set_ratio(epoch, covered, total_samples.max(1));
            }
            if this.needs_peak_build() {
                Ok(AnalysisBlockOutcome::Progress)
            } else {
                if let Some(progress) = progress {
                    progress.set_fraction(epoch, 1.0);
                }
                Ok(AnalysisBlockOutcome::Complete)
            }
        }
    }

    /// Decode every missing overview block, applying each pager chunk as it
    /// finishes so a shared UI lock can paint partial peaks.
    pub fn build_missing_peak_caches_shared(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        loop {
            match Self::build_next_peak_block(composition, progress, epoch)? {
                AnalysisBlockOutcome::Progress => {}
                other => return Ok(other),
            }
        }
    }

    /// Whether envelope-peak analysis covers the full timeline.
    pub fn needs_envelope_peak_build(&self) -> bool {
        let frames = self.frames();
        frames > 0
            && !self
                .analysis_streams
                .envelope_peak_ready(frames, self.channel_count)
    }

    /// Whether any envelope-peak bins are available for progressive paint.
    pub fn envelope_peak_has_data(&self) -> bool {
        self.analysis_streams
            .envelope_peak_has_data(self.channel_count)
    }

    /// Whether spectral analysis covers the full timeline.
    pub fn needs_spectral_build(&self) -> bool {
        let frames = self.frames();
        frames > 0
            && !self
                .analysis_streams
                .spectral_ready(frames, self.channel_count)
    }

    /// Whether any spectral hops are available for progressive paint.
    pub fn spectral_has_data(&self) -> bool {
        self.analysis_streams.spectral_has_data(self.channel_count)
    }

    /// Access in-memory analysis streams.
    pub fn analysis_streams(&self) -> &AnalysisStreams {
        &self.analysis_streams
    }

    /// Mutable access to in-memory analysis streams.
    pub fn analysis_streams_mut(&mut self) -> &mut AnalysisStreams {
        &mut self.analysis_streams
    }

    /// Frames remaining in the active analysis job target (for progress / tests).
    pub fn analysis_target_total(&self) -> u64 {
        self.analysis_target_total
    }

    /// Fill overview columns from the envelope-peak stream when present.
    pub fn fill_envelope_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [f32],
    ) {
        let Some(series) = self.analysis_streams.float(AnalysisKind::EnvelopePeak) else {
            dest.fill(0.0);
            return;
        };
        if channel >= series.channels.len() {
            dest.fill(0.0);
            return;
        }
        let bins = &series.channels[channel];
        let hop = series.hop.max(1) as f64;
        let hop_usize = series.hop.max(1);
        let covered = series.covered_hops().min(bins.len());
        for (i, slot) in dest.iter_mut().enumerate() {
            let a = start + i as f64 * samples_per_pixel;
            let b = a + samples_per_pixel;
            let start_bin = (a / hop).floor().max(0.0) as usize;
            let end_bin = ((b / hop).ceil() as usize).min(covered);
            if start_bin >= end_bin || covered == 0 {
                *slot = 0.0;
                continue;
            }
            let mut max = 0.0f32;
            let mut any = false;
            for bin in start_bin..end_bin {
                let frame = bin as u64 * hop_usize as u64;
                if series.is_dirty_frame(frame) {
                    continue;
                }
                max = max.max(bins[bin]);
                any = true;
            }
            *slot = if any { max } else { 0.0 };
        }
    }

    /// Fill packed `width * band_count` dB columns from the spectral stream.
    ///
    /// `dest.len()` must be a multiple of [`SPECTRAL_BAND_COUNT`]. Each pixel
    /// column stores low→high bands; overlapping hops contribute their max.
    /// Dirty (not yet recomputed) hops are skipped so they paint as the floor.
    pub fn fill_spectral_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [f32],
    ) {
        let band_count = SPECTRAL_BAND_COUNT;
        if band_count == 0 || dest.len() % band_count != 0 {
            dest.fill(SPECTRAL_DB_FLOOR);
            return;
        }
        let width = dest.len() / band_count;
        dest.fill(SPECTRAL_DB_FLOOR);
        let Some(series) = self.analysis_streams.spectral() else {
            return;
        };
        if channel >= series.channels.len() || series.band_count != band_count {
            return;
        }
        let data = &series.channels[channel];
        let hop = series.hop.max(1) as f64;
        let hop_usize = series.hop.max(1);
        let hop_count = series.hop_count(channel).min(series.covered_hops());
        if hop_count == 0 {
            return;
        }
        for i in 0..width {
            let a = start + i as f64 * samples_per_pixel;
            let b = a + samples_per_pixel;
            let start_hop = (a / hop).floor().max(0.0) as usize;
            let end_hop = ((b / hop).ceil() as usize).min(hop_count);
            if start_hop >= end_hop {
                continue;
            }
            let col = &mut dest[i * band_count..(i + 1) * band_count];
            for h in start_hop..end_hop {
                let frame = h as u64 * hop_usize as u64;
                if series.is_dirty_frame(frame) {
                    continue;
                }
                let frame_data = &data[h * band_count..(h + 1) * band_count];
                for (band, slot) in col.iter_mut().enumerate() {
                    *slot = slot.max(frame_data[band]);
                }
            }
        }
    }

    fn build_next_envelope_block(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        if progress.is_some_and(|p| !p.is_epoch(epoch)) {
            return Ok(AnalysisBlockOutcome::Cancelled);
        }
        {
            let mut this = composition.write().unwrap();
            if !this.analysis_job_started {
                let sample_rate = this.sample_rate;
                let channel_count = this.channel_count;
                let frames = this.frames();
                let hop = AnalysisStreams::envelope_hop();
                let bins_needed = if frames == 0 {
                    0
                } else {
                    ((frames as usize) + hop - 1) / hop
                };
                let use_dirty_target = {
                    let series = this.analysis_streams.ensure_float(
                        AnalysisKind::EnvelopePeak,
                        channel_count,
                        hop,
                    );
                    let regional_rebuild = !series.dirty_ranges.is_empty()
                        && series.channels.iter().all(|ch| ch.len() == bins_needed);
                    if regional_rebuild {
                        Some(series.dirty_ranges.clone())
                    } else {
                        for ch in &mut series.channels {
                            ch.clear();
                            ch.resize(bins_needed, 0.0);
                        }
                        series.covered_frames = frames;
                        series.dirty_ranges.clear();
                        None
                    }
                };
                if let Some(dirty) = use_dirty_target {
                    if !this.analysis_target_configured {
                        this.begin_analysis_target(Some(dirty));
                    }
                } else if !this.analysis_target_configured {
                    this.begin_analysis_target(None);
                }
                // Align dirty holes with the job target (full, selection, or edit).
                let targets = this.analysis_target_ranges.clone();
                if let Some(series) = this.analysis_streams.float_mut(AnalysisKind::EnvelopePeak) {
                    if series.dirty_ranges.is_empty() {
                        series.dirty_ranges = targets;
                    }
                }
                this.envelope_op = Some(EnvelopePeakOp::new(sample_rate, channel_count));
                this.analysis_job_started = true;
            }
        }
        let warmup_frames = {
            let this = composition.read().unwrap();
            match AnalysisKind::EnvelopePeak.recompute_scope(this.sample_rate) {
                RecomputeScope::Regional(r) => r.warmup_frames,
                RecomputeScope::FullTimeline => 0,
            }
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
            let mut this = composition.write().unwrap();
            this.envelope_op = None;
            let frames = this.frames();
            if let Some(series) = this.analysis_streams.float_mut(AnalysisKind::EnvelopePeak) {
                series.covered_frames = frames;
            }
            this.analysis_job_started = false;
            this.analysis_target_configured = false;
            if let Some(progress) = progress {
                progress.set_fraction(epoch, 1.0);
            }
            return Ok(AnalysisBlockOutcome::Complete);
        }
        if let Some(progress) = progress {
            progress.set_ratio(epoch, done, total.max(1));
        }

        // Warmup lookback at the start of each target range.
        let mut prime_planar: Option<Vec<Vec<f32>>> = None;
        if read_pos == range_start && warmup_frames > 0 && range_start > 0 {
            let prime_start = range_start.saturating_sub(warmup_frames);
            let prime_len = (range_start - prime_start) as usize;
            let mut planar = vec![vec![0.0; prime_len]; channel_count];
            {
                let this = composition.read().unwrap();
                let mut dests: Vec<&mut [f32]> =
                    planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
                this.read_planar(prime_start, prime_len as u64, &mut dests)?;
            }
            prime_planar = Some(planar);
        }

        let mut planar = vec![vec![0.0; take]; channel_count];
        {
            let this = composition.read().unwrap();
            let mut dests: Vec<&mut [f32]> =
                planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
            this.read_planar(read_pos, take as u64, &mut dests)?;
        }
        {
            let mut this = composition.write().unwrap();
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                return Ok(AnalysisBlockOutcome::Cancelled);
            }
            let sample_rate = this.sample_rate;
            if read_pos == range_start {
                this.envelope_op = Some(EnvelopePeakOp::new(sample_rate, channel_count));
                if let Some(prime) = prime_planar.as_ref() {
                    let op = this.envelope_op.as_mut().expect("just created");
                    for (ch, plane) in prime.iter().enumerate() {
                        op.prime_channel(ch, plane);
                    }
                }
            }
            let hop = AnalysisStreams::envelope_hop();
            let mut hop_bins = vec![Vec::new(); channel_count];
            let range_end = this
                .analysis_target_ranges
                .iter()
                .find(|(s, e)| read_pos >= *s && read_pos < *e)
                .map(|(_, e)| *e)
                .unwrap_or(read_pos + take as u64);
            {
                let op = this
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
            let next_pos = advance_read_pos(&this.analysis_target_ranges, read_pos + take as u64);
            let frames = this.frames();
            this.analysis_target_done += take as u64;
            this.analysis_read_pos = next_pos;
            let done_now = this.analysis_target_done;
            let complete = done_now >= this.analysis_target_total;
            {
                let series = this.analysis_streams.ensure_float(
                    AnalysisKind::EnvelopePeak,
                    channel_count,
                    hop,
                );
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
            if let Some(progress) = progress {
                progress.set_ratio(epoch, done_now, total.max(1));
            }
            if complete {
                this.envelope_op = None;
                this.analysis_job_started = false;
                this.analysis_target_configured = false;
                if let Some(progress) = progress {
                    progress.set_fraction(epoch, 1.0);
                }
                Ok(AnalysisBlockOutcome::Complete)
            } else {
                Ok(AnalysisBlockOutcome::Progress)
            }
        }
    }

    fn build_next_spectral_block(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        if progress.is_some_and(|p| !p.is_epoch(epoch)) {
            return Ok(AnalysisBlockOutcome::Cancelled);
        }
        {
            let mut this = composition.write().unwrap();
            if !this.analysis_job_started {
                let sample_rate = this.sample_rate;
                let channel_count = this.channel_count;
                let frames = this.frames();
                let hop = AnalysisStreams::spectral_hop();
                let band_count = SPECTRAL_BAND_COUNT;
                let hops_needed = if frames == 0 {
                    0
                } else {
                    ((frames as usize) + hop - 1) / hop
                };
                let target_ranges = {
                    let series = this.analysis_streams.ensure_spectral(channel_count);
                    let regional_rebuild = !series.dirty_ranges.is_empty()
                        && series.hop_count(0) == hops_needed
                        && series.band_count == band_count;
                    if regional_rebuild {
                        series.dirty_ranges.clone()
                    } else {
                        for ch in &mut series.channels {
                            ch.clear();
                            ch.resize(hops_needed * band_count, SPECTRAL_DB_FLOOR);
                        }
                        series.covered_frames = frames;
                        series.hop = hop;
                        series.band_count = band_count;
                        series.fft_size = field_audio_process::SPECTRAL_FFT_SIZE;
                        series.dirty_ranges = if frames == 0 {
                            Vec::new()
                        } else {
                            vec![(0, frames)]
                        };
                        series.dirty_ranges.clone()
                    }
                };
                this.begin_analysis_target(Some(target_ranges));
                this.spectral_op = Some(SpectralOp::new(sample_rate, channel_count));
                this.analysis_job_started = true;
            }
        }
        let warmup_frames = {
            let this = composition.read().unwrap();
            match AnalysisKind::Spectral.recompute_scope(this.sample_rate) {
                RecomputeScope::Regional(r) => r.warmup_frames,
                RecomputeScope::FullTimeline => 0,
            }
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
            let mut this = composition.write().unwrap();
            this.spectral_op = None;
            let frames = this.frames();
            if let Some(series) = this.analysis_streams.spectral_mut() {
                series.covered_frames = frames;
            }
            this.analysis_job_started = false;
            this.analysis_target_configured = false;
            if let Some(progress) = progress {
                progress.set_fraction(epoch, 1.0);
            }
            return Ok(AnalysisBlockOutcome::Complete);
        }
        if let Some(progress) = progress {
            progress.set_ratio(epoch, done, total.max(1));
        }

        let mut prime_planar: Option<Vec<Vec<f32>>> = None;
        if read_pos == range_start && warmup_frames > 0 && range_start > 0 {
            let prime_start = range_start.saturating_sub(warmup_frames);
            let prime_len = (range_start - prime_start) as usize;
            let mut planar = vec![vec![0.0; prime_len]; channel_count];
            {
                let this = composition.read().unwrap();
                let mut dests: Vec<&mut [f32]> =
                    planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
                this.read_planar(prime_start, prime_len as u64, &mut dests)?;
            }
            prime_planar = Some(planar);
        }

        let mut planar = vec![vec![0.0; take]; channel_count];
        {
            let this = composition.read().unwrap();
            let mut dests: Vec<&mut [f32]> =
                planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
            this.read_planar(read_pos, take as u64, &mut dests)?;
        }
        {
            let mut this = composition.write().unwrap();
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                return Ok(AnalysisBlockOutcome::Cancelled);
            }
            let sample_rate = this.sample_rate;
            if read_pos == range_start {
                this.spectral_op = Some(SpectralOp::new(sample_rate, channel_count));
                if let Some(prime) = prime_planar.as_ref() {
                    let op = this.spectral_op.as_mut().expect("just created");
                    for (ch, plane) in prime.iter().enumerate() {
                        op.prime_channel(ch, plane);
                    }
                }
            }
            let hop = AnalysisStreams::spectral_hop();
            let band_count = SPECTRAL_BAND_COUNT;
            let mut hop_frames = vec![Vec::new(); channel_count];
            let range_end = this
                .analysis_target_ranges
                .iter()
                .find(|(s, e)| read_pos >= *s && read_pos < *e)
                .map(|(_, e)| *e)
                .unwrap_or(read_pos + take as u64);
            {
                let op = this
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
            let next_pos = advance_read_pos(&this.analysis_target_ranges, read_pos + take as u64);
            let frames = this.frames();
            this.analysis_target_done += take as u64;
            this.analysis_read_pos = next_pos;
            let done_now = this.analysis_target_done;
            let complete = done_now >= this.analysis_target_total;
            {
                let series = this.analysis_streams.ensure_spectral(channel_count);
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
            if let Some(progress) = progress {
                progress.set_ratio(epoch, done_now, total.max(1));
            }
            if complete {
                this.spectral_op = None;
                this.analysis_job_started = false;
                this.analysis_target_configured = false;
                if let Some(progress) = progress {
                    progress.set_fraction(epoch, 1.0);
                }
                Ok(AnalysisBlockOutcome::Complete)
            } else {
                Ok(AnalysisBlockOutcome::Progress)
            }
        }
    }

    fn build_next_transient_block(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<AnalysisBlockOutcome> {
        if progress.is_some_and(|p| !p.is_epoch(epoch)) {
            let mut this = composition.write().unwrap();
            this.abandon_analysis_job_scratch();
            return Ok(AnalysisBlockOutcome::Cancelled);
        }
        {
            let mut this = composition.write().unwrap();
            if !this.analysis_job_started {
                if !this.analysis_target_configured {
                    this.begin_analysis_target(None);
                }
                this.ensure_transient_marker_type();
                // Do not remove existing Transient markers until the job
                // completes successfully — a cancelled pass must not wipe them.
                this.transient_op = None;
                this.analysis_job_started = true;
            }
        }
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
            let mut this = composition.write().unwrap();
            let mut sink = AnalysisSink::default();
            if let Some(mut op) = this.transient_op.take() {
                op.finish(&mut sink);
            }
            this.apply_transient_markers(sink.markers);
            this.analysis_job_started = false;
            this.analysis_target_configured = false;
            if let Some(progress) = progress {
                progress.set_fraction(epoch, 1.0);
            }
            return Ok(AnalysisBlockOutcome::Complete);
        }
        if let Some(progress) = progress {
            progress.set_ratio(epoch, done, total.max(1));
        }
        let sample_rate = {
            let this = composition.read().unwrap();
            this.sample_rate
        };
        // Pre-roll before a mid-timeline range so peak followers are settled
        // (Selection Only would otherwise miss onsets at the range start).
        let warmup = if read_pos == range_start && read_pos > 0 {
            TransientDetectOp::warmup_frames(sample_rate).min(read_pos)
        } else {
            0
        };
        let mut warm_planar = if warmup > 0 {
            let mut planes = vec![vec![0.0; warmup as usize]; channel_count];
            let this = composition.read().unwrap();
            let mut dests: Vec<&mut [f32]> =
                planes.iter_mut().map(|ch| ch.as_mut_slice()).collect();
            this.read_planar(read_pos - warmup, warmup, &mut dests)?;
            Some(planes)
        } else {
            None
        };
        let mut planar = vec![vec![0.0; take]; channel_count];
        {
            let this = composition.read().unwrap();
            let mut dests: Vec<&mut [f32]> =
                planar.iter_mut().map(|ch| ch.as_mut_slice()).collect();
            this.read_planar(read_pos, take as u64, &mut dests)?;
        }
        {
            let mut this = composition.write().unwrap();
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                this.abandon_analysis_job_scratch();
                return Ok(AnalysisBlockOutcome::Cancelled);
            }
            if read_pos == range_start {
                this.transient_op = Some(TransientDetectOp::new(sample_rate, channel_count));
                if let Some(op) = this.transient_op.as_mut() {
                    if let Some(warm) = warm_planar.take() {
                        op.begin(read_pos - warmup);
                        op.consume_planar(&warm);
                    }
                    // Keep follower state from pre-roll; drop pre-roll hits.
                    op.begin(read_pos);
                }
            }
            if let Some(op) = this.transient_op.as_mut() {
                op.consume_planar(&planar);
            }
            this.analysis_target_done += take as u64;
            this.analysis_read_pos =
                advance_read_pos(&this.analysis_target_ranges, read_pos + take as u64);
            if let Some(progress) = progress {
                progress.set_ratio(epoch, this.analysis_target_done, total.max(1));
            }
            if this.analysis_target_done >= this.analysis_target_total {
                let mut sink = AnalysisSink::default();
                if let Some(mut op) = this.transient_op.take() {
                    op.finish(&mut sink);
                }
                this.apply_transient_markers(sink.markers);
                this.analysis_job_started = false;
                this.analysis_target_configured = false;
                if let Some(progress) = progress {
                    progress.set_fraction(epoch, 1.0);
                }
                Ok(AnalysisBlockOutcome::Complete)
            } else {
                Ok(AnalysisBlockOutcome::Progress)
            }
        }
    }

    /// Replace Transient markers in the active analysis target with `markers`.
    fn apply_transient_markers(&mut self, markers: Vec<field_audio_model::NewMarker>) {
        let ranges = self.analysis_target_ranges.clone();
        for &(start, end) in &ranges {
            if end > start {
                self.remove_markers_of_type_in_range(
                    start,
                    end.saturating_sub(1),
                    MARKER_TYPE_TRANSIENT,
                );
            }
        }
        for marker in markers {
            self.add_marker(marker.frame, marker.marker_type, marker.note);
        }
    }

    /// Drop in-flight op scratch when a job is superseded. Leaves target
    /// ranges alone so a newer `begin_analysis_target` is not wiped.
    fn abandon_analysis_job_scratch(&mut self) {
        self.minmax_op = None;
        self.envelope_op = None;
        self.spectral_op = None;
        self.transient_op = None;
        self.analysis_job_started = false;
        self.analysis_pass_kinds.clear();
        self.analysis_minmax_clip = None;
    }

    fn ensure_transient_marker_type(&mut self) {
        if self.marker_type_color(MARKER_TYPE_TRANSIENT).is_none() {
            let color = field_audio_model::marker_type_color(MARKER_TYPE_TRANSIENT).unwrap_or([
                0xec as f32 / 255.0,
                0x48 as f32 / 255.0,
                0x99 as f32 / 255.0,
                1.0,
            ]);
            let _ = self.add_marker_type(MARKER_TYPE_TRANSIENT, color);
        }
    }

    fn remove_markers_of_type_in_range(&mut self, start: u64, end: u64, marker_type: &str) {
        let ids: Vec<_> = self
            .markers
            .iter()
            .filter(|m| m.marker_type == marker_type && m.frame >= start && m.frame <= end)
            .map(|m| m.id)
            .collect();
        for id in ids {
            self.remove_marker(id);
        }
    }

    /// Media ids referenced by the initial state or current clip tree.
    pub fn used_media_ids(&self) -> HashSet<MediaId> {
        let mut ids = HashSet::new();
        if let InitialState::FromMedia { media_id } = self.initial {
            ids.insert(media_id);
        }
        for span in self.spans() {
            if let Some(source) = &span.clip.source {
                ids.insert(source.media_id);
            }
        }
        ids
    }

    fn used_media_refs(&self) -> Vec<MediaRef> {
        let ids = self.used_media_ids();
        let store = self.store.lock().unwrap();
        let pool = store.pool();
        let mut refs: Vec<_> = ids
            .into_iter()
            .filter_map(|id| pool.get(id).cloned())
            .collect();
        refs.sort_by_key(|m| m.id.to_hex());
        refs
    }

    /// `to_project_file`.
    pub fn to_project_file(&self) -> ProjectFile {
        ProjectFile {
            id: Some(self.id),
            parent: self.parent,
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            media: self.used_media_refs(),
            initial: self.initial.clone(),
            edits: self.edl.ops_from_first_user(),
            edit_cursor: self.edl.cursor(),
            undo_floor: self.edl.undo_floor(),
            markers: self.markers.iter().map(StoredMarker::from).collect(),
            marker_types: self.marker_types.clone(),
            collections: self.collections.clone(),
            channel_layout: self.chosen_channel_layout.clone(),
            monitor_chain: self.monitor_chain.clone(),
            playback_channels: self.playback_channels.clone(),
            source_channels: self.source_channels.clone(),
        }
    }

    /// `to_json`.
    pub fn to_json(&self) -> Result<String> {
        self.to_json_with_base(None)
    }

    /// `to_json_with_base`.
    pub fn to_json_with_base(&self, base: Option<&Path>) -> Result<String> {
        let mut file = self.to_project_file();
        for media in &mut file.media {
            media.prepare_url(base);
        }
        ProjectEnvelope::wrap(file).to_json()
    }

    /// `from_project_file`.
    pub fn from_project_file(file: ProjectFile) -> Result<Self> {
        Self::from_project_file_into_store(file, None)
    }

    fn from_project_file_into_store(
        file: ProjectFile,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<Self> {
        let store = resolve_media_store(store);
        for media in &file.media {
            store.lock().unwrap().intern(media.clone());
        }
        let sample_rate = file.sample_rate;
        let channel_count = file.channel_count;
        let (id, identity_dirty) = match file.id {
            Some(id) => (id, false),
            None => (CompositionId::new(), true),
        };
        let parent = file.parent;
        let undo_floor = file.undo_floor;
        let initial = file.initial.clone();
        let mut composition = match &initial {
            InitialState::Empty => {
                let mut composition = Composition::new(sample_rate, channel_count);
                composition.store = Arc::clone(&store);
                composition
            }
            InitialState::FromMedia { media_id } => {
                store
                    .lock()
                    .unwrap()
                    .pool()
                    .get(*media_id)
                    .cloned()
                    .context("initial media missing from project")?;
                Self::bootstrap_from_media_id(*media_id, sample_rate, channel_count, store)?
            }
        };
        composition.id = id;
        composition.parent = parent;
        composition.identity_dirty = identity_dirty;
        for op in file.edits {
            composition.replay(&op)?;
        }
        if let Some(tree) = composition.edl.jump_to_index(file.edit_cursor) {
            composition.adopt_tree(tree);
        }
        composition.edl.set_undo_floor(undo_floor);
        composition.markers =
            MarkerList::from_vec(file.markers.iter().cloned().map(Marker::from).collect());
        if file.marker_types.is_empty() {
            composition.marker_types = MarkerType::defaults();
        } else {
            composition.marker_types = file.marker_types;
        }
        for stored in &file.markers {
            if let Some(color) = stored.color {
                if composition.marker_type_color(&stored.marker_type).is_none() {
                    composition.add_marker_type(&stored.marker_type, color);
                }
            }
        }
        composition.collections = file
            .collections
            .into_iter()
            .filter(|col| col.name != SELECTION_COLLECTION && !col.name.is_empty())
            .collect();
        composition.bump_next_region_id_from_collections();
        composition.chosen_channel_layout = file.channel_layout;
        composition.monitor_chain = file.monitor_chain.filter(|name| !name.is_empty());
        composition.playback_channels =
            normalize_playback_channels(file.playback_channels, composition.channel_count);
        composition.source_channels =
            normalize_source_channels(file.source_channels, composition.channel_count);
        if identity_dirty {
            // Minted id for a legacy file — leave dirty so the next save
            // persists identity.
            composition.clean_edit_id = composition.edl.current_id();
            composition.clean_markers = composition.markers.to_vec();
            composition.clean_collections = composition.collections.clone();
            composition.identity_dirty = true;
        } else {
            composition.mark_clean();
        }
        Ok(composition)
    }

    fn bootstrap_from_media_id(
        media_id: MediaId,
        sample_rate: u32,
        channel_count: usize,
        store: Arc<Mutex<MediaStore>>,
    ) -> Result<Self> {
        let frame_count = store
            .lock()
            .unwrap()
            .pool()
            .get(media_id)
            .context("initial media missing from project")?
            .frame_count;
        let mut next_clip_id = 1;
        let clip = Clip::from_media(ClipId(next_clip_id), media_id, 0, frame_count);
        next_clip_id += 1;
        let tree = ClipTree::from_clip(clip);
        let mut composed = Self {
            id: CompositionId::new(),
            parent: None,
            identity_dirty: false,
            display_title: None,
            path_display_name: None,
            sample_rate,
            channel_count,
            edl: Edl::new(tree.clone()),
            tree,
            store,
            next_clip_id,
            clipboard: Clipboard {
                sample_rate,
                channel_count,
                clips: Vec::new(),
            },
            initial: InitialState::FromMedia { media_id },
            markers: MarkerList::new(),
            marker_types: MarkerType::defaults(),
            collections: Vec::new(),
            next_region_id: 1,
            channel_layout: None,
            chosen_channel_layout: None,
            channel_labels: BTreeMap::new(),
            clean_edit_id: EditId(0),
            clean_markers: Vec::new(),
            clean_collections: Vec::new(),
            monitor_chain: None,
            playback_channels: None,
            source_channels: None,
            analysis_streams: AnalysisStreams::default(),
            minmax_op: None,
            envelope_op: None,
            spectral_op: None,
            transient_op: None,
            analysis_target_ranges: Vec::new(),
            analysis_read_pos: 0,
            analysis_target_total: 0,
            analysis_target_done: 0,
            analysis_job_started: false,
            analysis_target_configured: false,
            analysis_pass_kinds: Vec::new(),
            analysis_minmax_clip: None,
            analysis_pass_stats: AnalysisPassStats::default(),
        };
        let peaked = composed
            .store
            .lock()
            .unwrap()
            .pool()
            .first()
            .is_some_and(|media| media.samples.is_some());
        if peaked {
            composed.ensure_clip_peaks().ok();
            composed.edl = Edl::new(composed.tree.clone());
        }
        Ok(composed)
    }

    /// `from_json`.
    pub fn from_json(json: &str) -> Result<Self> {
        Self::from_json_at(json, None)
    }

    /// `from_json_at`.
    pub fn from_json_at(json: &str, base: Option<&Path>) -> Result<Self> {
        let envelope = ProjectEnvelope::from_json(json)?;
        let mut file = envelope.project;
        for media in &mut file.media {
            media.resolve_url(base)?;
        }
        Self::from_project_file(file)
    }

    /// `from_json_reprobing`.
    pub fn from_json_reprobing(json: &str) -> Result<(Self, OpenReport)> {
        Self::from_json_reprobing_at(json, None)
    }

    /// `from_json_reprobing_at`.
    pub fn from_json_reprobing_at(json: &str, base: Option<&Path>) -> Result<(Self, OpenReport)> {
        Self::from_json_reprobing_at_into_store(json, base, None)
    }

    fn from_json_reprobing_at_into_store(
        json: &str,
        base: Option<&Path>,
        store: Option<Arc<Mutex<MediaStore>>>,
    ) -> Result<(Self, OpenReport)> {
        let target = match base {
            Some(base) => Location::from_path(base),
            None => Location::parse("memory://composition").unwrap(),
        };
        let mut report = OpenReport::new(target);
        let envelope = match ProjectEnvelope::from_json(json) {
            Ok(envelope) => envelope,
            Err(err) => {
                let msg = err.to_string();
                let category = if msg.contains("format_version") {
                    ProblemCategory::UnsupportedFormat
                } else {
                    ProblemCategory::CorruptDocument
                };
                report.push(LoadProblem::new(category, "composition", msg, None));
                return Err(err);
            }
        };
        let mut file = envelope.project;
        let mut resolved = Vec::with_capacity(file.media.len());
        for mut media in file.media {
            if let Err(err) = media.resolve_url(base) {
                report.push(LoadProblem::new(
                    ProblemCategory::InvalidLocation,
                    media.basename.clone(),
                    err.to_string(),
                    Some(media.url.clone()),
                ));
                media.availability = MediaAvailability::Missing;
                resolved.push(media);
                continue;
            }
            let path_str = media.path.to_string_lossy();
            if path_str.starts_with("memory://") {
                resolved.push(media);
                continue;
            }
            if !media.path.is_file() {
                media.availability = MediaAvailability::Missing;
                report.push(LoadProblem::new(
                    ProblemCategory::MissingReference,
                    media.basename.clone(),
                    format!("missing source media {}", media.path.display()),
                    Some(media.url.clone()),
                ));
                resolved.push(media);
                continue;
            }
            let meta = match std::fs::metadata(&media.path) {
                Ok(meta) => meta,
                Err(err) => {
                    media.availability = MediaAvailability::Missing;
                    report.push(LoadProblem::new(
                        ProblemCategory::Other,
                        media.basename.clone(),
                        format!(
                            "failed to stat source media {}: {err}",
                            media.path.display()
                        ),
                        Some(media.url.clone()),
                    ));
                    resolved.push(media);
                    continue;
                }
            };
            if media_stats_match(&media, &meta) {
                resolved.push(media);
                continue;
            }
            media.availability = MediaAvailability::FreshnessMismatch;
            report.push(LoadProblem::new(
                ProblemCategory::MediaMismatch,
                media.basename.clone(),
                format!("source media changed: {}", media.path.display()),
                Some(media.url.clone()),
            ));
            match probe_header(&media.path) {
                Ok(probed) => {
                    let mut next = media_ref_from_probed(probed);
                    next.id = media.id;
                    next.path = media.path;
                    next.url = media.url;
                    next.availability = MediaAvailability::FreshnessMismatch;
                    resolved.push(next);
                }
                Err(_) => {
                    let mut next = media;
                    next.size_bytes = meta.len();
                    next.modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    resolved.push(next);
                }
            }
        }
        file.media = resolved;
        Ok((Self::from_project_file_into_store(file, store)?, report))
    }

    fn replay(&mut self, op: &EditOp) -> Result<()> {
        match op {
            EditOp::Init => Ok(()),
            EditOp::Copy { start, len } => {
                self.copy(*start, *len);
                Ok(())
            }
            EditOp::Cut { start, len } => {
                self.cut(*start, *len);
                Ok(())
            }
            EditOp::Remove { start, len } => {
                self.remove(*start, *len);
                Ok(())
            }
            EditOp::Clear { start, len } => {
                self.clear(*start, *len);
                Ok(())
            }
            EditOp::Paste { at, .. } => self.paste(*at),
            EditOp::Trim { start, len } => {
                self.trim(*start, *len);
                Ok(())
            }
            EditOp::Duplicate { start, len } => {
                self.duplicate(*start, *len);
                Ok(())
            }
            EditOp::Move { from, len, dest } => {
                self.move_range(*from, *len, *dest);
                Ok(())
            }
            EditOp::Roll { at, delta } => {
                self.roll(*at, *delta);
                Ok(())
            }
        }
    }

    /// `assert_invariants`.
    pub fn assert_invariants(&self) {
        let mut t = 0u64;
        for span in self.spans() {
            assert!(span.clip.len > 0, "zero-length clip");
            assert_eq!(span.start, t, "gap or overlap at {t}");
            t += span.clip.len;
        }
        assert_eq!(t, self.frames());
    }

    #[cfg(test)]
    fn replace_init_snapshot(&mut self, tree: ClipTree) {
        self.edl.replace_init_snapshot(tree);
    }
}

fn range_is_silence(tree: &ClipTree, start: u64, len: u64) -> bool {
    let end = start.saturating_add(len);
    let mut frame = start;
    while frame < end {
        let Some(span) = tree.at(frame) else {
            return false;
        };
        if span.clip.source.is_some() {
            return false;
        }
        frame = span.end().max(frame.saturating_add(1));
    }
    true
}

fn normalize_half_open_ranges(ranges: Vec<(u64, u64)>, frames: u64) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = ranges
        .into_iter()
        .filter_map(|(start, end)| {
            let start = start.min(frames);
            let end = end.min(frames);
            (end > start).then_some((start, end))
        })
        .collect();
    out.sort_by_key(|(start, _)| *start);
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in out {
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// Next chunk inside target ranges: `(range_start, chunk_end, read_pos)`.
pub(crate) fn next_chunk_in_ranges(
    ranges: &[(u64, u64)],
    read_pos: u64,
    max_frames: u64,
) -> Option<(u64, u64, u64)> {
    for &(start, end) in ranges {
        let pos = if read_pos < start {
            start
        } else if read_pos < end {
            read_pos
        } else {
            continue;
        };
        let chunk_end = pos.saturating_add(max_frames).min(end);
        if chunk_end > pos {
            return Some((start, chunk_end, pos));
        }
    }
    None
}

pub(crate) fn advance_read_pos(ranges: &[(u64, u64)], after: u64) -> u64 {
    for &(start, end) in ranges {
        if after < end {
            return after.max(start);
        }
    }
    after
}

pub(crate) fn peak_covered_samples(clip: &Clip) -> u64 {
    clip.cache
        .peaks
        .first()
        .map(|channel| channel.len() as u64 * PEAK_BLOCK as u64)
        .unwrap_or(0)
}

fn extend_clip_cache(
    tree: &ClipTree,
    id: ClipId,
    key: Option<(MediaId, u64, u64)>,
    bins: &[Vec<(f32, f32)>],
    chunk_min: f32,
    chunk_max: f32,
) -> ClipTree {
    tree.map_clips(|clip| {
        let same = clip.id == id || (key.is_some() && clip_cache_key(clip) == key);
        if !same {
            return clip.clone();
        }
        let mut clip = clip.clone();
        if clip.cache.peaks.is_empty() {
            clip.cache.peaks = bins.to_vec();
        } else {
            for (dst, src) in clip.cache.peaks.iter_mut().zip(bins.iter()) {
                dst.extend_from_slice(src);
            }
        }
        if chunk_min <= chunk_max {
            clip.cache.min = Some(clip.cache.min.unwrap_or(f32::MAX).min(chunk_min));
            clip.cache.max = Some(clip.cache.max.unwrap_or(f32::MIN).max(chunk_max));
        }
        clip
    })
}

fn clip_cache_key(clip: &Clip) -> Option<(MediaId, u64, u64)> {
    clip.source
        .as_ref()
        .map(|source| (source.media_id, source.offset, clip.len))
}

fn apply_clip_cache(tree: &ClipTree, peaked: &Clip) -> ClipTree {
    let peaked_key = clip_cache_key(peaked);
    tree.map_clips(|clip| {
        let same =
            clip.id == peaked.id || (peaked_key.is_some() && clip_cache_key(clip) == peaked_key);
        if !same {
            return clip.clone();
        }
        let mut clip = clip.clone();
        clip.cache = peaked.cache.clone();
        clip
    })
}

fn media_stats_match(media: &MediaRef, meta: &std::fs::Metadata) -> bool {
    if meta.len() != media.size_bytes {
        return false;
    }
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let stored = media
        .modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let actual = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    stored.as_secs() == actual.as_secs()
}

/// Build a [`MediaRef`] from a probed audio file (identity finalized).
pub fn media_ref_from_probed(probed: ProbedFile) -> MediaRef {
    let path = probed.path;
    let url = Location::from_path(&path);
    let mut media = MediaRef {
        id: MediaId([0u8; 32]),
        url,
        basename: String::new(),
        path,
        sample_rate: probed.sample_rate,
        channel_count: probed.channel_count,
        frame_count: probed.frame_count,
        bits_per_sample: probed.bits_per_sample,
        size_bytes: probed.size_bytes,
        modified: probed.modified,
        container_format: probed.container_format,
        codec: probed.codec,
        samples: probed.samples,
        availability: field_audio_model::MediaAvailability::Available,
    };
    media.finalize_identity();
    media
}

/// `is_facomp_path`.
pub fn is_facomp_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("facomp"))
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "composition.facomp".into());
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let tmp = match dir {
        Some(dir) => dir.join(format!(".{file_name}.tmp")),
        None => PathBuf::from(format!(".{file_name}.tmp")),
    };
    std::fs::write(&tmp, contents).with_context(|| format!("failed to write {}", tmp.display()))?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            tmp.display()
        )
    })
}

/// FramesIter.
pub struct FramesIter<'a> {
    composition: &'a Composition,
    pos: u64,
    buf: Vec<f32>,
}

impl Iterator for FramesIter<'_> {
    type Item = Vec<f32>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.composition.frames() {
            return None;
        }
        self.buf.fill(0.0);
        self.composition
            .read_interleaved(self.pos, 1, &mut self.buf)
            .ok()?;
        self.pos += 1;
        Some(self.buf.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MARKER_TYPE_BLUE;

    #[test]
    fn empty_composition_does_not_need_spectral_or_envelope() {
        let comp = Composition::new(44_100, 2);
        assert_eq!(comp.frames(), 0);
        assert!(!comp.needs_spectral_build());
        assert!(!comp.needs_envelope_peak_build());
        assert!(!comp.needs_peak_build());
    }

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

    fn materialize(comp: &Composition) -> Vec<Vec<f32>> {
        let frames = comp.frames() as usize;
        let mut planes = vec![vec![0.0; frames]; comp.channel_count()];
        if frames == 0 {
            return planes;
        }
        let mut refs: Vec<&mut [f32]> = planes.iter_mut().map(|p| p.as_mut_slice()).collect();
        comp.read_planar(0, frames as u64, &mut refs).unwrap();
        planes
    }

    #[test]
    fn from_media_reads_match_source() {
        let media = sine_media(64, 2, 44100);
        let expected = media.samples.as_ref().unwrap().clone();
        let comp = Composition::from_media(media).unwrap();
        comp.assert_invariants();
        assert_eq!(comp.frames(), 64);
        let got = materialize(&comp);
        assert_eq!(got, *expected);
        let clip = comp.clip_at(0).unwrap().clip;
        assert!(!clip.cache.peaks.is_empty());
    }

    #[test]
    fn unaligned_clear_keeps_right_peak_cache() {
        let block = PEAK_BLOCK as u64;
        let mut comp = Composition::from_media(sine_media((block * 4) as usize, 1, 44100)).unwrap();
        assert!(!comp.needs_peak_build());
        comp.clear(block + 7, 5);
        assert!(
            !comp.needs_peak_build(),
            "clear should reuse suffix peak bins instead of rebuilding"
        );
        let right = comp
            .spans()
            .into_iter()
            .rev()
            .find(|span| span.clip.source.is_some())
            .unwrap();
        assert!(!right.clip.cache.is_missing_peaks());
        let (min, max) =
            comp.min_max_in_range(0, right.start as f64, (right.start + right.clip.len) as f64);
        assert!(max > min);
    }

    #[test]
    fn remove_shrinks_clear_preserves_length() {
        let mut comp = Composition::from_media(sine_media(20, 1, 48000)).unwrap();
        comp.remove(5, 5);
        comp.assert_invariants();
        assert_eq!(comp.frames(), 15);
        comp.undo();
        comp.clear(5, 5);
        comp.assert_invariants();
        assert_eq!(comp.frames(), 20);
        let samples = materialize(&comp);
        assert!(samples[0][5..10].iter().all(|&s| s == 0.0));
        assert_ne!(samples[0][4], 0.0);
        assert_eq!(comp.modified_ranges(), vec![(5, 10)]);
        assert_eq!(comp.ranges_for_edit(comp.current_edit()), vec![(5, 10)]);
    }

    #[test]
    fn clear_is_idempotent_when_already_silence() {
        let mut comp = Composition::from_media(sine_media(20, 1, 48000)).unwrap();
        let before = comp.edits().len();
        comp.clear(5, 5);
        assert_eq!(comp.edits().len(), before + 1);
        let after_first = comp.edits().len();
        let cursor = comp.edit_cursor();
        let current = comp.current_edit();
        comp.clear(5, 5);
        assert_eq!(comp.edits().len(), after_first);
        assert_eq!(comp.edit_cursor(), cursor);
        assert_eq!(comp.current_edit(), current);
        let samples = materialize(&comp);
        assert!(samples[0][5..10].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn clear_mixed_audio_and_silence_still_commits() {
        let mut comp = Composition::from_media(sine_media(20, 1, 48000)).unwrap();
        comp.clear(5, 5);
        let after_first = comp.edits().len();
        // Spans existing silence [5,10) and audio [10,15).
        comp.clear(5, 10);
        assert_eq!(comp.edits().len(), after_first + 1);
        let samples = materialize(&comp);
        assert!(samples[0][5..15].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn cut_copy_paste_and_undo() {
        let mut comp = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        let original = materialize(&comp);
        comp.cut(4, 4);
        assert_eq!(comp.frames(), 12);
        comp.paste(0).unwrap();
        assert_eq!(comp.frames(), 16);
        let after_paste = materialize(&comp);
        assert_eq!(after_paste[0][..4], original[0][4..8]);
        assert!(comp.undo());
        assert_eq!(comp.frames(), 12);
        assert!(comp.redo());
        assert_eq!(comp.frames(), 16);
    }

    #[test]
    fn duplicate_trim_move_roll() {
        let mut comp = Composition::from_media(sine_media(10, 1, 44100)).unwrap();
        comp.duplicate(0, 3);
        assert_eq!(comp.frames(), 13);
        comp.trim(0, 6);
        assert_eq!(comp.frames(), 6);
        comp.move_range(0, 2, 6);
        assert_eq!(comp.frames(), 6);
        comp.assert_invariants();

        let mut rolled = Composition::from_media(sine_media(10, 1, 44100)).unwrap();
        rolled.trim(0, 8);
        rolled.roll(0, 1);
        rolled.assert_invariants();
        let clip = rolled.clip_at(0).unwrap();
        assert_eq!(clip.clip.source.as_ref().unwrap().offset, 1);
        rolled.roll(0, 100);
        assert_eq!(
            rolled
                .clip_at(0)
                .unwrap()
                .clip
                .source
                .as_ref()
                .unwrap()
                .offset,
            2
        );
    }

    #[test]
    fn jump_to_edit_restores_snapshot() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        let init = comp.current_edit();
        comp.remove(0, 2);
        let after = comp.current_edit();
        comp.clear(0, 2);
        assert!(comp.jump_to_edit(init));
        assert_eq!(comp.frames(), 8);
        assert!(comp.jump_to_edit(after));
        assert_eq!(comp.frames(), 6);
    }

    #[test]
    fn undo_to_init_keeps_original_media() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        let frames = comp.frames();
        let init = comp.current_edit();
        comp.remove(2, 4);
        assert_eq!(comp.frames(), 8);
        assert!(comp.undo());
        assert_eq!(comp.current_edit(), init);
        assert_eq!(comp.frames(), frames);
        assert!(comp.clip_at(0).is_some());
        assert!(!comp.clip_at(0).unwrap().clip.cache.peaks.is_empty());
    }

    #[test]
    fn new_composition_is_clean() {
        let comp = Composition::new(44100, 2);
        assert!(!comp.is_modified());
        let from_media = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(!from_media.is_modified());
    }

    #[test]
    fn edit_then_undo_to_init_is_clean() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(!comp.is_modified());
        comp.remove(2, 4);
        assert!(comp.is_modified());
        assert!(comp.undo());
        assert!(!comp.is_modified());
    }

    #[test]
    fn load_with_history_is_clean_until_edited() {
        let mut live = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        live.remove(2, 2);
        assert!(live.edit_cursor() > 0);
        let json = live.to_json().unwrap();
        let mut restored = Composition::from_json(&json).unwrap();
        assert!(restored.edit_cursor() > 0);
        assert!(!restored.is_modified());
        restored.remove(0, 1);
        assert!(restored.is_modified());
        assert!(restored.undo());
        assert!(!restored.is_modified());
    }

    #[test]
    fn marker_add_and_remove_are_dirty() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        let id = comp.add_marker(0, MARKER_TYPE_BLUE, None).unwrap();
        assert!(comp.is_modified());
        assert!(comp.remove_marker(id));
        assert!(!comp.is_modified());
        comp.add_marker(4, MARKER_TYPE_BLUE, None);
        assert!(comp.is_modified());
    }

    #[test]
    fn layout_monitor_and_marker_types_do_not_dirty() {
        use std::collections::BTreeMap;

        let mut comp = Composition::from_media(sine_media(12, 2, 44100)).unwrap();
        assert!(!comp.is_modified());
        let mut labels = BTreeMap::new();
        labels.insert(0, "L".into());
        labels.insert(1, "R".into());
        comp.choose_channel_layout(Some("stereo".into()), labels);
        comp.set_monitor_chain(Some("stereo".into()));
        comp.set_playback_channels(Some(vec![0]));
        assert!(comp.add_marker_type("Red", [1.0, 0.0, 0.0, 1.0]));
        assert!(!comp.is_modified());
        assert!(comp.ensure_collection("empty").is_some());
        assert!(!comp.is_modified());
    }

    #[test]
    fn named_regions_are_dirty() {
        use ChannelScope;

        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(!comp.is_modified());
        let id = comp
            .add_named_region("silent", 2, 5, ChannelScope::all(), Some("gap".into()))
            .expect("region");
        assert!(comp.is_modified());
        assert!(comp.collection_mut("silent").expect("silent").remove(id));
        assert!(!comp.is_modified());
    }

    #[test]
    fn save_clears_dirty() {
        let mut live = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        live.remove(2, 2);
        live.add_marker(0, MARKER_TYPE_BLUE, None);
        assert!(live.is_modified());
        let path = std::env::temp_dir().join("snd-composition-dirty-save.facomp");
        live.save_to_path(&path).unwrap();
        assert!(!live.is_modified());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn empty_init_snapshot_rebuilds_from_media() {
        let mut comp = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        let frames = comp.frames();
        let init = comp.current_edit();
        comp.remove(0, 4);
        comp.replace_init_snapshot(ClipTree::empty());
        assert!(comp.jump_to_edit(init));
        assert_eq!(comp.frames(), frames);
        assert!(!comp.is_empty());
        assert!(comp.clip_at(0).is_some());
    }

    #[test]
    fn paste_rejects_rate_mismatch() {
        let mut comp = Composition::new(48000, 2);
        comp.clipboard = Clipboard {
            sample_rate: 44100,
            channel_count: 2,
            clips: vec![Clip::silence(ClipId(1), 4)],
        };
        let err = comp.paste(0).unwrap_err();
        assert!(err.to_string().contains("clipboard"));
    }

    #[test]
    fn project_json_round_trip_skips_pcm() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        comp.remove(2, 2);
        let json = comp.to_json().unwrap();
        assert!(!json.contains("samples"));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["media"][0].get("samples").is_none());
        assert_eq!(value["kind"], "facomp");
        assert_eq!(value["format_version"], 9);
        let media = &value["media"][0];
        assert!(media.get("url").is_some());
        assert!(media.get("path").is_none());
        assert!(media.get("size_bytes").is_some());
        assert!(media.get("modified").is_some());
        assert_eq!(media["container_format"], "memory");
        assert_eq!(media["codec"], "pcm");
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.frames(), comp.frames());
        assert_eq!(restored.current_edit().0, comp.current_edit().0);
        restored.assert_invariants();
    }

    #[test]
    fn project_json_rejects_wrong_kind_and_future_version() {
        let err = Composition::from_json(
            r#"{"kind":"other","format_version":1,"sample_rate":1,"channel_count":1,"media":[],"initial":{"type":"empty"},"edits":[],"edit_cursor":0}"#,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("kind"));
        let err = Composition::from_json(r#"{"kind":"facomp"}"#)
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.contains("format_version")
                || err.contains("missing field")
                || err.contains("parse project JSON"),
            "{err}"
        );
        let err = Composition::from_json(
            r#"{"kind":"facomp","format_version":99,"sample_rate":1,"channel_count":1,"media":[],"initial":{"type":"empty"},"edits":[],"edit_cursor":0}"#,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("newer FieldAssist"));
    }

    #[test]
    fn suggested_facomp_name_uses_basename_stem() {
        let media = sine_media(4, 1, 44100);
        let mut media = media;
        media.path = std::path::PathBuf::from("take.wav");
        let comp = Composition::from_media(media).unwrap();
        assert_eq!(comp.suggested_facomp_name(), "take.facomp");
        assert_eq!(comp.display_name(), "take");
    }

    #[test]
    fn has_edits_false_until_user_edit() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        assert!(!comp.has_edits());
        comp.clear(0, 2);
        assert!(comp.has_edits());
        assert!(comp.undo());
        assert!(!comp.has_edits());
    }

    #[test]
    fn display_title_dirties_and_feeds_suggested_name() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        assert!(!comp.is_modified());
        comp.set_display_title("renamed take");
        assert!(comp.is_modified());
        assert_eq!(comp.display_name(), "renamed take");
        assert_eq!(comp.suggested_facomp_name(), "renamed take.facomp");
        comp.set_display_title("already.facomp");
        assert_eq!(comp.suggested_facomp_name(), "already.facomp");
        let path = std::env::temp_dir().join("snd-composition-display-title.facomp");
        comp.save_to_path(&path).unwrap();
        assert!(!comp.is_modified());
        assert!(comp.display_title().is_none());
        assert_eq!(comp.display_name(), "snd-composition-display-title");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn facomp_display_name_uses_project_path_stem() {
        let dir = std::env::temp_dir().join("fa-facomp-display-name");
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join("media-source.wav");
        write_minimal_wav(&wav, 8);
        let mut comp = Composition::from_media_path(&wav, None).unwrap();
        assert_eq!(comp.display_name(), "media-source");
        let facomp = dir.join("1-child-edit.facomp");
        comp.save_to_path(&facomp).unwrap();
        assert_eq!(comp.display_name(), "1-child-edit");
        let loaded = Composition::load_from_path(&facomp).unwrap();
        assert_eq!(loaded.display_name(), "1-child-edit");
        assert!(!loaded.is_modified());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frames_iter_matches_interleaved_read() {
        let comp = Composition::from_media(sine_media(5, 2, 44100)).unwrap();
        let mut all = vec![0.0; 10];
        comp.read_interleaved(0, 5, &mut all).unwrap();
        let collected: Vec<f32> = comp.frames_iter(0).flatten().collect();
        assert_eq!(collected, all);
    }

    #[test]
    fn load_from_wav_pages_off_disk() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
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
        let dir = std::env::temp_dir();
        let path = dir.join("snd-composition-page-test.wav");
        write_sine_wav(&path, 1, 512, 44100);
        let mut comp = Composition::load_from_path(&path).unwrap();
        assert_eq!(comp.frames(), 512);
        assert!(comp.pool().first().unwrap().samples.is_none());
        let mut dest = vec![0.0; 8];
        comp.read_interleaved(10, 8, &mut dest).unwrap();
        assert!(dest.iter().any(|&s| s.abs() > 0.01));
        assert!(comp.needs_peak_build());
        let progress = ProgressHandle::new();
        let epoch = progress.begin("building peaks");
        let updates = comp
            .build_missing_peak_caches(Some(&progress), epoch)
            .unwrap();
        assert!(!updates.is_empty());
        assert_eq!(
            progress.snapshot().unwrap().message(),
            "building peaks 100%"
        );
        comp.apply_peak_caches(updates);
        assert!(!comp.needs_peak_build());
        progress.finish(epoch);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shared_peak_build_fills_file_backed_caches() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
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
        let path = std::env::temp_dir().join("snd-composition-shared-peak-test.wav");
        write_sine_wav(&path, 1, 512, 44100);
        let lock = std::sync::RwLock::new(Composition::load_from_path(&path).unwrap());
        assert!(lock.read().unwrap().needs_peak_build());
        let progress = ProgressHandle::new();
        let epoch = progress.begin("building peaks");
        let outcome =
            Composition::build_missing_peak_caches_shared(&lock, Some(&progress), epoch).unwrap();
        assert_eq!(outcome, super::PeakBlockOutcome::Complete);
        assert!(!lock.read().unwrap().needs_peak_build());
        assert!(lock.read().unwrap().can_paint_overview());
        progress.finish(epoch);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn incremental_peak_block_paints_before_complete() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
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
        let frames = BLOCK_FRAMES as u32 + 512;
        let path = std::env::temp_dir().join("snd-composition-incremental-peak.wav");
        write_sine_wav(&path, 1, frames, 44100);
        let lock = std::sync::RwLock::new(Composition::load_from_path(&path).unwrap());
        assert!(lock.read().unwrap().needs_peak_build());
        let first = Composition::build_next_peak_block(&lock, None, 0).unwrap();
        assert_eq!(first, PeakBlockOutcome::Progress);
        assert!(lock.read().unwrap().can_paint_overview());
        assert!(lock.read().unwrap().needs_peak_build());
        let second = Composition::build_next_peak_block(&lock, None, 0).unwrap();
        assert_eq!(second, PeakBlockOutcome::Complete);
        assert!(!lock.read().unwrap().needs_peak_build());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn facomp_save_and_reload_reprobes_media() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
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
        let dir = std::env::temp_dir();
        let wav = dir.join("snd-composition-facomp-src.wav");
        let facomp = dir.join("snd-composition-facomp-src.wav.facomp");
        write_sine_wav(&wav, 1, 256, 44100);
        let mut live = Composition::load_from_path(&wav).unwrap();
        live.remove(10, 8);
        live.save_to_path(&facomp).unwrap();
        let json = std::fs::read_to_string(&facomp).unwrap();
        assert!(json.contains("\"kind\": \"facomp\""));
        assert!(!json.contains("samples"));
        let (restored, report) = Composition::load_facomp(&facomp).unwrap();
        assert!(report.is_ok());
        assert_eq!(restored.frames(), live.frames());
        assert_eq!(restored.sample_rate(), 44100);
        let pool = restored.pool();
        let media = pool.first().unwrap();
        assert_eq!(media.container_format, "wav");
        assert!(media.samples.is_none());
        let _ = std::fs::remove_file(wav);
        let _ = std::fs::remove_file(facomp);
    }

    #[test]
    fn facomp_reload_skips_audio_probe_when_stats_match() {
        let dummy = std::env::temp_dir().join("snd-facomp-not-audio.bin");
        std::fs::write(&dummy, b"not an audio file at all!!").unwrap();
        let meta = std::fs::metadata(&dummy).unwrap();
        let mut media = MediaRef {
            id: MediaId([0u8; 32]),
            url: Location::from_path(&dummy),
            basename: String::new(),
            path: dummy.clone(),
            sample_rate: 44100,
            channel_count: 1,
            frame_count: 1000,
            bits_per_sample: Some(16),
            size_bytes: meta.len(),
            modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            container_format: "wav".into(),
            codec: "pcm".into(),
            samples: None,
            availability: field_audio_model::MediaAvailability::Available,
        };
        media.finalize_identity();
        let media_id = media.id;
        let file = ProjectFile {
            id: None,
            parent: None,
            sample_rate: 44100,
            channel_count: 1,
            media: vec![media],
            initial: InitialState::FromMedia { media_id },
            edits: Vec::new(),
            edit_cursor: 0,
            undo_floor: 0,
            markers: Vec::new(),
            marker_types: Vec::new(),
            collections: Vec::new(),
            channel_layout: None,
            monitor_chain: None,
            playback_channels: None,
            source_channels: None,
        };
        let json = ProjectEnvelope::wrap(file).to_json().unwrap();
        let (comp, report) = Composition::from_json_reprobing(&json).unwrap();
        assert!(report.is_ok());
        assert_eq!(comp.frames(), 1000);
        assert!(comp.pool().first().unwrap().samples.is_none());
        let _ = std::fs::remove_file(dummy);
    }

    #[test]
    fn remove_matches_oracle_slice() {
        let media = sine_media(24, 1, 44100);
        let expected_src = media.samples.as_ref().unwrap()[0].clone();
        let mut comp = Composition::from_media(media).unwrap();
        comp.remove(6, 5);
        comp.assert_invariants();
        let mut expected = expected_src.clone();
        expected.drain(6..11);
        assert_eq!(materialize(&comp)[0], expected);
    }

    #[test]
    fn copy_does_not_change_timeline() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        let before = materialize(&comp);
        comp.copy(2, 3);
        assert_eq!(comp.frames(), 8);
        assert_eq!(materialize(&comp), before);
        assert_eq!(comp.clipboard().frames(), 3);
    }

    #[test]
    fn fade_in_scales_leading_frames() {
        let media = sine_media(8, 1, 44100);
        let original = media.samples.as_ref().unwrap()[0].clone();
        let mut comp = Composition::from_media(media).unwrap();
        let mut clip = (*comp.clip_at(0).unwrap().clip).clone();
        clip.fade_in = 4;
        let len = clip.len;
        comp.replace_range(0, len, vec![clip]);
        let got = materialize(&comp);
        assert_eq!(got[0][0], 0.0);
        assert!((got[0][2] - original[2] * 0.5).abs() < 1e-5);
        assert!((got[0][4] - original[4]).abs() < 1e-5);
    }

    #[test]
    fn json_replay_matches_live_snapshots() {
        let mut live = Composition::from_media(sine_media(20, 1, 44100)).unwrap();
        live.remove(2, 2);
        live.clear(0, 3);
        live.duplicate(0, 4);
        let json = live.to_json().unwrap();
        assert!(!json.contains("samples"));
        let restored = Composition::from_json(&json).unwrap();
        restored.assert_invariants();
        assert_eq!(restored.frames(), live.frames());
        assert_eq!(restored.current_edit().0, live.current_edit().0);
        let live_spans: Vec<_> = live
            .spans()
            .into_iter()
            .map(|s| (s.start, s.clip.len, s.clip.source.clone()))
            .collect();
        let restored_spans: Vec<_> = restored
            .spans()
            .into_iter()
            .map(|s| (s.start, s.clip.len, s.clip.source.clone()))
            .collect();
        assert_eq!(restored_spans, live_spans);
    }

    #[test]
    fn new_edit_drops_redo_tail() {
        let mut comp = Composition::from_media(sine_media(10, 1, 44100)).unwrap();
        comp.remove(0, 1);
        comp.remove(0, 1);
        assert!(comp.undo());
        assert!(comp.can_redo());
        comp.clear(0, 1);
        assert!(!comp.can_redo());
        assert_eq!(comp.frames(), 9);
    }

    #[test]
    fn zero_length_clips_are_dropped() {
        let mut comp = Composition::from_media(sine_media(6, 1, 44100)).unwrap();
        comp.replace_range(2, 2, vec![Clip::silence(ClipId(99), 0)]);
        comp.assert_invariants();
        assert_eq!(comp.frames(), 4);
        assert!(comp.spans().iter().all(|s| s.clip.len > 0));
    }

    #[test]
    fn markers_remap_through_cut_and_undo() {
        use crate::MARKER_TYPE_BLUE;

        let mut comp = Composition::from_media(sine_media(40, 1, 44100)).unwrap();
        comp.add_marker(5, MARKER_TYPE_BLUE, None).unwrap();
        comp.add_marker(15, MARKER_TYPE_BLUE, None).unwrap();
        comp.add_marker(30, MARKER_TYPE_BLUE, None).unwrap();
        comp.cut(10, 10);
        let frames: Vec<u64> = comp.markers().iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![5, 20]);
        assert!(comp.undo());
        let frames: Vec<u64> = comp.markers().iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![5, 30]);
    }

    #[test]
    fn project_json_v7_loads_without_markers() {
        let json = r#"{
            "kind":"facomp",
            "format_version":7,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0
        }"#;
        let restored = Composition::from_json(json).unwrap();
        assert!(restored.markers().is_empty());
        assert_eq!(restored.sample_rate(), 44100);
    }

    #[test]
    fn project_json_round_trip_keeps_markers() {
        use crate::{MARKER_TYPE_BLUE, MARKER_TYPE_YELLOW};

        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        comp.add_marker(3, MARKER_TYPE_BLUE, None).unwrap();
        comp.add_marker(8, MARKER_TYPE_YELLOW, Some("cue".into()))
            .unwrap();
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], 9);
        assert_eq!(value["markers"].as_array().unwrap().len(), 2);
        assert!(value["markers"][0].get("color").is_none());
        assert!(value["marker_types"].as_array().unwrap().len() >= 3);
        let restored = Composition::from_json(&json).unwrap();
        let markers: Vec<_> = restored.markers().iter().cloned().collect();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].frame, 3);
        assert_eq!(markers[0].marker_type, MARKER_TYPE_BLUE);
        assert_eq!(markers[1].frame, 8);
        assert_eq!(markers[1].note.as_deref(), Some("cue"));
    }

    #[test]
    fn project_json_v7_loads_without_collections() {
        let json = r#"{
            "kind":"facomp",
            "format_version":7,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0,
            "markers":[]
        }"#;
        let restored = Composition::from_json(json).unwrap();
        assert!(restored.collections().is_empty());
        assert_eq!(
            restored.marker_types().len(),
            field_audio_model::DEFAULT_MARKER_TYPES.len()
        );
        assert_eq!(restored.sample_rate(), 44100);
    }

    #[test]
    fn project_json_round_trip_keeps_collections_and_marker_types() {
        use ChannelScope;

        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(comp.add_marker_type("Red", [1.0, 0.0, 0.0, 1.0]));
        comp.add_named_region("silent", 2, 5, ChannelScope::all(), Some("gap".into()))
            .unwrap();
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], 9);
        assert_eq!(value["collections"].as_array().unwrap().len(), 1);
        assert!(value["marker_types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|ty| ty["name"] == "Red"));
        let restored = Composition::from_json(&json).unwrap();
        assert!(restored.marker_types().iter().any(|ty| ty.name == "Red"));
        let silent = restored.collection("silent").expect("silent");
        assert_eq!(silent.regions[0].label.as_deref(), Some("gap"));
        assert_eq!(silent.regions[0].start, 2);
        assert_eq!(silent.regions[0].end, 5);
    }

    #[test]
    fn transient_analysis_respects_target_ranges() {
        use crate::MARKER_TYPE_TRANSIENT;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = 8_000usize;
        let click_at = 5_000usize;
        let mut samples = vec![0.0f32; frames];
        for s in &mut samples[click_at..click_at + 40] {
            *s = 1.0;
        }
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        {
            let mut comp = lock.write().unwrap();
            // Selection-only window that includes the click, not the whole file.
            comp.begin_analysis_target(Some(vec![(4_000, 6_000)]));
        }
        loop {
            match Composition::build_next_analysis_block(&lock, AnalysisKind::Transients, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
        let markers: Vec<u64> = lock
            .read()
            .unwrap()
            .markers()
            .iter()
            .filter(|m| m.marker_type == MARKER_TYPE_TRANSIENT)
            .map(|m| m.frame)
            .collect();
        assert!(
            !markers.is_empty(),
            "expected transient markers inside the selection"
        );
        assert!(
            markers.iter().all(|f| (4_000..6_000).contains(f)),
            "markers should stay inside the target range: {markers:?}"
        );
    }

    #[test]
    fn transient_analysis_detects_onset_at_range_start() {
        use crate::MARKER_TYPE_TRANSIENT;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = 8_000usize;
        let click_at = 4_000usize;
        let mut samples = vec![0.0f32; frames];
        for s in &mut samples[click_at..click_at + 40] {
            *s = 1.0;
        }
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        {
            let mut comp = lock.write().unwrap();
            // Range starts on the click; pre-roll silence lives outside the target.
            comp.begin_analysis_target(Some(vec![(4_000, 6_000)]));
        }
        loop {
            match Composition::build_next_analysis_block(&lock, AnalysisKind::Transients, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
        let markers: Vec<u64> = lock
            .read()
            .unwrap()
            .markers()
            .iter()
            .filter(|m| m.marker_type == MARKER_TYPE_TRANSIENT)
            .map(|m| m.frame)
            .collect();
        assert!(
            !markers.is_empty(),
            "expected a transient at the selection start"
        );
    }

    #[test]
    fn transient_analysis_replaces_only_target_range_on_complete() {
        use crate::MARKER_TYPE_TRANSIENT;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = 8_000usize;
        let mut samples = vec![0.0f32; frames];
        for s in &mut samples[5_000..5_040] {
            *s = 1.0;
        }
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        {
            let mut comp = lock.write().unwrap();
            comp.add_marker(100, MARKER_TYPE_TRANSIENT, None).unwrap();
            comp.add_marker(5_010, MARKER_TYPE_TRANSIENT, None).unwrap();
            comp.begin_analysis_target(Some(vec![(4_000, 6_000)]));
        }
        loop {
            match Composition::build_next_analysis_block(&lock, AnalysisKind::Transients, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
        let markers: Vec<u64> = lock
            .read()
            .unwrap()
            .markers()
            .iter()
            .filter(|m| m.marker_type == MARKER_TYPE_TRANSIENT)
            .map(|m| m.frame)
            .collect();
        assert!(
            markers.contains(&100),
            "markers outside the target must survive: {markers:?}"
        );
        assert!(
            markers.iter().any(|f| (4_000..6_000).contains(f)),
            "expected a detection inside the target: {markers:?}"
        );
    }

    #[test]
    fn spectral_analysis_covers_timeline_and_is_non_flat() {
        use field_audio_process::SPECTRAL_BAND_COUNT;
        use std::f32::consts::PI;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = BLOCK_FRAMES + 1_024;
        let freq = 1_000.0f32;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * freq * i as f32 / rate as f32).sin())
            .collect();
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        assert!(lock.read().unwrap().needs_spectral_build());
        let first =
            Composition::build_next_analysis_block(&lock, AnalysisKind::Spectral, None, 0).unwrap();
        assert_eq!(first, AnalysisBlockOutcome::Progress);
        {
            let comp = lock.read().unwrap();
            assert!(
                comp.spectral_has_data(),
                "first block should expose progressive coverage"
            );
            assert!(comp.needs_spectral_build());
        }
        loop {
            match Composition::build_next_analysis_block(&lock, AnalysisKind::Spectral, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
        let comp = lock.read().unwrap();
        assert!(!comp.needs_spectral_build());
        let series = comp.analysis_streams().spectral().expect("spectral series");
        assert_eq!(series.band_count, SPECTRAL_BAND_COUNT);
        assert!(series.hop_count(0) > 0);
        let data = &series.channels[0];
        let max = data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min = data.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            max > min + 1.0,
            "expected spectral contrast, min={min} max={max}"
        );
        let mut columns = vec![0.0f32; 8 * SPECTRAL_BAND_COUNT];
        comp.fill_spectral_columns(0, 0.0, 256.0, &mut columns);
        assert!(columns.iter().any(|&v| v > SPECTRAL_DB_FLOOR + 1.0));
    }

    fn run_spectral_to_complete(lock: &std::sync::RwLock<Composition>) {
        loop {
            match Composition::build_next_analysis_block(lock, AnalysisKind::Spectral, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
    }

    #[test]
    fn spectral_delete_keeps_distant_hops_and_scopes_rebuild() {
        use field_audio_process::{SPECTRAL_BAND_COUNT, SPECTRAL_FFT_SIZE};
        use std::f32::consts::PI;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = (BLOCK_FRAMES as usize) * 2 + 4_096;
        let freq = 1_000.0f32;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * freq * i as f32 / rate as f32).sin())
            .collect();
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        run_spectral_to_complete(&lock);

        let hop = PEAK_BLOCK;
        let bands = SPECTRAL_BAND_COUNT;
        let far_hop = 2usize;
        let far_value = {
            let series = lock
                .read()
                .unwrap()
                .analysis_streams()
                .spectral()
                .unwrap()
                .clone();
            series.channels[0][far_hop * bands]
        };

        let delete_start = frames as u64 / 2;
        let delete_len = 500u64;
        {
            let mut comp = lock.write().unwrap();
            comp.clear(delete_start, delete_len);
            assert!(
                comp.spectral_has_data(),
                "edit must not wipe spectral stream"
            );
            assert!(comp.needs_spectral_build());
            let series = comp.analysis_streams().spectral().unwrap();
            assert_eq!(series.channels[0][far_hop * bands], far_value);
            let radius = (SPECTRAL_FFT_SIZE - PEAK_BLOCK) as u64;
            let dirty_total: u64 = series
                .dirty_ranges
                .iter()
                .map(|(s, e)| e.saturating_sub(*s))
                .sum();
            let expected_max = delete_len + 2 * radius + hop as u64 * 2;
            assert!(
                dirty_total <= expected_max,
                "dirty {dirty_total} should be edit-sized (≤ {expected_max})"
            );
        }

        // First job block configures the target from dirty ranges.
        let first =
            Composition::build_next_analysis_block(&lock, AnalysisKind::Spectral, None, 0).unwrap();
        assert_ne!(first, AnalysisBlockOutcome::Cancelled);
        {
            let comp = lock.read().unwrap();
            let radius = (SPECTRAL_FFT_SIZE - PEAK_BLOCK) as u64;
            let expected_max = delete_len + 2 * radius + hop as u64 * 4;
            assert!(
                comp.analysis_target_total() <= expected_max,
                "target {} should be edit-sized (≤ {})",
                comp.analysis_target_total(),
                expected_max
            );
            assert!(
                comp.analysis_target_total() < frames as u64 / 2,
                "must not rescan the whole composition"
            );
        }
        run_spectral_to_complete(&lock);
        let comp = lock.read().unwrap();
        assert!(!comp.needs_spectral_build());
        let series = comp.analysis_streams().spectral().unwrap();
        assert_eq!(series.channels[0][far_hop * bands], far_value);
    }

    #[test]
    fn spectral_cut_keeps_data_and_marks_join_dirty() {
        use field_audio_process::SPECTRAL_BAND_COUNT;
        use std::f32::consts::PI;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = (BLOCK_FRAMES as usize) + 8_192;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * 440.0 * i as f32 / rate as f32).sin())
            .collect();
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        run_spectral_to_complete(&lock);

        let bands = SPECTRAL_BAND_COUNT;
        let far_hop = 1usize;
        let far_value = lock
            .read()
            .unwrap()
            .analysis_streams()
            .spectral()
            .unwrap()
            .channels[0][far_hop * bands];

        {
            let mut comp = lock.write().unwrap();
            // Unaligned cut mid-timeline.
            comp.cut(3_100, 800);
            assert!(comp.spectral_has_data());
            assert!(comp.needs_spectral_build());
            let series = comp.analysis_streams().spectral().unwrap();
            assert_eq!(series.channels[0][far_hop * bands], far_value);
            assert!(!series.dirty_ranges.is_empty());
        }
        run_spectral_to_complete(&lock);
        assert!(!lock.read().unwrap().needs_spectral_build());
    }

    #[test]
    fn spectral_paste_inserts_dirty_hole() {
        use std::f32::consts::PI;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = (BLOCK_FRAMES as usize) + 4_096;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * 880.0 * i as f32 / rate as f32).sin())
            .collect();
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        run_spectral_to_complete(&lock);

        {
            let mut comp = lock.write().unwrap();
            comp.copy(0, 1_024);
            let before = comp.frames();
            comp.paste(before / 2).unwrap();
            assert!(comp.frames() > before);
            assert!(comp.spectral_has_data());
            assert!(comp.needs_spectral_build());
            let series = comp.analysis_streams().spectral().unwrap();
            let dirty: u64 = series
                .dirty_ranges
                .iter()
                .map(|(s, e)| e.saturating_sub(*s))
                .sum();
            assert!(dirty > 0);
            assert!(
                dirty < before,
                "paste dirty should not cover the whole file"
            );
        }
        run_spectral_to_complete(&lock);
        assert!(!lock.read().unwrap().needs_spectral_build());
    }

    #[test]
    fn spectral_jump_to_earlier_edit_keeps_distant_hops() {
        use field_audio_process::SPECTRAL_BAND_COUNT;
        use std::f32::consts::PI;
        use std::sync::RwLock;

        let rate = 8_000u32;
        let frames = (BLOCK_FRAMES as usize) * 2 + 4_096;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * 660.0 * i as f32 / rate as f32).sin())
            .collect();
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        run_spectral_to_complete(&lock);

        let bands = SPECTRAL_BAND_COUNT;
        let far_hop = 2usize;
        let (init_id, far_value) = {
            let comp = lock.read().unwrap();
            let init = comp.edits()[0].id;
            let far = comp.analysis_streams().spectral().unwrap().channels[0][far_hop * bands];
            (init, far)
        };

        {
            let mut comp = lock.write().unwrap();
            let mid = comp.frames() / 2;
            comp.clear(mid, 400);
        }
        run_spectral_to_complete(&lock);
        {
            let mut comp = lock.write().unwrap();
            let mid2 = comp.frames() / 2;
            comp.clear(mid2 + 2_000, 300);
        }
        run_spectral_to_complete(&lock);

        {
            let mut comp = lock.write().unwrap();
            assert!(comp.jump_to_edit(init_id));
            assert_eq!(comp.frames(), frames as u64);
            assert!(
                comp.spectral_has_data(),
                "history jump must not wipe the spectral stream"
            );
            let series = comp.analysis_streams().spectral().unwrap();
            assert_eq!(series.channels[0][far_hop * bands], far_value);
            // Inverse deletes re-dirty the restored neighborhoods; rebuild is
            // still regional, not a full-timeline wipe.
            assert!(comp.needs_spectral_build());
            let dirty: u64 = series
                .dirty_ranges
                .iter()
                .map(|(s, e)| e.saturating_sub(*s))
                .sum();
            assert!(
                dirty < frames as u64 / 2,
                "jump dirty {dirty} should stay well below half the timeline"
            );
        }
        run_spectral_to_complete(&lock);
        assert!(!lock.read().unwrap().needs_spectral_build());
    }

    #[test]
    fn envelope_delete_keeps_distant_bins() {
        use std::sync::RwLock;

        let rate = 1_000u32;
        let frames = 8_000usize;
        let mut samples = vec![0.0f32; frames];
        samples[100] = 1.0;
        samples[4_000] = 1.0;
        let media = MediaRef::from_memory_samples(rate, vec![samples]);
        let lock = RwLock::new(Composition::from_media(media).unwrap());
        loop {
            match Composition::build_next_analysis_block(&lock, AnalysisKind::EnvelopePeak, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
        let far_bin = 0usize;
        let far_value = lock
            .read()
            .unwrap()
            .analysis_streams()
            .float(AnalysisKind::EnvelopePeak)
            .unwrap()
            .channels[0][far_bin];

        {
            let mut comp = lock.write().unwrap();
            comp.clear(3_500, 200);
            assert!(comp.envelope_peak_has_data());
            assert!(comp.needs_envelope_peak_build());
            let series = comp
                .analysis_streams()
                .float(AnalysisKind::EnvelopePeak)
                .unwrap();
            assert_eq!(series.channels[0][far_bin], far_value);
            assert!(!series.dirty_ranges.is_empty());
        }
        loop {
            match Composition::build_next_analysis_block(&lock, AnalysisKind::EnvelopePeak, None, 0)
                .unwrap()
            {
                AnalysisBlockOutcome::Progress => {}
                AnalysisBlockOutcome::Complete => break,
                AnalysisBlockOutcome::Cancelled => panic!("unexpected cancel"),
            }
        }
        assert!(!lock.read().unwrap().needs_envelope_peak_build());
    }

    #[test]
    fn project_json_round_trip_keeps_chosen_channel_layout() {
        use std::collections::BTreeMap;

        let mut comp = Composition::from_media(sine_media(12, 2, 44100)).unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(0, "M".into());
        labels.insert(1, "S".into());
        comp.choose_channel_layout(Some("MS".into()), labels);
        assert!(!comp.is_modified());
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["channel_layout"], "MS");
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.chosen_channel_layout(), Some("MS"));
        assert!(restored.channel_layout().is_none());
        assert!(!restored.is_modified());
    }

    #[test]
    fn project_json_omits_unset_channel_layout() {
        let comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("channel_layout").is_none());
        assert!(value.get("monitor_chain").is_none());
        assert!(value.get("playback_channels").is_none());
    }

    #[test]
    fn project_json_round_trip_keeps_monitor_routing() {
        let mut comp = Composition::from_media(sine_media(12, 6, 44100)).unwrap();
        comp.set_monitor_chain(Some("foa".into()));
        comp.set_playback_channels(Some(vec![0, 1, 2, 3]));
        assert!(!comp.is_modified());
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["monitor_chain"], "foa");
        assert_eq!(value["playback_channels"], serde_json::json!([0, 1, 2, 3]));
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.monitor_chain(), Some("foa"));
        assert_eq!(restored.playback_channels(), Some(&[0, 1, 2, 3][..]));
        assert!(!restored.is_modified());
        restored.assert_invariants();
    }

    #[test]
    fn playback_channels_all_clears_to_default() {
        let mut comp = Composition::from_media(sine_media(8, 2, 44100)).unwrap();
        comp.set_playback_channels(Some(vec![0, 1]));
        assert!(comp.playback_channels().is_none());
        comp.set_playback_channels(Some(vec![5, 1, 1]));
        assert_eq!(comp.playback_channels(), Some(&[1][..]));
    }

    #[test]
    fn project_json_v7_instance_color_becomes_marker_type() {
        let json = r#"{
            "kind":"facomp",
            "format_version":7,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0,
            "markers":[
                {"id":1,"frame":10,"type":"Red","color":[1.0,0.0,0.0,1.0],"note":"hit"}
            ]
        }"#;
        let restored = Composition::from_json(json).unwrap();
        let marker = restored.markers().iter().next().expect("marker");
        assert_eq!(marker.frame, 10);
        assert_eq!(marker.marker_type, "Red");
        assert_eq!(marker.note.as_deref(), Some("hit"));
        assert_eq!(restored.resolved_marker_color("Red"), [1.0, 0.0, 0.0, 1.0]);
        assert!(restored
            .marker_types()
            .iter()
            .any(|ty| ty.name == "Red" && ty.color == [1.0, 0.0, 0.0, 1.0]));
        let saved: serde_json::Value = serde_json::from_str(&restored.to_json().unwrap()).unwrap();
        assert!(saved["markers"][0].get("color").is_none());
        assert!(saved["marker_types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|ty| ty["name"] == "Red"));
    }

    #[test]
    fn break_out_shares_pager_and_sets_parent() {
        let parent = Composition::from_media(sine_media(48, 1, 44100)).unwrap();
        let parent_id = parent.id();
        let child = parent.break_out(10, 12).unwrap();
        assert_eq!(child.frames(), 12);
        assert_eq!(child.parent_id(), Some(parent_id));
        assert_ne!(child.id(), parent_id);
        assert!(child.shares_pager_with(&parent));
        assert!(child.is_modified());
        assert!(!child.can_undo());
        let json = child.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], 9);
        assert_eq!(value["parent"], parent_id.to_string());
        assert_eq!(value["id"], child.id().to_string());
        assert_eq!(value["edits"][0]["type"], "trim");
        assert_eq!(value["edits"][0]["start"], 10);
        assert_eq!(value["edits"][0]["len"], 12);
        assert_eq!(value["undo_floor"], 1);
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.frames(), 12);
        assert_eq!(restored.parent_id(), Some(parent_id));
        assert_eq!(restored.id(), child.id());
        assert!(!restored.can_undo());
        restored.assert_invariants();
    }

    #[test]
    fn break_out_channels_maps_reads_and_composes() {
        let parent = Composition::from_media(sine_media(32, 4, 44100)).unwrap();
        let expected = materialize(&parent);
        let child = parent
            .break_out_channels(&[(4, 12)], Some(&[0, 2]))
            .unwrap();
        assert_eq!(child.frames(), 12);
        assert_eq!(child.channel_count(), 2);
        assert_eq!(child.source_channels(), Some(&[0usize, 2][..]));
        assert_eq!(child.parent_id(), Some(parent.id()));
        assert!(!child.can_undo());
        let got = materialize(&child);
        assert_eq!(got[0], expected[0][4..16]);
        assert_eq!(got[1], expected[2][4..16]);

        let mut parent2 = Composition::from_media(sine_media(16, 4, 44100)).unwrap();
        parent2.apply_channel_layout(Some("test".into()), {
            let mut m = BTreeMap::new();
            m.insert(0, "W".into());
            m.insert(1, "X".into());
            m.insert(2, "Y".into());
            m.insert(3, "Z".into());
            m
        });
        let mid = parent2.break_out_channels(&[], Some(&[0, 2, 3])).unwrap();
        assert_eq!(mid.channel_count(), 3);
        assert_eq!(mid.source_channels(), Some(&[0usize, 2, 3][..]));
        assert_eq!(mid.channel_label(0), "W");
        assert_eq!(mid.channel_label(1), "Y");
        assert_eq!(mid.channel_label(2), "Z");
        assert!(mid.chosen_channel_layout().is_none());

        let nested = mid.break_out_channels(&[], Some(&[0, 2])).unwrap();
        assert_eq!(nested.channel_count(), 2);
        assert_eq!(nested.source_channels(), Some(&[0usize, 3][..]));
        assert_eq!(nested.parent_id(), Some(mid.id()));

        let json = nested.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], 9);
        assert_eq!(value["channel_count"], 2);
        assert_eq!(value["source_channels"], serde_json::json!([0, 3]));
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.source_channels(), Some(&[0usize, 3][..]));
        assert_eq!(restored.channel_count(), 2);
    }

    #[test]
    fn format8_load_keeps_identity_source_channels() {
        let parent = Composition::from_media(sine_media(8, 2, 44100)).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_str(&parent.to_json().unwrap()).unwrap();
        value["format_version"] = serde_json::json!(8);
        value.as_object_mut().unwrap().remove("source_channels");
        let restored = Composition::from_json(&value.to_string()).unwrap();
        assert!(restored.source_channels().is_none());
        assert_eq!(restored.channel_count(), 2);
    }

    #[test]
    fn break_out_nested_replays_two_trims() {
        let root = Composition::from_media(sine_media(100, 1, 44100)).unwrap();
        let child = root.break_out(20, 40).unwrap();
        let grand = child.break_out(5, 10).unwrap();
        assert_eq!(grand.frames(), 10);
        assert_eq!(child.parent_id(), Some(root.id()));
        assert_eq!(grand.parent_id(), Some(child.id()));
        let json = grand.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["edits"].as_array().unwrap().len(), 2);
        assert_eq!(value["edits"][0]["type"], "trim");
        assert_eq!(value["edits"][1]["type"], "trim");
        assert_eq!(value["parent"], child.id().to_string());
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.frames(), 10);
        assert_eq!(restored.parent_id(), Some(child.id()));
        assert!(!restored.can_undo());
    }

    #[test]
    fn break_out_founding_trim_not_undoable_after_edit() {
        let parent = Composition::from_media(sine_media(32, 1, 44100)).unwrap();
        let mut child = parent.break_out(4, 16).unwrap();
        assert!(!child.can_undo());
        child.remove(0, 2);
        assert!(child.can_undo());
        assert!(child.undo());
        assert_eq!(child.frames(), 16);
        assert!(!child.can_undo());
    }

    #[test]
    fn break_out_founding_trim_excluded_from_change_bars() {
        let parent = Composition::from_media(sine_media(32, 1, 44100)).unwrap();
        let mut child = parent.break_out(4, 16).unwrap();
        assert!(
            child.modified_ranges().is_empty(),
            "founding Trim must not paint a full-timeline change bar"
        );
        child.clear(2, 4);
        assert_eq!(child.modified_ranges(), vec![(2, 6)]);
        assert!(child.ranges_for_edit(child.edits()[1].id).is_empty());
    }

    #[test]
    fn legacy_v7_without_id_mints_id_and_is_dirty() {
        let json = r#"{
            "kind":"facomp",
            "format_version":7,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0
        }"#;
        let restored = Composition::from_json(json).unwrap();
        assert!(restored.is_modified());
        let saved: serde_json::Value = serde_json::from_str(&restored.to_json().unwrap()).unwrap();
        assert_eq!(saved["format_version"], 9);
        assert!(saved.get("id").is_some());
    }

    #[test]
    fn mint_new_id_keeps_parent() {
        let parent = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        let mut child = parent.break_out(2, 8).unwrap();
        let old = child.id();
        let parent_id = child.parent_id();
        child.mint_new_id();
        assert_ne!(child.id(), old);
        assert_eq!(child.parent_id(), parent_id);
        assert!(child.is_modified());
    }

    #[test]
    fn adopt_shared_media_rebinds_pager() {
        let parent = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        let mut child = parent.break_out(0, 8).unwrap();
        let other = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        child.adopt_shared_media(&other);
        assert!(child.shares_pager_with(&other));
        assert!(!child.shares_pager_with(&parent));
    }

    fn write_minimal_wav(path: &std::path::Path, frames: u32) {
        use std::io::Write;
        let bits_per_sample: u16 = 16;
        let channels: u16 = 1;
        let sample_rate: u32 = 44100;
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
        for _ in 0..frames {
            out.write_all(&0i16.to_le_bytes()).unwrap();
        }
    }

    fn write_sine_wav_i16(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
        use std::f32::consts::TAU;
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
            let sample = (0.5 * (TAU * 440.0 * t).sin() * i16::MAX as f32) as i16;
            for _ in 0..channels {
                out.write_all(&sample.to_le_bytes()).unwrap();
            }
        }
    }

    #[test]
    fn from_media_path_without_session_store() {
        let path = std::env::temp_dir().join("snd-from-media-path-none.wav");
        write_minimal_wav(&path, 8);
        let comp = Composition::from_media_path(&path, None).unwrap();
        assert_eq!(comp.frames(), 8);
        assert_eq!(comp.pool().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn from_media_path_cannot_decode_without_spill_dir() {
        let path = std::env::temp_dir().join("snd-from-media-no-decoder.wav");
        write_sine_wav_i16(&path, 2, 512, 44_100);
        let comp = Composition::from_media_path(&path, None).unwrap();
        let mut buf = vec![0.0f32; 256 * comp.channel_count()];
        let err = comp
            .read_interleaved(0, 256, &mut buf)
            .expect_err("null block source must reject file-backed reads");
        assert!(
            err.to_string().contains("no block decoder"),
            "unexpected error: {err:#}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_from_path_installs_decoder_and_reads_pcm() {
        let path = std::env::temp_dir().join("snd-load-from-path-decoder.wav");
        write_sine_wav_i16(&path, 2, 512, 44_100);
        let (comp, _) = Composition::load_from_path_with_warnings(&path).unwrap();
        let mut buf = vec![0.0f32; 256 * comp.channel_count()];
        comp.read_interleaved(0, 256, &mut buf).expect("decode");
        let peak = buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(peak > 0.1, "expected sine PCM, peak={peak}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shared_store_dedups_media_and_pager() {
        let path = std::env::temp_dir().join("snd-shared-store-dedup.wav");
        write_minimal_wav(&path, 8);
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let a = Composition::from_media_path(&path, Some(Arc::clone(&store))).unwrap();
        let b = Composition::from_media_path(&path, Some(Arc::clone(&store))).unwrap();
        assert!(Arc::ptr_eq(&a.media_store(), &b.media_store()));
        assert!(Arc::ptr_eq(&a.pager_arc(), &b.pager_arc()));
        assert_eq!(store.lock().unwrap().pool().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn from_descriptor_missing_media_reports_without_bail() {
        let dir = std::env::temp_dir().join("fa-from-descriptor-missing");
        let _ = std::fs::create_dir_all(&dir);
        let absolute = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        let mut descriptor = absolute.pool().first().unwrap().to_descriptor();
        descriptor.url = "missing-clip.wav".into();
        descriptor.basename = "missing-clip.wav".into();
        let (comp, report) =
            Composition::from_descriptor_with_base(descriptor, Some(&dir), None).unwrap();
        assert!(!report.is_ok());
        assert_eq!(report.problems.len(), 1);
        assert_eq!(
            report.problems[0].category,
            field_core::ProblemCategory::MissingReference
        );
        assert_eq!(
            comp.pool().first().unwrap().availability,
            MediaAvailability::Missing
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_descriptor_resolves_relative_url_with_base() {
        let dir = std::env::temp_dir().join("fa-from-descriptor-rel");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("rel-clip.wav");
        write_minimal_wav(&path, 12);
        let absolute = Composition::from_media_path(&path, None).unwrap();
        let mut descriptor = absolute.pool().first().unwrap().to_descriptor();
        // Simulate session persistence: relative URL beside the session file.
        descriptor.url = "rel-clip.wav".into();
        let (comp, report) =
            Composition::from_descriptor_with_base(descriptor, Some(&dir), None).unwrap();
        assert!(report.is_ok());
        assert_eq!(comp.frames(), 12);
        assert_eq!(comp.display_name(), "rel-clip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_store_keeps_per_composition_display_names() {
        let dir = std::env::temp_dir().join("fa-shared-display-names");
        let _ = std::fs::create_dir_all(&dir);
        let a_path = dir.join("alpha-take.wav");
        let b_path = dir.join("beta-take.wav");
        write_minimal_wav(&a_path, 8);
        write_minimal_wav(&b_path, 16);
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let a = Composition::from_media_path(&a_path, Some(Arc::clone(&store))).unwrap();
        let b = Composition::from_media_path(&b_path, Some(Arc::clone(&store))).unwrap();
        assert_eq!(store.lock().unwrap().pool().len(), 2);
        assert_eq!(a.pool().len(), 1);
        assert_eq!(b.pool().len(), 1);
        assert_eq!(a.display_name(), "alpha-take");
        assert_eq!(b.display_name(), "beta-take");
        assert_ne!(a.pool().first().unwrap().id, b.pool().first().unwrap().id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn facomp_v6_is_rejected() {
        let err = Composition::from_json(
            r#"{"kind":"facomp","format_version":6,"sample_rate":44100,"channel_count":1,"media":[],"initial":{"type":"empty"},"edits":[],"edit_cursor":0}"#,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("unsupported format_version 6"), "{err}");
    }

    #[test]
    fn facomp_v8_initial_records_media_id() {
        let comp = Composition::from_media(sine_media(6, 1, 44100)).unwrap();
        let media_id = comp.pool().first().unwrap().id;
        let value: serde_json::Value = serde_json::from_str(&comp.to_json().unwrap()).unwrap();
        assert_eq!(value["format_version"], 9);
        assert_eq!(value["initial"]["type"], "from_media");
        assert_eq!(value["initial"]["media_id"], media_id.to_string());
        assert_eq!(value["media"].as_array().unwrap().len(), 1);
        assert_eq!(value["media"][0]["id"], media_id.to_string());
    }

    #[test]
    fn facomp_v8_clear_writes_clear_tag() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        comp.clear(2, 3);
        let value: serde_json::Value = serde_json::from_str(&comp.to_json().unwrap()).unwrap();
        assert_eq!(value["format_version"], 9);
        let edits = value["edits"].as_array().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["type"], "clear");
        assert_eq!(edits[0]["start"], 2);
        assert_eq!(edits[0]["len"], 3);
    }

    #[test]
    fn facomp_v7_delete_tag_loads_as_clear() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        comp.clear(2, 3);
        let mut value: serde_json::Value = serde_json::from_str(&comp.to_json().unwrap()).unwrap();
        value["format_version"] = serde_json::json!(7);
        value["edits"][0]["type"] = serde_json::json!("delete");
        let restored = Composition::from_json(&value.to_string()).unwrap();
        assert_eq!(restored.edits().len(), 2);
        assert!(matches!(
            restored.edits()[1].op,
            EditOp::Clear { start: 2, len: 3 }
        ));
        let silence = restored
            .spans()
            .into_iter()
            .find(|span| span.contains(2))
            .expect("span covering cleared range");
        assert!(silence.clip.source.is_none());
        assert!(silence.end() >= 5);
    }

    #[test]
    fn save_writes_only_timeline_media() {
        let store = Arc::new(Mutex::new(MediaStore::in_memory()));
        let used = sine_media(8, 1, 44100);
        let used_id = used.id;
        let unused = sine_media(12, 1, 44100);
        store.lock().unwrap().intern(unused);
        let comp = Composition::from_media_with_store(used, Some(Arc::clone(&store))).unwrap();
        assert_eq!(store.lock().unwrap().pool().len(), 2);
        let value: serde_json::Value = serde_json::from_str(&comp.to_json().unwrap()).unwrap();
        assert_eq!(value["media"].as_array().unwrap().len(), 1);
        assert_eq!(value["media"][0]["id"], used_id.to_string());
    }
}
