// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! [`PlaybackDataProvider`] over a shared [`Composition`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use field_audio_playback::PlaybackDataProvider;
use field_composition::Composition;

/// Playback provider that reads interleaved PCM from a locked composition.
pub struct CompositionProvider {
    composition: Arc<RwLock<Composition>>,
    read_error_logged: AtomicBool,
}

impl CompositionProvider {
    /// Bind `composition` as the PCM source.
    pub fn new(composition: Arc<RwLock<Composition>>) -> Self {
        Self {
            composition,
            read_error_logged: AtomicBool::new(false),
        }
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
        if let Err(err) =
            self.composition
                .read()
                .unwrap()
                .read_interleaved(start as u64, count as u64, dest)
        {
            if !self.read_error_logged.swap(true, Ordering::Relaxed) {
                eprintln!("field-play: read_interleaved failed: {err:#}");
            }
        }
    }
}
