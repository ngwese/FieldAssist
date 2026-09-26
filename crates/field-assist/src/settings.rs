// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Application preferences (`settings.json` next to `init.lua`).

use std::path::PathBuf;

use field_features::{Feature, FeatureFlags, FeatureRegistry};
use field_ui_components::{
    PeakRendering, WaveformRepresentation, DEFAULT_THREADED_RIBBON_DB,
    DEFAULT_THREADED_SHELL_VALUE_REDUCE,
};
use gpui_kit::{App, Global};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

/// Top-level preferences file schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub view: ViewSettings,
    pub waveform: WaveformSettings,
    pub selection: SelectionSettings,
    pub audio: AudioSettings,
    pub experimental: ExperimentalSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceSettings::default(),
            view: ViewSettings::default(),
            waveform: WaveformSettings::default(),
            selection: SelectionSettings::default(),
            audio: AudioSettings::default(),
            experimental: ExperimentalSettings::default(),
        }
    }
}

/// Theme name and light/dark mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppearanceSettings {
    pub theme_name: String,
    /// `"light"` or `"dark"`.
    pub theme_mode: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme_name: "Default Dark".into(),
            theme_mode: "dark".into(),
        }
    }
}

/// Default show/hide and preferred tab for tool docks on a fresh window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ViewSettings {
    pub explorer: bool,
    /// `"hidden"` or a detail-dock tab title (`Marker`, `Regions`, `History`, `Monitor`).
    /// Legacy booleans still deserialize (`true` → Marker, `false` → hidden).
    #[serde(deserialize_with = "deserialize_detail_pref")]
    #[schemars(with = "String")]
    pub detail: String,
    /// `"hidden"` or a bottom-dock tab title (`Script`, `Messages`, `Media`).
    /// Legacy booleans still deserialize (`true` → Script, `false` → hidden).
    #[serde(deserialize_with = "deserialize_script_pref")]
    #[schemars(with = "String")]
    pub script: String,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            explorer: false,
            detail: DOCK_HIDDEN.into(),
            script: DOCK_HIDDEN.into(),
        }
    }
}

/// Sentinel for a closed dock in [`ViewSettings::detail`] / [`ViewSettings::script`].
pub const DOCK_HIDDEN: &str = "hidden";

impl ViewSettings {
    pub fn detail_open(&self) -> bool {
        self.detail != DOCK_HIDDEN
    }

    pub fn script_open(&self) -> bool {
        self.script != DOCK_HIDDEN
    }
}

fn deserialize_detail_pref<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_dock_pref(deserializer, crate::dock_titles::DETAIL_TAB_MARKER)
}

fn deserialize_script_pref<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_dock_pref(deserializer, crate::dock_titles::BOTTOM_TAB_SCRIPT)
}

fn deserialize_dock_pref<'de, D>(
    deserializer: D,
    when_true: &'static str,
) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Bool(true) => Ok(when_true.into()),
        serde_json::Value::Bool(false) => Ok(DOCK_HIDDEN.into()),
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Null => Ok(DOCK_HIDDEN.into()),
        other => Err(serde::de::Error::custom(format!(
            "expected string or bool for dock preference, got {other}"
        ))),
    }
}

/// Default waveform body and playhead following for new windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WaveformSettings {
    /// `"peaks"`, `"spectrum"`, or `"peaks_spectrum"`.
    pub representation: String,
    /// Keep the playhead in view while playing / after seeks.
    pub follow_playhead: bool,
    /// `"simple"` or `"threaded"` overview peak paint.
    pub peak_rendering: String,
    /// HSV value reduction for Threaded shell bars (`0.0..=1.0`).
    pub threaded_shell_value_reduce: f32,
    /// Ribbon amplitude scale in dBFS for Threaded peak paint.
    pub threaded_ribbon_db: f32,
}

impl Default for WaveformSettings {
    fn default() -> Self {
        Self {
            representation: "peaks".into(),
            follow_playhead: true,
            peak_rendering: "threaded".into(),
            threaded_shell_value_reduce: DEFAULT_THREADED_SHELL_VALUE_REDUCE,
            threaded_ribbon_db: DEFAULT_THREADED_RIBBON_DB,
        }
    }
}

impl WaveformSettings {
    pub fn representation_enum(&self) -> WaveformRepresentation {
        Self::from_representation_string(&self.representation)
    }

    pub fn set_representation_enum(&mut self, rep: WaveformRepresentation) {
        self.representation = match rep {
            WaveformRepresentation::Peaks => "peaks".into(),
            WaveformRepresentation::Spectrum => "spectrum".into(),
            WaveformRepresentation::PeaksSpectrum => "peaks_spectrum".into(),
        };
    }

    pub fn set_representation_str(&mut self, value: &str) {
        self.set_representation_enum(Self::from_representation_string(value));
    }

    fn from_representation_string(value: &str) -> WaveformRepresentation {
        match value {
            "spectrum" => WaveformRepresentation::Spectrum,
            "peaks_spectrum" => WaveformRepresentation::PeaksSpectrum,
            _ => WaveformRepresentation::Peaks,
        }
    }

    pub fn peak_rendering_enum(&self) -> PeakRendering {
        Self::from_peak_rendering_string(&self.peak_rendering)
    }

    pub fn set_peak_rendering_enum(&mut self, mode: PeakRendering) {
        self.peak_rendering = match mode {
            PeakRendering::Simple => "simple".into(),
            PeakRendering::Threaded => "threaded".into(),
        };
    }

