// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};

use super::media::{MediaId, MediaPool, MediaRef};

/// Decodes a frame range from a media file path for the block pager.
pub trait BlockSource: Send + Sync {
    /// Decode `count` frames starting at `start` from `path` as planar f32.
    fn decode_range(&self, path: &Path, start: u64, count: u64) -> Result<Vec<Vec<f32>>>;
}

/// Block source that never decodes files (memory-only media / tests).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullBlockSource;

impl BlockSource for NullBlockSource {
    fn decode_range(&self, path: &Path, _start: u64, _count: u64) -> Result<Vec<Vec<f32>>> {
        bail!("no block decoder configured for {}", path.display())
    }
}

/// Frames per cached block.
pub const BLOCK_FRAMES: u64 = 65_536;
/// Decoded sample cache budget. Large enough for smooth playback and
/// overview paints without keeping a whole file in RAM.
pub const RAM_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// Cumulative pager cache / decode counters (resettable).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PagerStats {
    /// Blocks served from the RAM LRU.
    pub ram_hits: u64,
    /// Blocks reloaded from disk spill.
    pub spill_hits: u64,
    /// Blocks decoded via [`BlockSource`].
    pub decodes: u64,
    /// Wall time spent in [`BlockSource::decode_range`].
    pub decode_ns: u64,
    /// Blocks written to spill when evicted from RAM.
    pub spill_writes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BlockKey {
    media_id: MediaId,
    block_index: u64,
}

/// LRU RAM cache of decoded blocks with optional disk spill.
pub struct BlockPager {
    ram: HashMap<BlockKey, Arc<Vec<Vec<f32>>>>,
    order: VecDeque<BlockKey>,
    ram_bytes: usize,
    ram_limit: usize,
    spill_dir: PathBuf,
    source: Arc<dyn BlockSource>,
    stats: PagerStats,
}

impl BlockPager {
    /// Create a pager that spills under `spill_dir` using the default RAM budget.
    pub fn new(spill_dir: PathBuf, source: Arc<dyn BlockSource>) -> Result<Self> {
        Self::with_cache_bytes(spill_dir, RAM_CACHE_BYTES, source)
    }

    /// Create a pager with an explicit RAM cache budget.
    pub fn with_cache_bytes(
        spill_dir: PathBuf,
        ram_limit: usize,
        source: Arc<dyn BlockSource>,
    ) -> Result<Self> {
        fs::create_dir_all(&spill_dir)
            .with_context(|| format!("failed to create block cache {}", spill_dir.display()))?;
        Ok(Self {
            ram: HashMap::new(),
            order: VecDeque::new(),
            ram_bytes: 0,
            ram_limit: ram_limit.max(1),
            spill_dir,
            source,
            stats: PagerStats::default(),
        })
    }

    /// In-memory pager with no spill directory and a null decoder.
    pub fn in_memory() -> Self {
        Self {
            ram: HashMap::new(),
            order: VecDeque::new(),
            ram_bytes: 0,
            ram_limit: RAM_CACHE_BYTES,
            spill_dir: PathBuf::new(),
            source: Arc::new(NullBlockSource),
            stats: PagerStats::default(),
        }
    }

    /// Snapshot of cache / decode counters.
    pub fn stats(&self) -> PagerStats {
        self.stats
    }

    /// Clear counters without flushing the cache.
    pub fn reset_stats(&mut self) {
        self.stats = PagerStats::default();
    }

    /// Drop RAM-cached blocks for `media_id` (spill files are left for temp cleanup).
    pub fn evict_media(&mut self, media_id: MediaId) {
        let keys: Vec<_> = self
            .ram
            .keys()
            .copied()
            .filter(|key| key.media_id == media_id)
            .collect();
        for key in keys {
            if let Some(block) = self.ram.remove(&key) {
                self.ram_bytes = self.ram_bytes.saturating_sub(block_ram_bytes(&block));
            }
            self.order.retain(|k| k != &key);
        }
    }

