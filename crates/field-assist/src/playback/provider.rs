// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! App-specific [`PlaybackDataProvider`] implementations.
//!
//! `DecodedAudio` / `Buffer` impls live behind local newtypes because those
//! types are defined in other crates (orphan rules).

use std::sync::{Arc, RwLock};

use field_audio_playback::PlaybackDataProvider;

use crate::audio::DecodedAudio;
use crate::model::composition::Composition;
use crate::model::Buffer;

/// Playback provider whose composition target can be swapped without replacing
/// the `Arc<dyn PlaybackDataProvider>` the engine already holds.
pub struct SharedCompositionProvider {
    current: RwLock<Arc<RwLock<Composition>>>,
}

impl SharedCompositionProvider {
    /// Bind the initial composition.
    pub fn new(composition: Arc<RwLock<Composition>>) -> Self {
        Self {
            current: RwLock::new(composition),
        }
    }

    /// Swap the composition target without changing the provider `Arc`.
    pub fn bind(&self, composition: Arc<RwLock<Composition>>) {
        *self.current.write().unwrap() = composition;
    }

    fn composition(&self) -> Arc<RwLock<Composition>> {
        self.current.read().unwrap().clone()
    }
}

impl PlaybackDataProvider for SharedCompositionProvider {
    fn sample_rate(&self) -> u32 {
        self.composition().read().unwrap().sample_rate()
    }

    fn channel_count(&self) -> usize {
        self.composition().read().unwrap().channel_count()
    }

    fn frames(&self) -> usize {
        self.composition().read().unwrap().frames() as usize
    }

    fn read_interleaved(&self, start: usize, count: usize, dest: &mut [f32]) {
        let _ =
            self.composition()
                .read()
                .unwrap()
                .read_interleaved(start as u64, count as u64, dest);
    }
}

/// Newtype so assist can implement [`PlaybackDataProvider`] for decoded PCM.
pub struct DecodedAudioProvider(pub DecodedAudio);

impl PlaybackDataProvider for DecodedAudioProvider {
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate
    }

    fn channel_count(&self) -> usize {
        self.0.channel_count()
    }

    fn frames(&self) -> usize {
        self.0.frames()
    }

    fn read_interleaved(&self, start: usize, count: usize, dest: &mut [f32]) {
        read_planar_interleaved(&self.0.channels, start, count, dest);
    }
}

/// Newtype so assist can implement [`PlaybackDataProvider`] for [`Buffer`].
pub struct BufferProvider<'a>(pub &'a Buffer);

impl PlaybackDataProvider for BufferProvider<'_> {
    fn sample_rate(&self) -> u32 {
        self.0.audio.sample_rate
    }

    fn channel_count(&self) -> usize {
        self.0.audio.channel_count()
    }

    fn frames(&self) -> usize {
        self.0.frames()
    }

    fn read_interleaved(&self, start: usize, count: usize, dest: &mut [f32]) {
        read_planar_interleaved(&self.0.audio.channels, start, count, dest);
    }
}

fn read_planar_interleaved(channels: &[Vec<f32>], start: usize, count: usize, dest: &mut [f32]) {
    let ch_count = channels.len();
    if ch_count == 0 {
        dest.fill(0.0);
        return;
    }
    let total = count * ch_count;
    debug_assert!(dest.len() >= total);
    dest[..total].fill(0.0);

    for (ch, samples) in channels.iter().enumerate() {
        let end = (start + count).min(samples.len());
        if start >= end {
            continue;
        }
        let slice = &samples[start..end];
        for (frame, &sample) in slice.iter().enumerate() {
            dest[frame * ch_count + ch] = sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MediaRef;

    #[test]
    fn interleaves_stereo_frames() {
        let audio = DecodedAudioProvider(DecodedAudio {
            sample_rate: 44100,
            channels: vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            peaks: vec![vec![], vec![]],
        });
        let mut dest = [0.0; 4];
        audio.read_interleaved(0, 2, &mut dest);
        assert_eq!(dest, [1.0, 3.0, 2.0, 4.0]);
    }

    #[test]
    fn zero_fills_past_end() {
        let audio = DecodedAudioProvider(DecodedAudio {
            sample_rate: 44100,
            channels: vec![vec![1.0], vec![2.0]],
            peaks: vec![vec![], vec![]],
        });
        let mut dest = [9.0; 4];
        audio.read_interleaved(1, 2, &mut dest);
        assert_eq!(dest, [0.0, 0.0, 0.0, 0.0]);
    }

    fn memory_composition(frames: usize) -> Composition {
        use crate::model::composition::MediaId;
        let samples = vec![vec![0.0; frames]];
        let media = MediaRef::from_memory(MediaId(0), 44100, samples);
        Composition::from_media(media).unwrap()
    }

    #[test]
    fn bind_switches_provider_frames() {
        let first = Arc::new(RwLock::new(memory_composition(8)));
        let second = Arc::new(RwLock::new(memory_composition(32)));
        let provider = SharedCompositionProvider::new(first);
        assert_eq!(PlaybackDataProvider::frames(&provider), 8);
        provider.bind(second);
        assert_eq!(PlaybackDataProvider::frames(&provider), 32);
    }

    #[test]
    fn bind_leaves_previous_composition_intact() {
        let first = Arc::new(RwLock::new(memory_composition(8)));
        let second = Arc::new(RwLock::new(memory_composition(32)));
        let provider = SharedCompositionProvider::new(first.clone());
        provider.bind(second);
        assert_eq!(first.read().unwrap().frames(), 8);
        assert_eq!(PlaybackDataProvider::frames(&provider), 32);
    }
}
