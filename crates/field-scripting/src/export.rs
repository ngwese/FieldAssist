// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Export profile registration for `field.exports` and `composition:export`.

use std::path::{Path, PathBuf};

use field_audio_io::{encoder, snap_format, EncodeSpec, PcmFormat};
use field_composition::{export_to_path, Composition, ExportJob};
use field_session::DocumentId;
use mlua::{FromLua, Table, UserData, UserDataFields, UserDataMethods, Value};

use crate::fs::path_from_lua;
use crate::host::{host_from_lua, HostHandle};

/// Registered export profile from Lua.
#[derive(Clone, Debug, Default)]
pub struct ExportProfileDef {
    /// Profile name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Encoder id (`wav`, `flac`, `ogg`, …).
    pub encoder: Option<String>,
    /// PCM sample format when the encoder stores one.
    pub sample_format: Option<PcmFormat>,
    /// Output sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Channel selection (`None` = all channels).
    pub channels: Option<ExportChannels>,
    /// Output directory (joined with [`Self::filename`] when `path` is unset).
    pub directory: Option<PathBuf>,
    /// Output filename.
    pub filename: Option<String>,
    /// Full destination path (wins over directory + filename).
    pub path: Option<PathBuf>,
}

/// How an export profile selects composition channels.
#[derive(Clone, Debug)]
pub enum ExportChannels {
    /// Every composition channel.
    All,
    /// Explicit 0-based channel indices.
    Indices(Vec<usize>),
}

/// Defaults taken from the open composition / primary media.
///
/// These are layer 1 of export resolution (see [`resolve_export_settings`]):
/// used whenever a profile (or override) does not set the corresponding field.
#[derive(Clone, Debug)]
pub struct ExportSourceDefaults {
    /// Composition sample rate (Hz).
    pub sample_rate: u32,
    /// Preferred PCM format from primary media bit depth (when known).
    pub sample_format: Option<PcmFormat>,
    /// Composition channel count.
    pub channel_count: usize,
    /// Display name used when inventing a default filename.
    pub display_name: String,
}

impl ExportSourceDefaults {
    /// Build source defaults from an open composition.
    pub fn from_composition(composition: &Composition) -> Self {
        Self {
            sample_rate: composition.sample_rate().max(1),
            sample_format: composition.bit_depth().and_then(PcmFormat::from_bits),
            channel_count: composition.channel_count().max(1),
            display_name: composition.display_name(),
        }
    }
}

/// Concrete export settings after source ← profile resolution.
#[derive(Clone, Debug)]
pub struct ResolvedExportSettings {
    /// Encoder id.
    pub encoder_id: String,
    /// PCM format after encoder capability snap (`None` when the encoder does not store PCM).
    pub sample_format: Option<PcmFormat>,
    /// Output sample rate.
    pub sample_rate: u32,
    /// Selected composition channel indices (0-based).
    pub channel_indices: Vec<usize>,
    /// Optional full destination from the profile.
    pub path: Option<PathBuf>,
    /// Optional directory from the profile.
    pub directory: Option<PathBuf>,
    /// Optional filename from the profile.
    pub filename: Option<String>,
}

/// Resolve export settings by layering a profile onto composition/media defaults.
///
/// # Resolution order
///
/// Each field is taken from the **first** layer that explicitly supplies it:
///
/// 1. **Source defaults** ([`ExportSourceDefaults`]) from the composition /
///    primary media:
///    - `sample_rate` ← composition rate
///    - `sample_format` ← media bit depth (else preference `S24`)
///    - `channels` ← all composition channels
///    - `encoder` ← product default `"wav"` (not a media property)
///    - destination ← unset
/// 2. **Profile** ([`ExportProfileDef`]): each `Some` / set field overrides
///    the source value for that field only. Omitted profile fields keep the
///    source default.
///
/// After merging, the chosen encoder's capabilities **snap** `sample_format`
/// and validate rate / channel count.
///
/// Call-time Lua overrides and Export-sheet UI edits are applied the same way
/// as an additional profile layer on top of (1)+(2) before this function runs
/// (see [`apply_overrides`] / sheet `apply_profile`).
pub fn resolve_export_settings(
    source: &ExportSourceDefaults,
    profile: &ExportProfileDef,
) -> Result<ResolvedExportSettings, String> {
    // --- encoder ---
    let encoder_id = profile.encoder.clone().unwrap_or_else(|| "wav".to_string());
    let encoder = encoder(&encoder_id).ok_or_else(|| format!("unknown encoder `{encoder_id}`"))?;

    // --- channels: source = all; profile may narrow ---
    let channel_indices = match profile.channels.as_ref().unwrap_or(&ExportChannels::All) {
        ExportChannels::All => (0..source.channel_count).collect::<Vec<_>>(),
        ExportChannels::Indices(indices) => {
            for &index in indices {
                if index >= source.channel_count {
                    return Err(format!(
                        "channel index {index} is out of range for {} channels",
                        source.channel_count
                    ));
                }
            }
            if indices.is_empty() {
                return Err("select at least one channel".into());
            }
            indices.clone()
        }
    };

    // --- sample rate: source composition rate unless profile sets one ---
    let sample_rate = profile.sample_rate.unwrap_or(source.sample_rate).max(1);

    // --- sample format: source media bits / S24 unless profile sets one ---
    let preferred_format = profile
        .sample_format
        .or(source.sample_format)
        .or(Some(PcmFormat::S24));
    let sample_format = snap_format(encoder.capabilities(), preferred_format);

    let spec = EncodeSpec {
        sample_rate,
        sample_format,
        channel_count: channel_indices.len().max(1) as u16,
    };
    if !encoder.supports(&spec) {
        return Err(format!(
            "{} cannot encode the selected format, rate, or channel count",
            encoder.label()
        ));
    }

    Ok(ResolvedExportSettings {
        encoder_id,
        sample_format,
        sample_rate,
        channel_indices,
        path: profile.path.clone(),
        directory: profile.directory.clone(),
        filename: profile.filename.clone(),
    })
}