    /// Fill planar destination channels from media starting at `src_offset`.
    pub fn fill_planar(
        &mut self,
        pool: &MediaPool,
        media_id: MediaId,
        src_offset: u64,
        count: u64,
        dest: &mut [&mut [f32]],
        dest_offset: usize,
    ) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let media = pool
            .get(media_id)
            .with_context(|| format!("unknown media {media_id}"))?;
        let mut remaining = count;
        let mut src = src_offset;
        let mut dst = dest_offset;
        while remaining > 0 {
            let block_index = src / BLOCK_FRAMES;
            let block_off = (src % BLOCK_FRAMES) as usize;
            let block = self.load_block(media, block_index)?;
            let available = block
                .first()
                .map(|ch| ch.len().saturating_sub(block_off))
                .unwrap_or(0) as u64;
            if available == 0 {
                break;
            }
            let take = remaining.min(available);
            for (ch, dest_ch) in dest.iter_mut().enumerate() {
                copy_block_channel(&block, ch, block_off, take, dest_ch, dst);
            }
            remaining -= take;
            src += take;
            dst += take as usize;
        }
        Ok(())
    }

    /// Fill a single channel destination from media starting at `src_offset`.
    pub fn fill_channel(
        &mut self,
        pool: &MediaPool,
        media_id: MediaId,
        src_offset: u64,
        count: u64,
        channel: usize,
        dest: &mut [f32],
    ) -> Result<()> {
        if count == 0 || dest.is_empty() {
            return Ok(());
        }
        let media = pool
            .get(media_id)
            .with_context(|| format!("unknown media {media_id}"))?;
        let mut remaining = count;
        let mut src = src_offset;
        let mut dst = 0usize;
        while remaining > 0 {
            let block_index = src / BLOCK_FRAMES;
            let block_off = (src % BLOCK_FRAMES) as usize;
            let block = self.load_block(media, block_index)?;
            let available = block
                .get(channel)
                .map(|ch| ch.len().saturating_sub(block_off))
                .or_else(|| block.first().map(|ch| ch.len().saturating_sub(block_off)))
                .unwrap_or(0) as u64;
            if available == 0 {
                break;
            }
            let take = remaining.min(available);
            copy_block_channel(&block, channel, block_off, take, dest, dst);
            remaining -= take;
            src += take;
            dst += take as usize;
        }
        Ok(())
    }

    /// Fill planar destinations from media, mapping each dest plane to a media
    /// channel via `source_channels[dest_index]`. When `source_channels` is
    /// `None`, behaves like [`Self::fill_planar`] (identity mapping).
    pub fn fill_planar_mapped(
        &mut self,
        pool: &MediaPool,
        media_id: MediaId,
        src_offset: u64,
        count: u64,
        dest: &mut [&mut [f32]],
        dest_offset: usize,
        source_channels: Option<&[usize]>,
    ) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let media = pool
            .get(media_id)
            .with_context(|| format!("unknown media {media_id}"))?;
        let mut remaining = count;
        let mut src = src_offset;
        let mut dst = dest_offset;
        while remaining > 0 {
            let block_index = src / BLOCK_FRAMES;
            let block_off = (src % BLOCK_FRAMES) as usize;
            let block = self.load_block(media, block_index)?;
            let available = block
                .first()
                .map(|ch| ch.len().saturating_sub(block_off))
                .unwrap_or(0) as u64;
            if available == 0 {
                break;
            }
            let take = remaining.min(available);
            for (dest_ch, dest_plane) in dest.iter_mut().enumerate() {
                let media_ch = source_channels
                    .and_then(|map| map.get(dest_ch).copied())
                    .unwrap_or(dest_ch);
                copy_block_channel(&block, media_ch, block_off, take, dest_plane, dst);
            }
            remaining -= take;
            src += take;
            dst += take as usize;
        }
        Ok(())
    }

    fn load_block(&mut self, media: &MediaRef, block_index: u64) -> Result<Arc<Vec<Vec<f32>>>> {
        let key = BlockKey {
            media_id: media.id,
            block_index,
        };
        if let Some(block) = self.ram.get(&key).cloned() {
            self.touch(key);
            self.stats.ram_hits = self.stats.ram_hits.saturating_add(1);
            return Ok(block);
        }
        if let Some(block) = self.load_spill(&key, media.channel_count)? {
            self.stats.spill_hits = self.stats.spill_hits.saturating_add(1);
            self.insert_ram(key, block.clone())?;
            return Ok(block);
        }
        let start = block_index * BLOCK_FRAMES;
        if start >= media.frame_count {
            let empty = Arc::new(vec![Vec::new(); media.channel_count.max(1)]);
            self.insert_ram(key, empty.clone())?;
            return Ok(empty);
        }
        let count = BLOCK_FRAMES.min(media.frame_count - start);
        let started = Instant::now();
        let decoded = decode_block(media, start, count, self.source.as_ref())?;
        self.stats.decode_ns = self
            .stats
            .decode_ns
            .saturating_add(started.elapsed().as_nanos() as u64);
        self.stats.decodes = self.stats.decodes.saturating_add(1);
        let block = Arc::new(decoded);
        self.insert_ram(key, block.clone())?;
        Ok(block)
    }

    fn touch(&mut self, key: BlockKey) {
        self.order.retain(|k| k != &key);
        self.order.push_back(key);
    }

    fn insert_ram(&mut self, key: BlockKey, block: Arc<Vec<Vec<f32>>>) -> Result<()> {
        if let Some(old) = self.ram.remove(&key) {
            self.ram_bytes = self.ram_bytes.saturating_sub(block_ram_bytes(&old));
            self.order.retain(|k| k != &key);
        }
        let incoming = block_ram_bytes(&block);
        while !self.ram.is_empty() && self.ram_bytes + incoming > self.ram_limit {
            if let Some(old) = self.order.pop_front() {
                if let Some(evicted) = self.ram.remove(&old) {
                    self.ram_bytes = self.ram_bytes.saturating_sub(block_ram_bytes(&evicted));
                    self.spill(&old, &evicted)?;
                }
            } else {
                break;
            }
        }
        self.ram_bytes = self.ram_bytes.saturating_add(incoming);
        self.ram.insert(key, block);
        self.touch(key);
        Ok(())
    }

    fn spill_path(&self, key: &BlockKey) -> PathBuf {
        self.spill_dir.join(format!(
            "m{}_b{}.blk",
            key.media_id.to_hex(),
            key.block_index
        ))
    }

    fn spill(&mut self, key: &BlockKey, block: &Arc<Vec<Vec<f32>>>) -> Result<()> {
        if self.spill_dir.as_os_str().is_empty() {
            return Ok(());
        }
        write_block(&self.spill_path(key), block)?;
        self.stats.spill_writes = self.stats.spill_writes.saturating_add(1);
        Ok(())
    }

    fn load_spill(
        &self,
        key: &BlockKey,
        channel_count: usize,
    ) -> Result<Option<Arc<Vec<Vec<f32>>>>> {
        if self.spill_dir.as_os_str().is_empty() {
            return Ok(None);
        }
        let path = self.spill_path(key);
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(Arc::new(read_block(&path, channel_count)?)))
    }
}

