// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Lock-free live parameter and meter store.
//!
//! UI threads write/read control values via [`AtomicU32`] bit-casts of `f32`.
//! The audio thread snapshots controls at the start of each block and writes
//! meter values after DSP compute. Slot registration may take a short
//! [`RwLock`]; value get/set never waits on the DSP graph mutex.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

fn f32_bits(value: f32) -> u32 {
    value.to_bits()
}

fn bits_f32(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Shared atomic store for monitor control and meter parameters.
#[derive(Debug, Default)]
pub struct ParamStore {
    controls: RwLock<HashMap<String, Arc<AtomicU32>>>,
    meters: RwLock<HashMap<String, Arc<AtomicU32>>>,
}

impl ParamStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_slot(
        map: &RwLock<HashMap<String, Arc<AtomicU32>>>,
        address: &str,
        init: f32,
    ) -> Arc<AtomicU32> {
        {
            let guard = map.read().expect("param store poisoned");
            if let Some(slot) = guard.get(address) {
                return Arc::clone(slot);
            }
        }
        let mut guard = map.write().expect("param store poisoned");
        Arc::clone(
            guard
                .entry(address.to_string())
                .or_insert_with(|| Arc::new(AtomicU32::new(f32_bits(init)))),
        )
    }

    /// Set a live control value without locking the DSP graph.
    pub fn set(&self, address: &str, value: f32) {
        let slot = Self::ensure_slot(&self.controls, address, value);
        slot.store(f32_bits(value), Ordering::Release);
    }

    /// Read a live control value.
    pub fn get(&self, address: &str) -> Option<f32> {
        let guard = self.controls.read().expect("param store poisoned");
        guard
            .get(address)
            .map(|slot| bits_f32(slot.load(Ordering::Acquire)))
    }

    /// Snapshot all live control values for applying to DSP at block start.
    pub fn snapshot_controls(&self) -> Vec<(String, f32)> {
        let guard = self.controls.read().expect("param store poisoned");
        guard
            .iter()
            .map(|(address, slot)| {
                (
                    address.clone(),
                    bits_f32(slot.load(Ordering::Acquire)),
                )
            })
            .collect()
    }

    /// Replace control slots with the given map (used when loading working params).
    pub fn replace_controls(&self, values: &HashMap<String, f32>) {
        let mut guard = self.controls.write().expect("param store poisoned");
        guard.clear();
        for (address, value) in values {
            guard.insert(
                address.clone(),
                Arc::new(AtomicU32::new(f32_bits(*value))),
            );
        }
    }

    /// Clear all live control slots.
    pub fn clear_controls(&self) {
        self.controls
            .write()
            .expect("param store poisoned")
            .clear();
    }

    /// Write a meter value from the audio thread.
    pub fn set_meter(&self, address: &str, value: f32) {
        let slot = Self::ensure_slot(&self.meters, address, value);
        slot.store(f32_bits(value), Ordering::Release);
    }

    /// Read a meter value from the UI thread.
    pub fn meter(&self, address: &str) -> Option<f32> {
        let guard = self.meters.read().expect("param store poisoned");
        guard
            .get(address)
            .map(|slot| bits_f32(slot.load(Ordering::Acquire)))
    }

    /// Snapshot all meters.
    pub fn meters_snapshot(&self) -> HashMap<String, f32> {
        let guard = self.meters.read().expect("param store poisoned");
        guard
            .iter()
            .map(|(address, slot)| {
                (
                    address.clone(),
                    bits_f32(slot.load(Ordering::Acquire)),
                )
            })
            .collect()
    }

    /// Replace meter slots after a chain change.
    pub fn replace_meters(&self, addresses: &[String]) {
        let mut guard = self.meters.write().expect("param store poisoned");
        guard.clear();
        for address in addresses {
            guard.insert(address.clone(), Arc::new(AtomicU32::new(f32_bits(0.0))));
        }
    }

    /// Clear meters.
    pub fn clear_meters(&self) {
        self.meters.write().expect("param store poisoned").clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_round_trip() {
        let store = ParamStore::new();
        store.set("/Gain", -6.0);
        assert!((store.get("/Gain").unwrap() + 6.0).abs() < 1e-6);
    }

    #[test]
    fn snapshot_includes_writes() {
        let store = ParamStore::new();
        store.set("/a", 1.0);
        store.set("/b", 2.0);
        let snap = store.snapshot_controls();
        assert_eq!(snap.len(), 2);
    }
}