/// Handle to one registered export profile (looked up live by name).
#[derive(Clone, Debug)]
pub struct LuaExportProfile {
    name: String,
}

impl FromLua for LuaExportProfile {
    fn from_lua(value: Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => ud.borrow::<Self>().map(|profile| profile.clone()),
            Value::String(s) => Ok(Self {
                name: s.to_str()?.to_owned(),
            }),
            other => Err(mlua::Error::runtime(format!(
                "expected export profile userdata or profile name string, got {}",
                other.type_name()
            ))),
        }
    }
}

impl UserData for LuaExportProfile {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
        fields.add_field_method_get("description", |lua, this| {
            with_profile(lua, &this.name, |profile| Ok(profile.description.clone()))
        });
        fields.add_field_method_get("encoder", |lua, this| {
            with_profile(lua, &this.name, |profile| Ok(profile.encoder.clone()))
        });
        fields.add_field_method_get("sample_format", |lua, this| {
            with_profile(lua, &this.name, |profile| {
                Ok(profile
                    .sample_format
                    .map(|format| format.label().to_string()))
            })
        });
        fields.add_field_method_get("sample_rate", |lua, this| {
            with_profile(lua, &this.name, |profile| Ok(profile.sample_rate))
        });
        fields.add_field_method_get("channels", |lua, this| {
            with_profile(lua, &this.name, |profile| match &profile.channels {
                None | Some(ExportChannels::All) => Ok(Value::String(lua.create_string("all")?)),
                Some(ExportChannels::Indices(indices)) => {
                    let table = lua.create_table_with_capacity(indices.len(), 0)?;
                    for (i, index) in indices.iter().enumerate() {
                        table.set(i + 1, *index as i64)?;
                    }
                    Ok(Value::Table(table))
                }
            })
        });
        fields.add_field_method_get("directory", |lua, this| {
            with_profile(lua, &this.name, |profile| {
                Ok(profile
                    .directory
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()))
            })
        });
        fields.add_field_method_get("filename", |lua, this| {
            with_profile(lua, &this.name, |profile| Ok(profile.filename.clone()))
        });
        fields.add_field_method_get("path", |lua, this| {
            with_profile(lua, &this.name, |profile| {
                Ok(profile
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()))
            })
        });
    }
}

/// Process-wide export profile registry.
#[derive(Clone, Copy, Debug)]
pub struct LuaExportRegistry;

impl UserData for LuaExportRegistry {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("define", |lua, _, spec: Table| {
            let profile = profile_from_lua(spec)?;
            host_from_lua(lua)?.define_export_profile(profile);
            Ok(())
        });
        methods.add_method("remove", |lua, _, value: Value| {
            let profile = LuaExportProfile::from_lua(value, lua)?;
            host_from_lua(lua)?.remove_export_profile(&profile.name);
            Ok(())
        });
        methods.add_method("items", |lua, _, ()| {
            let host = host_from_lua(lua)?;
            let names = host.export_profile_names();
            let table = lua.create_table_with_capacity(names.len(), 0)?;
            for (index, name) in names.into_iter().enumerate() {
                table.set(index + 1, LuaExportProfile { name })?;
            }
            Ok(table)
        });
    }
}