fn block_ram_bytes(block: &Arc<Vec<Vec<f32>>>) -> usize {
    block
        .iter()
        .map(|ch| ch.len() * std::mem::size_of::<f32>())
        .sum()
}

fn copy_block_channel(
    block: &[Vec<f32>],
    channel: usize,
    block_off: usize,
    take: u64,
    dest: &mut [f32],
    dst: usize,
) {
    let src_ch = block.get(channel).map(|c| c.as_slice()).unwrap_or(&[]);
    let src_end = (block_off + take as usize).min(src_ch.len());
    if block_off < src_end {
        let n = src_end - block_off;
        let dst_end = (dst + n).min(dest.len());
        let n = dst_end.saturating_sub(dst);
        dest[dst..dst + n].copy_from_slice(&src_ch[block_off..block_off + n]);
    }
}

fn decode_block(
    media: &MediaRef,
    start: u64,
    count: u64,
    source: &dyn BlockSource,
) -> Result<Vec<Vec<f32>>> {
    if let Some(samples) = &media.samples {
        let mut out = Vec::with_capacity(samples.len());
        for ch in samples.iter() {
            let s = start as usize;
            let e = (start + count) as usize;
            if s >= ch.len() {
                out.push(vec![0.0; count as usize]);
            } else {
                let e = e.min(ch.len());
                let mut slice = ch[s..e].to_vec();
                slice.resize(count as usize, 0.0);
                out.push(slice);
            }
        }
        return Ok(out);
    }
    source.decode_range(&media.path, start, count)
}

