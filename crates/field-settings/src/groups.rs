// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Settings group structs and the versioned `AppSettings` envelope.

use std::path::{Path, PathBuf};

use field_features::FeatureFlags;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// On-disk kind marker for `settings.json`.
pub const SETTINGS_KIND: &str = "settings";
/// Current `settings.json` format version.
pub const SETTINGS_FORMAT_VERSION: u32 = 1;

/// Sentinel for a closed dock in [`ViewSettings::detail`] / [`ViewSettings::script`].
pub const DOCK_HIDDEN: &str = "hidden";

/// Legacy `detail: true` maps to this FieldAssist detail-dock tab title.
pub const DETAIL_DOCK_TRUE_TAB: &str = "Marker";
/// Legacy `script: true` maps to this FieldAssist bottom-dock tab title.
pub const SCRIPT_DOCK_TRUE_TAB: &str = "Script";

/// Default HSV value reduction for threaded peak shells (`0.0..=1.0`).
pub const DEFAULT_THREADED_SHELL_VALUE_REDUCE: f32 = 0.25;
/// Default ribbon amplitude scale (dBFS) for threaded peaks.
pub const DEFAULT_THREADED_RIBBON_DB: f32 = -3.0;
/// Spectrum heatmap floor (dBFS).
pub const SPECTRUM_GRADIENT_DB_FLOOR: f32 = -80.0;

/// Top-level preferences file schema (versioned envelope).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppSettings {
    /// File kind marker.
    pub kind: String,
    /// Format version.
    pub format_version: u32,
    /// Application chrome: theme, docks, selection defaults.
    pub appearance: AppearanceSettings,
    /// Dock visibility defaults.
    pub view: ViewSettings,
    /// Waveform display defaults.
    pub waveform: WaveformSettings,
    /// Selection / marker targeting defaults.
    pub selection: SelectionSettings,
    /// Preferred output device (Device group).
    pub audio: AudioSettings,
    /// Lua search path and package policy toggles.
    pub scripting: ScriptingSettings,
    /// Unstable / preview feature flags.
    pub experimental: ExperimentalSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            kind: SETTINGS_KIND.into(),
            format_version: SETTINGS_FORMAT_VERSION,
            appearance: AppearanceSettings::default(),
            view: ViewSettings::default(),
            waveform: WaveformSettings::default(),
            selection: SelectionSettings::default(),
            audio: AudioSettings::default(),
            scripting: ScriptingSettings::default(),
            experimental: ExperimentalSettings::default(),
        }
    }
}

impl AppSettings {
    /// Normalize scripting paths using `config_dir` when provided.
    pub fn normalize(&mut self, config_dir: Option<&Path>) {
        self.scripting.normalize(config_dir);
    }
}

/// Theme name and light/dark mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppearanceSettings {
    /// Theme display name.
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
    /// Whether the explorer dock is open by default.
    pub explorer: bool,
    /// `"hidden"` or a detail-dock tab title.
    /// Legacy booleans still deserialize (`true` → Marker, `false` → hidden).
    #[serde(deserialize_with = "deserialize_detail_pref")]
    #[schemars(with = "String")]
    pub detail: String,
    /// `"hidden"` or a bottom-dock tab title.
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

impl ViewSettings {
    /// Whether the detail dock should open.
    pub fn detail_open(&self) -> bool {
        self.detail != DOCK_HIDDEN
    }

    /// Whether the bottom script dock should open.
    pub fn script_open(&self) -> bool {
        self.script != DOCK_HIDDEN
    }
}

fn deserialize_detail_pref<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_dock_pref(deserializer, DETAIL_DOCK_TRUE_TAB)
}

fn deserialize_script_pref<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_dock_pref(deserializer, SCRIPT_DOCK_TRUE_TAB)
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
    /// Five spectrum heatmap stops (quiet → loud).
    pub spectrum_gradient: [SpectrumGradientStopSettings; 5],
}

/// One spectrum heatmap stop stored in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SpectrumGradientStopSettings {
    /// Linear RGB in `0..=1`.
    pub rgb: [f32; 3],
    /// Intensity in dBFS (`SPECTRUM_GRADIENT_DB_FLOOR..=0`).
    pub db: f32,
}

impl Default for SpectrumGradientStopSettings {
    fn default() -> Self {
        Self {
            rgb: [0.0, 0.0, 0.0],
            db: SPECTRUM_GRADIENT_DB_FLOOR,
        }
    }
}

impl Default for WaveformSettings {
    fn default() -> Self {
        let floor = SPECTRUM_GRADIENT_DB_FLOOR;
        let span = 0.0 - floor;
        Self {
            representation: "peaks".into(),
            follow_playhead: true,
            peak_rendering: "threaded".into(),
            threaded_shell_value_reduce: DEFAULT_THREADED_SHELL_VALUE_REDUCE,
            threaded_ribbon_db: DEFAULT_THREADED_RIBBON_DB,
            spectrum_gradient: [
                SpectrumGradientStopSettings {
                    rgb: [0.02, 0.02, 0.08],
                    db: floor,
                },
                SpectrumGradientStopSettings {
                    rgb: [0.07, 0.17, 0.63],
                    db: floor + 0.25 * span,
                },
                SpectrumGradientStopSettings {
                    rgb: [0.12, 0.72, 0.83],
                    db: floor + 0.5 * span,
                },
                SpectrumGradientStopSettings {
                    rgb: [0.87, 0.92, 0.28],
                    db: floor + 0.75 * span,
                },
                SpectrumGradientStopSettings {
                    rgb: [1.0, 1.0, 1.0],
                    db: 0.0,
                },
            ],
        }
    }
}