    pub fn set_peak_rendering_str(&mut self, value: &str) {
        self.set_peak_rendering_enum(Self::from_peak_rendering_string(value));
    }

    fn from_peak_rendering_string(value: &str) -> PeakRendering {
        match value {
            "simple" => PeakRendering::Simple,
            _ => PeakRendering::Threaded,
        }
    }

    pub fn set_threaded_shell_value_reduce(&mut self, value: f32) {
        self.threaded_shell_value_reduce =
            field_ui_components::clamp_threaded_shell_value_reduce(value);
    }

    pub fn set_threaded_ribbon_db(&mut self, value: f32) {
        self.threaded_ribbon_db = field_ui_components::clamp_threaded_ribbon_db(value);
    }
}

/// Defaults for selection / marker targeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SelectionSettings {
    pub zero_crossing: bool,
    pub snap_to_marker: bool,
    pub add_at_hover: bool,
}

impl Default for SelectionSettings {
    fn default() -> Self {
        Self {
            zero_crossing: true,
            snap_to_marker: false,
            add_at_hover: true,
        }
    }
}

/// Preferred output device (`null` = system default).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AudioSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device: None,
        }
    }
}

/// Unstable / preview preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExperimentalSettings {
    pub flags: FeatureFlags,
}

impl Default for ExperimentalSettings {
    fn default() -> Self {
        Self {
            flags: FeatureFlags::default(),
        }
    }
}

impl AppSettings {
    /// Path to `settings.json` beside `init.lua`, if the config dir resolves.
    pub fn path() -> Option<PathBuf> {
        crate::commands::user_config_dir().map(|dir| dir.join("settings.json"))
    }

    /// Load from disk, or defaults when missing/invalid.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<AppSettings>(&text) {
                Ok(settings) => settings,
                Err(err) => {
                    eprintln!(
                        "FieldAssist: ignoring invalid settings.json ({}): {err}",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!(
                    "FieldAssist: could not read settings.json ({}): {err}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write pretty JSON to the config directory (creating it if needed).
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "config directory is unavailable".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("{err:#}"))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| format!("{err:#}"))?;
        std::fs::write(&path, text + "\n").map_err(|err| format!("{err:#}"))
    }
}

/// Ensure a global store exists (defaults only; init.lua loads from disk).
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
    let settings = AppSettings::load();
    let store = store_mut(cx);
    store
        .features
        .set_flags(settings.experimental.flags.clone());
    store.settings = settings;
}

/// Persist current in-memory settings.
pub fn save_store(cx: &App) -> Result<(), String> {
    store(cx).settings.save()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_shipping_behavior() {
        let s = AppSettings::default();
        assert_eq!(s.appearance.theme_name, "Default Dark");
        assert_eq!(s.appearance.theme_mode, "dark");
        assert!(!s.view.explorer);
        assert_eq!(s.view.detail, DOCK_HIDDEN);
        assert_eq!(s.view.script, DOCK_HIDDEN);
        assert!(!s.view.detail_open() && !s.view.script_open());
        assert_eq!(s.waveform.representation, "peaks");
        assert!(s.waveform.follow_playhead);
        assert_eq!(s.waveform.peak_rendering, "threaded");
        assert_eq!(
            s.waveform.threaded_shell_value_reduce,
            DEFAULT_THREADED_SHELL_VALUE_REDUCE
        );
        assert_eq!(s.waveform.threaded_ribbon_db, DEFAULT_THREADED_RIBBON_DB);
        assert!(s.selection.zero_crossing);
        assert!(!s.selection.snap_to_marker);
        assert!(s.selection.add_at_hover);
        assert!(s.audio.output_device.is_none());
        assert!(!s.experimental.flags.content_credentials);
        assert!(!s.experimental.flags.analysis_ops);
    }

    #[test]
    fn round_trip_json() {
        let mut s = AppSettings::default();
        s.appearance.theme_mode = "light".into();
        s.view.script = crate::dock_titles::BOTTOM_TAB_SCRIPT.into();
        s.waveform
            .set_representation_enum(WaveformRepresentation::Spectrum);
        s.waveform.set_peak_rendering_enum(PeakRendering::Threaded);
        s.waveform.set_threaded_shell_value_reduce(0.15);
        s.waveform.set_threaded_ribbon_db(-6.0);
        s.audio.output_device = Some("Speakers".into());
        s.experimental.flags.analysis_ops = true;
        let text = serde_json::to_string_pretty(&s).unwrap();
        let back: AppSettings = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let json = r#"{
            "appearance": { "theme_name": "Default Dark", "theme_mode": "dark" },
            "future_key": 1
        }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.appearance.theme_mode, "dark");
        assert!(!s.view.explorer);
        assert_eq!(s.view.detail, DOCK_HIDDEN);
    }

    #[test]
    fn legacy_bool_dock_prefs_deserialize() {
        let json = r#"{
            "view": { "explorer": true, "detail": true, "script": false }
        }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.view.detail, crate::dock_titles::DETAIL_TAB_MARKER);
        assert_eq!(s.view.script, DOCK_HIDDEN);

        let json = r#"{
            "view": { "detail": false, "script": true }
        }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.view.detail, DOCK_HIDDEN);
        assert_eq!(s.view.script, crate::dock_titles::BOTTOM_TAB_SCRIPT);
    }

    #[test]
    fn path_ends_with_settings_json() {
        let Some(path) = AppSettings::path() else {
            return;
        };
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("settings.json")
        );
    }
}