fn write_block(path: &Path, block: &[Vec<f32>]) -> Result<()> {
    let mut bytes = Vec::new();
    let channels = block.len() as u32;
    let frames = block.first().map(|c| c.len() as u32).unwrap_or(0);
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&frames.to_le_bytes());
    for ch in block {
        for sample in ch {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    fs::write(path, bytes).with_context(|| format!("failed to spill {}", path.display()))
}

fn read_block(path: &Path, expected_channels: usize) -> Result<Vec<Vec<f32>>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() < 8 {
        bail!("truncated block cache {}", path.display());
    }
    let channels = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let frames = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if expected_channels != 0 && channels != expected_channels {
        bail!("channel mismatch in {}", path.display());
    }
    let mut offset = 8;
    let mut out = Vec::with_capacity(channels);
    for _ in 0..channels {
        let mut ch = Vec::with_capacity(frames);
        for _ in 0..frames {
            if offset + 4 > bytes.len() {
                bail!("truncated samples in {}", path.display());
            }
            let bits: [u8; 4] = bytes[offset..offset + 4].try_into().unwrap();
            ch.push(f32::from_le_bytes(bits));
            offset += 4;
        }
        out.push(ch);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_from_memory_media() {
        let mut pool = MediaPool::new();
        let id = pool.insert(MediaRef::from_memory(
            MediaId([0u8; 32]),
            44100,
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]],
        ));
        let mut pager = BlockPager::in_memory();
        let mut left = [0.0; 2];
        let mut right = [0.0; 2];
        pager
            .fill_planar(&pool, id, 1, 2, &mut [&mut left[..], &mut right[..]], 0)
            .unwrap();
        assert_eq!(left, [2.0, 3.0]);
        assert_eq!(right, [6.0, 7.0]);
    }

    #[test]
    fn spills_evicted_blocks_to_disk() {
        let dir = std::env::temp_dir().join(format!(
            "snd-pager-spill-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut pool = MediaPool::new();
        let frames = BLOCK_FRAMES as usize * 2;
        let samples = vec![(0..frames).map(|i| i as f32).collect::<Vec<_>>()];
        let id = pool.insert(MediaRef::from_memory(MediaId([0u8; 32]), 44100, samples));
        let mut pager =
            BlockPager::with_cache_bytes(dir.clone(), 1024, Arc::new(NullBlockSource)).unwrap();
        let mut buf = [0.0; 1];
        pager
            .fill_planar(&pool, id, 0, 1, &mut [&mut buf[..]], 0)
            .unwrap();
        assert_eq!(buf[0], 0.0);
        pager
            .fill_planar(&pool, id, BLOCK_FRAMES, 1, &mut [&mut buf[..]], 0)
            .unwrap();
        assert_eq!(buf[0], BLOCK_FRAMES as f32);
        let spilled = dir.join(format!("m{}_b0.blk", id.to_hex()));
        assert!(
            spilled.is_file(),
            "expected spill file {}",
            spilled.display()
        );
        drop(pager);
        let mut pager =
            BlockPager::with_cache_bytes(dir.clone(), 1024, Arc::new(NullBlockSource)).unwrap();
        pager
            .fill_planar(&pool, id, 0, 1, &mut [&mut buf[..]], 0)
            .unwrap();
        assert_eq!(buf[0], 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
