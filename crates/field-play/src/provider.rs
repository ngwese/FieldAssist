// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! [`PlaybackDataProvider`] over a shared [`Composition`].

use std::sync::{Arc, RwLock};

use field_audio_playback::PlaybackDataProvider;
use field_composition::Composition;

/// Playback provider that reads interleaved PCM from a locked composition.
pub struct CompositionProvider {
    composition: Arc<RwLock<Composition>>,
}

impl CompositionProvider {
    /// Bind `composition` as the PCM source.
    pub fn new(composition: Arc<RwLock<Composition>>) -> Self {
        Self { composition }
    }
}

impl PlaybackDataProvider for CompositionProvider {
    fn sample_rate(&self) -> u32 {
        self.composition.read().unwrap().sample_rate()
    }

    fn channel_count(&self) -> usize {
        self.composition.read().unwrap().channel_count()
    }

    fn frames(&self) -> usize {
        self.composition.read().unwrap().frames() as usize
    }

    fn read_interleaved(&self, start: usize, count: usize, dest: &mut [f32]) {
        let _ = self
            .composition
            .read()
            .unwrap()
            .read_interleaved(start as u64, count as u64, dest);
    }
}
