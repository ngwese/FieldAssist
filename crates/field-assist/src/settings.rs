// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! FieldAssist GPUI settings store over [`field_settings`].

use field_features::{Feature, FeatureRegistry};
use field_settings::{load_from_dir, path_in, save_to_dir, SpectrumGradientStopSettings};
use field_ui_components::{
    PeakRendering, SpectrumGradient, SpectrumGradientStop, WaveformRepresentation,
};
use gpui_kit::{App, Global};

#[allow(unused_imports)] // public re-exports for callers / docs
pub use field_settings::{
    AppSettings, AppearanceSettings, AudioSettings, ExperimentalSettings, ScriptingSettings,
    SelectionSettings, ViewSettings, WaveformSettings, DOCK_HIDDEN, SETTINGS_FORMAT_VERSION,
    SETTINGS_KIND,
};

/// In-memory settings + launch flags used by the Settings UI and `app:load_settings`.
#[derive(Clone, Debug)]
pub struct AppSettingsStore {
    pub settings: AppSettings,
    /// Live feature flags synced from [`AppSettings::experimental`].
    pub features: FeatureRegistry,
    /// When true, `load_settings` must not overwrite the CLI `--output` device.
    pub cli_output_locked: bool,
}

impl Default for AppSettingsStore {
    fn default() -> Self {
        let settings = AppSettings::default();
        let features = FeatureRegistry::from_flags(settings.experimental.flags.clone());
        Self {
            settings,
            features,
            cli_output_locked: false,
        }
    }
}

impl Global for AppSettingsStore {}

/// Resolve the user config directory used for `settings.json`.
pub fn config_dir() -> Option<std::path::PathBuf> {
    crate::commands::user_config_dir()
}

/// Path to `settings.json` beside `init.lua`, if the config dir resolves.
#[allow(dead_code)] // public helper; covered by unit tests
pub fn settings_path() -> Option<std::path::PathBuf> {
    path_in(config_dir().as_deref())
}

/// Load from the user config directory (or defaults).
pub fn load_app_settings() -> AppSettings {
    let dir = config_dir();
    load_from_dir(dir.as_deref())
}

/// Persist settings to the user config directory.
pub fn save_app_settings(settings: &AppSettings) -> Result<(), String> {
    save_to_dir(settings, config_dir().as_deref())
}

/// Ensure a global store exists (defaults only until reload).
pub fn ensure_store(cx: &mut App) {
    if cx.try_global::<AppSettingsStore>().is_none() {
        cx.set_global(AppSettingsStore::default());
    }
}

/// Mark that CLI `--output` should win over settings.json audio.
pub fn lock_cli_output_device(cx: &mut App) {
    ensure_store(cx);
    cx.global_mut::<AppSettingsStore>().cli_output_locked = true;
}

pub fn store(cx: &App) -> &AppSettingsStore {
    cx.global::<AppSettingsStore>()
}

pub fn store_mut(cx: &mut App) -> &mut AppSettingsStore {
    ensure_store(cx);
    cx.global_mut::<AppSettingsStore>()
}

/// Replace in-memory settings from disk (does not apply to the live UI).
pub fn reload_from_disk(cx: &mut App) {
    let settings = load_app_settings();
    let store = store_mut(cx);
    store
        .features
        .set_flags(settings.experimental.flags.clone());
    store.settings = settings;
}

/// Persist current in-memory settings.
pub fn save_store(cx: &App) -> Result<(), String> {
    save_app_settings(&store(cx).settings)
}

/// Mutate settings, persist, and return the new snapshot.
pub fn update_and_save(cx: &mut App, f: impl FnOnce(&mut AppSettings)) -> Result<(), String> {
    {
        let store = store_mut(cx);
        f(&mut store.settings);
        store
            .features
            .set_flags(store.settings.experimental.flags.clone());
    }
    save_store(cx)
}

/// Whether a product feature flag is currently enabled.
pub fn feature_enabled(feature: Feature, cx: &App) -> bool {
    cx.try_global::<AppSettingsStore>()
        .map(|s| s.features.is_enabled(feature))
        .unwrap_or(false)
}