impl WaveformSettings {
    /// Canonicalize a representation string.
    pub fn set_representation_str(&mut self, value: &str) {
        self.representation = match value {
            "spectrum" => "spectrum".into(),
            "peaks_spectrum" => "peaks_spectrum".into(),
            _ => "peaks".into(),
        };
    }

    /// Canonicalize a peak-rendering string.
    pub fn set_peak_rendering_str(&mut self, value: &str) {
        self.peak_rendering = match value {
            "simple" => "simple".into(),
            _ => "threaded".into(),
        };
    }

    /// Clamp and store threaded shell value reduction.
    pub fn set_threaded_shell_value_reduce(&mut self, value: f32) {
        self.threaded_shell_value_reduce = value.clamp(0.0, 1.0);
    }

    /// Clamp and store threaded ribbon dB.
    pub fn set_threaded_ribbon_db(&mut self, value: f32) {
        self.threaded_ribbon_db = value.clamp(-48.0, 0.0);
    }

    /// Update one stop's RGB (no neighbor normalization; hosts may re-normalize for paint).
    pub fn set_spectrum_stop_rgb(&mut self, index: usize, rgb: [f32; 3]) {
        if let Some(stop) = self.spectrum_gradient.get_mut(index) {
            stop.rgb = [
                rgb[0].clamp(0.0, 1.0),
                rgb[1].clamp(0.0, 1.0),
                rgb[2].clamp(0.0, 1.0),
            ];
        }
    }

    /// Update one stop's dB, clamped between neighbors and the axis ends.
    pub fn set_spectrum_stop_db(&mut self, index: usize, db: f32) {
        let n = self.spectrum_gradient.len();
        if index >= n {
            return;
        }
        let lo = if index == 0 {
            SPECTRUM_GRADIENT_DB_FLOOR
        } else {
            self.spectrum_gradient[index - 1].db
        };
        let hi = if index + 1 >= n {
            0.0
        } else {
            self.spectrum_gradient[index + 1].db
        };
        let clamped = if index == 0 {
            SPECTRUM_GRADIENT_DB_FLOOR
        } else if index + 1 >= n {
            0.0
        } else {
            db.clamp(lo, hi)
        };
        self.spectrum_gradient[index].db = clamped;
        self.spectrum_gradient[0].db = SPECTRUM_GRADIENT_DB_FLOOR;
        self.spectrum_gradient[n - 1].db = 0.0;
    }
}

/// Defaults for selection / marker targeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SelectionSettings {
    /// Snap selection edges to zero crossings.
    pub zero_crossing: bool,
    /// Snap selection edges to nearby markers.
    pub snap_to_marker: bool,
    /// Place markers at the pointer instead of the caret.
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

/// Preferred output device (`null` = system default). Device settings group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AudioSettings {
    /// Preferred device name substring, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    /// Output period in frames. When unset, the host probes the device default
    /// and snaps up to the nearest listed period of at least twice that size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_frames: Option<u32>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device: None,
            period_frames: None,
        }
    }
}

/// Lua module / workflow / resolver discovery preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ScriptingSettings {
    /// Extra folders after the user config directory (order preserved).
    ///
    /// Used for `package.path` / `package.cpath` templates and for scanning
    /// `resolver_*.lua` / `workflow_*.lua`. The config directory itself is
    /// always first and is not stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_path: Vec<String>,
    /// Append system-wide Lua path/cpath templates before `init.lua`.
    #[serde(default)]
    pub enable_system_package_paths: bool,
    /// Restore native C module loaders before `init.lua`.
    #[serde(default)]
    pub enable_native_modules: bool,
}

impl Default for ScriptingSettings {
    fn default() -> Self {
        Self {
            search_path: Vec::new(),
            enable_system_package_paths: false,
            enable_native_modules: false,
        }
    }
}

impl ScriptingSettings {
    /// Drop blanks and entries equal to the user config directory.
    pub fn normalize(&mut self, config_dir: Option<&Path>) {
        self.search_path.retain(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return false;
            }
            if let Some(config) = config_dir {
                let path = PathBuf::from(trimmed);
                if paths_equal(&path, config) {
                    return false;
                }
            }
            true
        });
        for entry in &mut self.search_path {
            *entry = entry.trim().to_string();
        }
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Unstable / preview preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExperimentalSettings {
    /// Feature flag bag.
    pub flags: FeatureFlags,
}

impl Default for ExperimentalSettings {
    fn default() -> Self {
        Self {
            flags: FeatureFlags::default(),
        }
    }
}