fn with_profile<R>(
    lua: &mlua::Lua,
    name: &str,
    f: impl FnOnce(&ExportProfileDef) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    let profile = host
        .export_profile(name)
        .ok_or_else(|| mlua::Error::runtime(format!("unknown export profile `{name}`")))?;
    f(&profile)
}

/// Parse an export profile definition table from Lua (`name` required).
pub fn profile_from_lua(table: Table) -> mlua::Result<ExportProfileDef> {
    let name: String = table.get("name")?;
    if name.is_empty() {
        return Err(mlua::Error::runtime("export profile name is required"));
    }
    profile_fields_from_lua(table, name)
}

fn profile_fields_from_lua(table: Table, name: String) -> mlua::Result<ExportProfileDef> {
    let description: String = table.get("description").unwrap_or_default();
    let encoder = optional_string(table.get("encoder")?)?;
    let sample_format = optional_pcm_format(table.get("sample_format")?)?;
    let sample_rate = optional_u32(table.get("sample_rate")?)?;
    let channels = optional_channels(table.get("channels")?)?;
    let directory = optional_path(table.get("directory")?)?;
    let filename = optional_string(table.get("filename")?)?;
    let path = optional_path(table.get("path")?)?;
    Ok(ExportProfileDef {
        name,
        description,
        encoder,
        sample_format,
        sample_rate,
        channels,
        directory,
        filename,
        path,
    })
}

fn optional_string(value: Value) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(text) => Ok(Some(text.to_str()?.to_owned())),
        other => Err(mlua::Error::runtime(format!(
            "expected string, got {}",
            other.type_name()
        ))),
    }
}

fn optional_u32(value: Value) -> mlua::Result<Option<u32>> {
    match value {
        Value::Nil => Ok(None),
        Value::Integer(n) if n > 0 => Ok(Some(n as u32)),
        Value::Number(n) if n > 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 => {
            Ok(Some(n as u32))
        }
        other => Err(mlua::Error::runtime(format!(
            "sample_rate must be a positive integer, got {}",
            other.type_name()
        ))),
    }
}

fn optional_path(value: Value) -> mlua::Result<Option<PathBuf>> {
    match value {
        Value::Nil => Ok(None),
        other => Ok(Some(path_from_lua(other)?)),
    }
}

fn optional_pcm_format(value: Value) -> mlua::Result<Option<PcmFormat>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(text) => {
            let label = text.to_str()?;
            parse_pcm_format(label.as_ref())
                .map(Some)
                .ok_or_else(|| mlua::Error::runtime(format!("unknown sample_format `{label}`")))
        }
        other => Err(mlua::Error::runtime(format!(
            "sample_format must be a string, got {}",
            other.type_name()
        ))),
    }
}

fn parse_pcm_format(label: &str) -> Option<PcmFormat> {
    PcmFormat::ALL
        .into_iter()
        .find(|format| format.label().eq_ignore_ascii_case(label))
}

fn optional_channels(value: Value) -> mlua::Result<Option<ExportChannels>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(text) => {
            let text = text.to_str()?;
            if text.eq_ignore_ascii_case("all") {
                Ok(Some(ExportChannels::All))
            } else {
                Err(mlua::Error::runtime(format!(
                    "channels string must be \"all\", got `{text}`"
                )))
            }
        }
        Value::Table(table) => {
            let mut indices = Vec::new();
            for pair in table.sequence_values::<Value>() {
                indices.push(channel_index(pair?)?);
            }
            if indices.is_empty() {
                for pair in table.pairs::<Value, Value>() {
                    let (key, value) = pair?;
                    match value {
                        Value::Boolean(false) => continue,
                        Value::Boolean(true) | Value::Nil => {
                            indices.push(channel_index(key)?);
                        }
                        other => indices.push(channel_index(other)?),
                    }
                }
            }
            if indices.is_empty() {
                return Err(mlua::Error::runtime(
                    "channels table must list at least one channel index",
                ));
            }
            indices.sort_unstable();
            indices.dedup();
            Ok(Some(ExportChannels::Indices(indices)))
        }
        other => Err(mlua::Error::runtime(format!(
            "channels must be \"all\" or a table of indices, got {}",
            other.type_name()
        ))),
    }
}

fn channel_index(value: Value) -> mlua::Result<usize> {
    match value {
        Value::Integer(index) if index >= 0 => Ok(index as usize),
        Value::Number(index) if index >= 0.0 && index.fract() == 0.0 => Ok(index as usize),
        other => Err(mlua::Error::runtime(format!(
            "channel index must be a non-negative integer, got {}",
            other.type_name()
        ))),
    }
}