/// FieldAssist UI helpers for [`WaveformSettings`].
#[allow(dead_code)] // setters kept for Settings / script parity
pub trait WaveformSettingsExt {
    /// Parse the representation string into the paint enum.
    fn representation_enum(&self) -> WaveformRepresentation;
    /// Store a representation enum.
    fn set_representation_enum(&mut self, rep: WaveformRepresentation);
    /// Parse peak rendering into the paint enum.
    fn peak_rendering_enum(&self) -> PeakRendering;
    /// Store a peak-rendering enum.
    fn set_peak_rendering_enum(&mut self, mode: PeakRendering);
    /// Convert stored stops into a normalized paint colormap.
    fn spectrum_gradient_enum(&self) -> SpectrumGradient;
    /// Replace the spectrum gradient (normalized) and sync stored stops.
    fn set_spectrum_gradient(&mut self, gradient: SpectrumGradient);
    /// Update one stop's RGB and re-normalize for paint.
    fn set_spectrum_stop_rgb_normalized(&mut self, index: usize, rgb: [f32; 3]);
    /// Update one stop's dB (clamped between neighbors) and re-normalize.
    fn set_spectrum_stop_db_normalized(&mut self, index: usize, db: f32);
}

impl WaveformSettingsExt for WaveformSettings {
    fn representation_enum(&self) -> WaveformRepresentation {
        match self.representation.as_str() {
            "spectrum" => WaveformRepresentation::Spectrum,
            "peaks_spectrum" => WaveformRepresentation::PeaksSpectrum,
            _ => WaveformRepresentation::Peaks,
        }
    }

    fn set_representation_enum(&mut self, rep: WaveformRepresentation) {
        self.representation = match rep {
            WaveformRepresentation::Peaks => "peaks".into(),
            WaveformRepresentation::Spectrum => "spectrum".into(),
            WaveformRepresentation::PeaksSpectrum => "peaks_spectrum".into(),
        };
    }

    fn peak_rendering_enum(&self) -> PeakRendering {
        match self.peak_rendering.as_str() {
            "simple" => PeakRendering::Simple,
            _ => PeakRendering::Threaded,
        }
    }

    fn set_peak_rendering_enum(&mut self, mode: PeakRendering) {
        self.peak_rendering = match mode {
            PeakRendering::Simple => "simple".into(),
            PeakRendering::Threaded => "threaded".into(),
        };
    }

    fn spectrum_gradient_enum(&self) -> SpectrumGradient {
        SpectrumGradient {
            stops: self.spectrum_gradient.map(|stop| SpectrumGradientStop {
                rgb: stop.rgb,
                db: stop.db,
            }),
        }
        .normalized()
    }

    fn set_spectrum_gradient(&mut self, gradient: SpectrumGradient) {
        let gradient = gradient.normalized();
        self.spectrum_gradient = gradient.stops.map(|stop| SpectrumGradientStopSettings {
            rgb: stop.rgb,
            db: stop.db,
        });
    }

    fn set_spectrum_stop_rgb_normalized(&mut self, index: usize, rgb: [f32; 3]) {
        self.set_spectrum_stop_rgb(index, rgb);
        let g = self.spectrum_gradient_enum();
        self.set_spectrum_gradient(g);
    }

    fn set_spectrum_stop_db_normalized(&mut self, index: usize, db: f32) {
        let g = self.spectrum_gradient_enum();
        let db = g.clamp_stop_db(index, db);
        if let Some(stop) = self.spectrum_gradient.get_mut(index) {
            stop.db = db;
        }
        let g = self.spectrum_gradient_enum();
        self.set_spectrum_gradient(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field_settings::{
        DEFAULT_THREADED_RIBBON_DB, DEFAULT_THREADED_SHELL_VALUE_REDUCE, SETTINGS_FORMAT_VERSION,
        SETTINGS_KIND,
    };

    #[test]
    fn defaults_match_shipping_behavior() {
        let s = AppSettings::default();
        assert_eq!(s.kind, SETTINGS_KIND);
        assert_eq!(s.format_version, SETTINGS_FORMAT_VERSION);
        assert_eq!(s.appearance.theme_name, "Default Dark");
        assert!(!s.view.explorer);
        assert_eq!(s.view.detail, DOCK_HIDDEN);
        assert_eq!(s.waveform.representation, "peaks");
        assert_eq!(
            s.waveform.threaded_shell_value_reduce,
            DEFAULT_THREADED_SHELL_VALUE_REDUCE
        );
        assert_eq!(s.waveform.threaded_ribbon_db, DEFAULT_THREADED_RIBBON_DB);
        assert_eq!(
            s.waveform.spectrum_gradient_enum(),
            SpectrumGradient::classic()
        );
        assert!(!s.scripting.enable_system_package_paths);
        assert!(!s.scripting.enable_native_modules);
    }

    #[test]
    fn path_ends_with_settings_json() {
        let Some(path) = settings_path() else {
            return;
        };
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("settings.json")
        );
    }
}
