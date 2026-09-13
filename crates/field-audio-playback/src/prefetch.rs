// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Lock-free SPSC prefetch ring for the realtime output callback.
//!
//! # Why not `dasp::ring_buffer`?
//!
//! `dasp`'s `Fixed` / `Bounded` ring buffers are excellent single-owner DSP
//! helpers, but every `push`/`pop` takes `&mut self`. Sharing them between the
//! prefetch thread and the audio callback would require a mutex and violate the
//! realtime quality gates. This module instead uses atomic cursors over a
//! preallocated interleaved `f32` slab so the producer and consumer never
//! contend for a lock.
//!
//! Prefetch may allocate and block; the callback only copies from this ring.

use std::sync::atomic::{AtomicU64, Ordering};

/// Source frames held in the prefetch ring (power of two).
pub const PREFETCH_CAPACITY_FRAMES: usize = 8192;
/// Source frames written per prefetch iteration.
pub const PREFETCH_CHUNK_FRAMES: usize = 256;

/// Interleaved `f32` ring shared by one producer (prefetch) and one consumer
/// (audio callback). Holds **pre-monitor** device-rate source frames.
pub struct PrefetchRing {
    samples: Box<[f32]>,
    capacity_frames: usize,
    channels: usize,
    write_frames: AtomicU64,
    read_frames: AtomicU64,
}

impl PrefetchRing {
    /// Allocate a zeroed ring for `channels`-wide interleaved frames.
    pub fn new(capacity_frames: usize, channels: usize) -> Self {
        let capacity_frames = capacity_frames.max(1).next_power_of_two();
        let channels = channels.max(1);
        let samples = vec![0.0f32; capacity_frames * channels].into_boxed_slice();
        Self {
            samples,
            capacity_frames,
            channels,
            write_frames: AtomicU64::new(0),
            read_frames: AtomicU64::new(0),
        }
    }

    /// Interleaved channel count.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Frame capacity (power of two).
    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Frames currently available to the consumer.
    pub fn frames_available(&self) -> usize {
        let w = self.write_frames.load(Ordering::Acquire);
        let r = self.read_frames.load(Ordering::Acquire);
        w.saturating_sub(r) as usize
    }

    /// Free frame slots for the producer.
    pub fn frames_free(&self) -> usize {
        self.capacity_frames.saturating_sub(self.frames_available())
    }

    /// Producer: discard unread audio after a seek/epoch change.
    pub fn discard_unread(&self) {
        let w = self.write_frames.load(Ordering::Acquire);
        self.read_frames.store(w, Ordering::Release);
    }

    /// Producer: push interleaved device frames. Returns frames written.
    pub fn push_interleaved(&self, src: &[f32]) -> usize {
        let ch = self.channels;
        if ch == 0 || src.len() < ch {
            return 0;
        }
        let want = src.len() / ch;
        let free = self.frames_free();
        let n = want.min(free);
        if n == 0 {
            return 0;
        }
        let mask = self.capacity_frames - 1;
        let mut w = self.write_frames.load(Ordering::Relaxed);
        for frame in 0..n {
            let slot = (w as usize & mask) * ch;
            let base = frame * ch;
            // SAFETY: exclusive producer write to distinct slots; consumer only
            // reads indices behind `read_frames`.
            unsafe {
                let dst = self.samples.as_ptr().add(slot) as *mut f32;
                std::ptr::copy_nonoverlapping(src.as_ptr().add(base), dst, ch);
            }
            w = w.wrapping_add(1);
        }
        self.write_frames.store(w, Ordering::Release);
        n
    }

    /// Consumer: pop interleaved device frames into `dst`. Returns frames read.
    ///
    /// Remainder of `dst` is left untouched (caller should zero first).
    pub fn pop_interleaved(&self, dst: &mut [f32]) -> usize {
        let ch = self.channels;
        if ch == 0 || dst.len() < ch {
            return 0;
        }
        let want = dst.len() / ch;
        let avail = self.frames_available();
        let n = want.min(avail);
        if n == 0 {
            return 0;
        }
        let mask = self.capacity_frames - 1;
        let mut r = self.read_frames.load(Ordering::Relaxed);
        for frame in 0..n {
            let slot = (r as usize & mask) * ch;
            let base = frame * ch;
            unsafe {
                let src = self.samples.as_ptr().add(slot);
                std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr().add(base), ch);
            }
            r = r.wrapping_add(1);
        }
        self.read_frames.store(r, Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_round_trip() {
        let ring = PrefetchRing::new(8, 2);
        assert_eq!(ring.push_interleaved(&[1.0, 2.0, 3.0, 4.0]), 2);
        let mut out = [0.0; 4];
        assert_eq!(ring.pop_interleaved(&mut out), 2);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn discard_unread_empties_ring() {
        let ring = PrefetchRing::new(8, 1);
        assert_eq!(ring.push_interleaved(&[1.0, 2.0, 3.0]), 3);
        ring.discard_unread();
        assert_eq!(ring.frames_available(), 0);
        let mut out = [9.0; 3];
        assert_eq!(ring.pop_interleaved(&mut out), 0);
    }

    #[test]
    fn does_not_overwrite_when_full() {
        let ring = PrefetchRing::new(4, 1);
        assert_eq!(ring.push_interleaved(&[1.0, 2.0, 3.0, 4.0]), 4);
        assert_eq!(ring.push_interleaved(&[5.0]), 0);
        let mut out = [0.0; 4];
        assert_eq!(ring.pop_interleaved(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }
}