/// Merge override fields from `table` onto `base`.
fn apply_overrides(mut base: ExportProfileDef, table: &Table) -> mlua::Result<ExportProfileDef> {
    if let Ok(value) = table.get::<Value>("encoder") {
        if !matches!(value, Value::Nil) {
            base.encoder = optional_string(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("sample_format") {
        if !matches!(value, Value::Nil) {
            base.sample_format = optional_pcm_format(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("sample_rate") {
        if !matches!(value, Value::Nil) {
            base.sample_rate = optional_u32(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("channels") {
        if !matches!(value, Value::Nil) {
            base.channels = optional_channels(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("directory") {
        if !matches!(value, Value::Nil) {
            base.directory = optional_path(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("filename") {
        if !matches!(value, Value::Nil) {
            base.filename = optional_string(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("path") {
        if !matches!(value, Value::Nil) {
            base.path = optional_path(value)?;
        }
    }
    if let Ok(value) = table.get::<Value>("description") {
        if let Some(text) = optional_string(value)? {
            base.description = text;
        }
    }
    Ok(base)
}

fn lookup_profile(host: &HostHandle, name: &str) -> mlua::Result<ExportProfileDef> {
    host.export_profile(name)
        .ok_or_else(|| mlua::Error::runtime(format!("unknown export profile `{name}`")))
}

fn profile_ref_from_value(host: &HostHandle, value: Value) -> mlua::Result<ExportProfileDef> {
    match value {
        Value::String(name) => lookup_profile(host, name.to_str()?.as_ref()),
        Value::UserData(ud) => {
            let profile = ud.borrow::<LuaExportProfile>()?;
            lookup_profile(host, &profile.name)
        }
        other => Err(mlua::Error::runtime(format!(
            "profile must be a name or userdata, got {}",
            other.type_name()
        ))),
    }
}

/// Resolve a Lua `:export` argument into a concrete job for `composition`.
pub fn resolve_export_job(
    host: &HostHandle,
    composition: &Composition,
    arg: Value,
) -> mlua::Result<ExportJob> {
    let profile = match arg {
        Value::String(name) => lookup_profile(host, name.to_str()?.as_ref())?,
        Value::UserData(ud) => {
            let profile = ud.borrow::<LuaExportProfile>()?;
            lookup_profile(host, &profile.name)?
        }
        Value::Table(table) => {
            let profile_value: Value = table.get("profile")?;
            if !matches!(profile_value, Value::Nil) {
                apply_overrides(profile_ref_from_value(host, profile_value)?, &table)?
            } else if let Ok(name) = table.get::<String>("name") {
                if !name.is_empty() && host.export_profile(&name).is_some() {
                    // `export({ name = "wav48", path = ... })` treats `name` as a
                    // profile reference with overrides.
                    apply_overrides(lookup_profile(host, &name)?, &table)?
                } else {
                    // Ad-hoc options table (optionally with unused `name`).
                    profile_fields_from_lua(table, name)?
                }
            } else {
                profile_fields_from_lua(table, String::new())?
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "export expects a profile name, profile userdata, or options table, got {}",
                other.type_name()
            )))
        }
    };
    build_job(composition, profile)
}

fn build_job(composition: &Composition, profile: ExportProfileDef) -> mlua::Result<ExportJob> {
    let source = ExportSourceDefaults::from_composition(composition);
    let resolved = resolve_export_settings(&source, &profile).map_err(mlua::Error::runtime)?;
    let encoder = encoder(&resolved.encoder_id).ok_or_else(|| {
        mlua::Error::runtime(format!("unknown encoder `{}`", resolved.encoder_id))
    })?;
    let dest = resolve_dest_from_resolved(&resolved, &source, encoder.extension())?;
    Ok(ExportJob {
        encoder_id: resolved.encoder_id,
        spec: EncodeSpec {
            sample_rate: resolved.sample_rate,
            sample_format: resolved.sample_format,
            channel_count: resolved.channel_indices.len().max(1) as u16,
        },
        channel_indices: resolved.channel_indices,
        dest,
    })
}

fn resolve_dest_from_resolved(
    resolved: &ResolvedExportSettings,
    source: &ExportSourceDefaults,
    extension: &str,
) -> mlua::Result<PathBuf> {
    if let Some(path) = &resolved.path {
        if path.as_os_str().is_empty() {
            return Err(mlua::Error::runtime("export path is empty"));
        }
        return Ok(path.clone());
    }
    let directory = resolved.directory.as_ref().ok_or_else(|| {
        mlua::Error::runtime("export requires `path` or `directory` + `filename`")
    })?;
    let filename = resolved
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.{}", source.display_name, extension));
    Ok(directory.join(filename))
}

/// Run an export for an open composition document.
pub fn export_composition(lua: &mlua::Lua, id: DocumentId, arg: Value) -> mlua::Result<bool> {
    let host = host_from_lua(lua)?;
    host.with_backend(|backend| {
        backend.with_open_document(id, &mut |doc| {
            let composition = doc.composition.read().unwrap();
            let job = resolve_export_job(&host, &composition, arg.clone())?;
            if let Some(parent) = job.dest.parent() {
                ensure_parent_dir(parent)?;
            }
            export_to_path(&composition, &job, None, 0)
                .map_err(|err| mlua::Error::runtime(format!("export failed: {err:#}")))?;
            Ok(())
        })
    })?;
    Ok(true)
}

fn ensure_parent_dir(path: &Path) -> mlua::Result<()> {
    if path.as_os_str().is_empty() || path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .map_err(|err| mlua::Error::runtime(format!("create directory {}: {err}", path.display())))
}

/// Register export module functions on the host.
pub fn bind_exports(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let exports = lua.create_table()?;
    exports.set(
        "define",
        lua.create_function(|lua, spec: Table| {
            let profile = profile_from_lua(spec)?;
            host_from_lua(lua)?.define_export_profile(profile);
            Ok(())
        })?,
    )?;
    exports.set(
        "shared_registry",
        lua.create_function(|_, ()| Ok(LuaExportRegistry))?,
    )?;
    field.set("exports", exports)?;
    Ok(())
}

impl HostHandle {
    /// Register or replace an export profile definition.
    pub(crate) fn define_export_profile(&self, profile: ExportProfileDef) {
        let mut inner = self.inner.borrow_mut();
        if let Some(existing) = inner
            .export_profiles
            .iter_mut()
            .find(|p| p.name == profile.name)
        {
            *existing = profile;
        } else {
            inner.export_profiles.push(profile);
        }
    }

    /// Remove a registered export profile by name. No-op if missing.
    pub(crate) fn remove_export_profile(&self, name: &str) {
        let mut inner = self.inner.borrow_mut();
        inner.export_profiles.retain(|profile| profile.name != name);
    }

    /// Look up a registered export profile by name.
    pub(crate) fn export_profile(&self, name: &str) -> Option<ExportProfileDef> {
        self.inner
            .borrow()
            .export_profiles
            .iter()
            .find(|p| p.name == name)
            .cloned()
    }

    /// Names of all registered export profiles.
    pub(crate) fn export_profile_names(&self) -> Vec<String> {
        self.inner
            .borrow()
            .export_profiles
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }

    /// `(name, description)` pairs for registered export profiles.
    pub(crate) fn export_profile_choices(&self) -> Vec<(String, String)> {
        self.inner
            .borrow()
            .export_profiles
            .iter()
            .map(|p| (p.name.clone(), p.description.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> ExportSourceDefaults {
        ExportSourceDefaults {
            sample_rate: 48_000,
            sample_format: Some(PcmFormat::S24),
            channel_count: 2,
            display_name: "take".into(),
        }
    }

    #[test]
    fn empty_profile_keeps_source_defaults() {
        let resolved = resolve_export_settings(&source(), &ExportProfileDef::default()).unwrap();
        assert_eq!(resolved.encoder_id, "wav");
        assert_eq!(resolved.sample_rate, 48_000);
        assert_eq!(resolved.sample_format, Some(PcmFormat::S24));
        assert_eq!(resolved.channel_indices, vec![0, 1]);
    }

    #[test]
    fn profile_overrides_only_set_fields() {
        let profile = ExportProfileDef {
            encoder: Some("flac".into()),
            sample_rate: Some(44_100),
            // sample_format / channels omitted → source
            ..ExportProfileDef::default()
        };
        let resolved = resolve_export_settings(&source(), &profile).unwrap();
        assert_eq!(resolved.encoder_id, "flac");
        assert_eq!(resolved.sample_rate, 44_100);
        assert_eq!(resolved.sample_format, Some(PcmFormat::S24));
        assert_eq!(resolved.channel_indices, vec![0, 1]);
    }

    #[test]
    fn profile_channel_subset_replaces_all() {
        let profile = ExportProfileDef {
            channels: Some(ExportChannels::Indices(vec![1])),
            ..ExportProfileDef::default()
        };
        let resolved = resolve_export_settings(&source(), &profile).unwrap();
        assert_eq!(resolved.channel_indices, vec![1]);
    }
}
