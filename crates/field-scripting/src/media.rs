// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Shared media pool bindings.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, UNIX_EPOCH};

use mlua::{FromLua, Function, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_audio_io::{transcode, EncodeSpec, PcmFormat, TagMap, TranscodeRequest};
use field_audio_model::{MediaId, MediaRef};
use field_composition::media_ref_from_probed;

use crate::export::{
    profile_fields_from_lua, resolve_export_settings, ExportProfileDef, ExportSourceDefaults,
    LuaExportProfile,
};
use crate::host::host_from_lua;

/// Handle to pooled or detached (open-without-pool) media.
#[derive(Clone, Debug)]
pub enum LuaMedia {
    /// Entry in the shared media store.
    Pool(MediaId),
    /// Probed media not yet interned into the pool.
    Detached(MediaRef),
}

impl LuaMedia {
    /// Pooled handle.
    #[must_use]
    pub fn pooled(id: MediaId) -> Self {
        Self::Pool(id)
    }

    /// Media id when pooled; computed identity id when detached.
    pub fn id(&self) -> MediaId {
        match self {
            Self::Pool(id) => *id,
            Self::Detached(media) => media.id,
        }
    }

    fn with_ref<R>(
        &self,
        lua: &Lua,
        f: impl FnOnce(&MediaRef) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        match self {
            Self::Pool(id) => {
                let host = host_from_lua(lua)?;
                let media = host.with_backend(|b| b.get_media(*id))?;
                f(&media)
            }
            Self::Detached(media) => f(media),
        }
    }

    fn filesystem_path(&self, lua: &Lua) -> mlua::Result<PathBuf> {
        self.with_ref(lua, |media| {
            if media.path.as_os_str().is_empty() || media.url.is_memory() {
                return Err(mlua::Error::runtime(
                    "media has no filesystem path (memory media cannot be transcoded this way)",
                ));
            }
            Ok(media.path.clone())
        })
    }
}

impl FromLua for LuaMedia {
    fn from_lua(value: Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => ud.borrow::<Self>().map(|m| m.clone()),
            Value::String(s) => {
                let text = s.to_str()?.to_owned();
                let id = MediaId::from_str(&text).map_err(mlua::Error::runtime)?;
                Ok(Self::Pool(id))
            }
            other => Err(mlua::Error::runtime(format!(
                "expected media userdata or media id string, got {}",
                other.type_name()
            ))),
        }
    }
}

impl UserData for LuaMedia {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id().to_string()));
        fields.add_field_method_get("url", |lua, this| {
            this.with_ref(lua, |media| {
                Ok(crate::url::LuaUrl::from_location(media.url.clone()))
            })
        });
        fields.add_field_method_get("basename", |lua, this| {
            this.with_ref(lua, |media| Ok(media.basename.clone()))
        });
        fields.add_field_method_get("sample_rate", |lua, this| {
            this.with_ref(lua, |media| Ok(media.sample_rate as i64))
        });
        fields.add_field_method_get("channels", |lua, this| {
            this.with_ref(lua, |media| Ok(media.channel_count as i64))
        });
        fields.add_field_method_get("frames", |lua, this| {
            this.with_ref(lua, |media| Ok(media.frame_count as i64))
        });
        fields.add_field_method_get("bit_depth", |lua, this| {
            this.with_ref(lua, |media| Ok(media.bits_per_sample.map(|b| b as i64)))
        });
        fields.add_field_method_get("size_bytes", |lua, this| {
            this.with_ref(lua, |media| Ok(media.size_bytes as i64))
        });
        fields.add_field_method_get("modified", |lua, this| {
            this.with_ref(lua, |media| Ok(format_modified(media.modified)))
        });
        fields.add_field_method_get("container_format", |lua, this| {
            this.with_ref(lua, |media| Ok(media.container_format.clone()))
        });
        fields.add_field_method_get("codec", |lua, this| {
            this.with_ref(lua, |media| Ok(media.codec.clone()))
        });
        fields.add_field_method_get("duration", |lua, this| {
            this.with_ref(lua, |media| {
                Ok(media.frame_count as f64 / media.sample_rate.max(1) as f64)
            })
        });
        fields.add_field_method_get("variables", |lua, this| {
            this.with_ref(lua, |media| {
                Ok(crate::bindings::LuaBindings::media(
                    "source",
                    media.variables.clone(),
                    media.variables.is_scope_writable("source"),
                ))
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("bindings", |lua, this, scope: Option<String>| {
            this.with_ref(lua, |media| {
                let scope = scope.unwrap_or_else(|| "source".to_string());
                if scope != "source" && !scope.starts_with("source.") {
                    return Err(mlua::Error::runtime(format!(
                        "media bindings scope must be `source` or `source.*`, got `{scope}`"
                    )));
                }
                let writable = media.variables.is_scope_writable(&scope);
                Ok(crate::bindings::LuaBindings::media(
                    scope,
                    media.variables.clone(),
                    writable,
                ))
            })
        });
    }
}

/// Singleton handle to the process media pool.
#[derive(Clone, Copy, Debug)]
pub struct LuaMediaPool;

impl UserData for LuaMediaPool {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("add", |lua, _, value: Value| {
            let host = host_from_lua(lua)?;
            match value {
                Value::UserData(ud) if ud.is::<LuaMedia>() => {
                    let media = ud.borrow::<LuaMedia>()?.clone();
                    match media {
                        LuaMedia::Pool(id) => Ok(LuaMedia::Pool(id)),
                        LuaMedia::Detached(media_ref) => {
                            // Already enriched on open; intern shares the variable store.
                            let id = host.with_backend_mut(|b| b.intern_media(media_ref))?;
                            Ok(LuaMedia::Pool(id))
                        }
                    }
                }
                other => {
                    let path = crate::fs::path_from_lua(other)?;
                    let id = host.with_backend_mut(|b| b.add_media(&path))?;
                    let pooled = LuaMedia::Pool(id);
                    host.fire_enrich_media(pooled.clone());
                    Ok(pooled)
                }
            }
        });
        methods.add_method("remove", |lua, _, value: Value| {
            let media = LuaMedia::from_lua(value, lua)?;
            let id = match media {
                LuaMedia::Pool(id) => id,
                LuaMedia::Detached(_) => {
                    return Err(mlua::Error::runtime(
                        "cannot remove detached media (it is not in the pool)",
                    ));
                }
            };
            let host = host_from_lua(lua)?;
            host.with_backend_mut(|b| b.remove_media(id))?;
            Ok(())
        });
        methods.add_method("items", |lua, _, ()| {
            let host = host_from_lua(lua)?;
            let ids = host.with_backend(|b| b.list_media());
            let table = lua.create_table_with_capacity(ids.len(), 0)?;
            for (index, id) in ids.into_iter().enumerate() {
                table.set(index + 1, LuaMedia::pooled(id))?;
            }
            Ok(table)
        });
    }
}

fn format_modified(time: std::time::SystemTime) -> String {
    let dur = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    format!("{}", dur.as_secs())
}

fn open_media(lua: &Lua, path: &Path) -> mlua::Result<LuaMedia> {
    let probed = field_audio_io::probe_file(path)
        .map_err(|err| mlua::Error::runtime(format!("open media {}: {err:#}", path.display())))?;
    let media = LuaMedia::Detached(media_ref_from_probed(probed));
    host_from_lua(lua)?.fire_enrich_media(media.clone());
    Ok(media)
}

fn resolve_transcode_profile(
    lua: &Lua,
    media: &MediaRef,
    arg: Value,
) -> mlua::Result<(String, EncodeSpec, Vec<usize>)> {
    let source = ExportSourceDefaults {
        sample_rate: media.sample_rate.max(1),
        sample_format: media
            .bits_per_sample
            .and_then(PcmFormat::from_bits)
            .or(Some(PcmFormat::S24)),
        channel_count: media.channel_count.max(1),
        display_name: media.basename.clone(),
    };
    let profile = match arg {
        Value::Nil => ExportProfileDef::default(),
        Value::String(s) => {
            let name = s.to_str()?.to_owned();
            if name.is_empty() {
                ExportProfileDef::default()
            } else {
                host_from_lua(lua)?.export_profile(&name).ok_or_else(|| {
                    mlua::Error::runtime(format!("unknown export profile `{name}`"))
                })?
            }
        }
        Value::UserData(ud) if ud.is::<LuaExportProfile>() => {
            let name = ud.borrow::<LuaExportProfile>()?.name.clone();
            host_from_lua(lua)?
                .export_profile(&name)
                .ok_or_else(|| mlua::Error::runtime(format!("unknown export profile `{name}`")))?
        }
        Value::Table(table) => {
            // Anonymous options table (no name required).
            let name = table
                .get::<Value>("name")
                .ok()
                .and_then(|v| match v {
                    Value::String(s) => s.to_str().ok().map(|s| s.to_owned()),
                    _ => None,
                })
                .unwrap_or_else(|| "transcode".into());
            profile_fields_from_lua(table, name)?
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "expected export profile name, userdata, or options table, got {}",
                other.type_name()
            )));
        }
    };
    let resolved = resolve_export_settings(&source, &profile).map_err(mlua::Error::runtime)?;
    let spec = EncodeSpec {
        sample_rate: resolved.sample_rate,
        sample_format: resolved.sample_format,
        channel_count: resolved.channel_indices.len().max(1) as u16,
    };
    Ok((resolved.encoder_id, spec, resolved.channel_indices))
}

fn media_transcode(
    lua: &Lua,
    media: LuaMedia,
    dest: Value,
    profile: Value,
    on_progress: Option<Function>,
) -> mlua::Result<()> {
    let source_path = media.filesystem_path(lua)?;
    let dest_path = crate::fs::path_from_lua(dest)?;
    let media_ref = media.with_ref(lua, |m| Ok(m.clone()))?;
    let (encoder_id, spec, channel_indices) = resolve_transcode_profile(lua, &media_ref, profile)?;
    let tags = TagMap::new();
    match on_progress {
        Some(func) => {
            let mut cb = move |done: u64, total: u64| {
                let _ = func.call::<()>((done as i64, total as i64));
            };
            transcode(TranscodeRequest {
                source: &source_path,
                dest: &dest_path,
                encoder_id: &encoder_id,
                spec,
                channel_indices: &channel_indices,
                tags: &tags,
                block_frames: 0,
                on_progress: Some(&mut cb),
            })
        }
        None => transcode(TranscodeRequest {
            source: &source_path,
            dest: &dest_path,
            encoder_id: &encoder_id,
            spec,
            channel_indices: &channel_indices,
            tags: &tags,
            block_frames: 0,
            on_progress: None,
        }),
    }
    .map_err(|err| mlua::Error::runtime(format!("transcode failed: {err:#}")))
}

/// Background transcode job for UI `:defer` polling (does not block the host).
pub struct LuaTranscodeJob {
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl LuaTranscodeJob {
    fn start(
        source: PathBuf,
        dest: PathBuf,
        encoder_id: String,
        spec: EncodeSpec,
        channel_indices: Vec<usize>,
    ) -> Self {
        let done = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let done_w = done.clone();
        let total_w = total.clone();
        let finished_w = finished.clone();
        let error_w = error.clone();
        let join = thread::Builder::new()
            .name("fa-transcode".into())
            .spawn(move || {
                let tags = TagMap::new();
                let mut cb = |d: u64, t: u64| {
                    done_w.store(d, Ordering::Relaxed);
                    total_w.store(t, Ordering::Relaxed);
                };
                let result = transcode(TranscodeRequest {
                    source: &source,
                    dest: &dest,
                    encoder_id: &encoder_id,
                    spec,
                    channel_indices: &channel_indices,
                    tags: &tags,
                    block_frames: 0,
                    on_progress: Some(&mut cb),
                });
                if let Err(err) = result {
                    *error_w.lock().unwrap() = Some(format!("{err:#}"));
                }
                finished_w.store(true, Ordering::Release);
            })
            .expect("spawn fa-transcode");
        Self {
            done,
            total,
            finished,
            error,
            join: Mutex::new(Some(join)),
        }
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    fn join_thread(&self) {
        if let Some(handle) = self.join.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl UserData for LuaTranscodeJob {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("done", |_, this| {
            Ok(this.done.load(Ordering::Relaxed) as i64)
        });
        fields.add_field_method_get("total", |_, this| {
            Ok(this.total.load(Ordering::Relaxed) as i64)
        });
        fields.add_field_method_get("finished", |_, this| Ok(this.is_finished()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("progress", |_, this, ()| {
            Ok((
                this.done.load(Ordering::Relaxed) as i64,
                this.total.load(Ordering::Relaxed) as i64,
            ))
        });
        methods.add_method("error", |_, this, ()| Ok(this.take_error()));
        methods.add_method_mut("join", |_, this, ()| {
            this.join_thread();
            if let Some(err) = this.take_error() {
                return Err(mlua::Error::runtime(format!("transcode failed: {err}")));
            }
            Ok(())
        });
    }
}

fn media_begin_transcode(
    lua: &Lua,
    media: LuaMedia,
    dest: Value,
    profile: Value,
) -> mlua::Result<LuaTranscodeJob> {
    let source_path = media.filesystem_path(lua)?;
    let dest_path = crate::fs::path_from_lua(dest)?;
    let media_ref = media.with_ref(lua, |m| Ok(m.clone()))?;
    let (encoder_id, spec, channel_indices) = resolve_transcode_profile(lua, &media_ref, profile)?;
    Ok(LuaTranscodeJob::start(
        source_path,
        dest_path,
        encoder_id,
        spec,
        channel_indices,
    ))
}

/// Install `field.media`.
pub fn bind_media_module(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let media = lua.create_table()?;
    media.set(
        "shared_pool",
        lua.create_function(|_, ()| Ok(LuaMediaPool))?,
    )?;
    media.set(
        "open",
        lua.create_function(|lua, path: Value| {
            let path = crate::fs::path_from_lua(path)?;
            open_media(lua, &path)
        })?,
    )?;
    media.set(
        "transcode",
        lua.create_function(
            |lua, (media, dest, profile, on_progress): (Value, Value, Value, Value)| {
                let media = LuaMedia::from_lua(media, lua)?;
                let progress = match on_progress {
                    Value::Nil => None,
                    Value::Function(f) => Some(f),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "expected progress function or nil, got {}",
                            other.type_name()
                        )));
                    }
                };
                media_transcode(lua, media, dest, profile, progress)
            },
        )?,
    )?;
    media.set(
        "begin_transcode",
        lua.create_function(|lua, (media, dest, profile): (Value, Value, Value)| {
            let media = LuaMedia::from_lua(media, lua)?;
            media_begin_transcode(lua, media, dest, profile)
        })?,
    )?;
    field.set("media", media)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Write;
    use std::path::Path;
    use std::rc::Rc;

    use crate::backend::{BackendHandle, HeadlessBackend};
    use crate::host::{HostProfile, ScriptHost};
    use crate::world::HeadlessWorld;

    fn write_sine_wav(path: &Path, frames: u32, sample_rate: u32) {
        let channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let block_align = channels * bits_per_sample / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len = frames * u32::from(block_align);
        let mut out = std::fs::File::create(path).unwrap();
        out.write_all(b"RIFF").unwrap();
        out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        out.write_all(b"WAVE").unwrap();
        out.write_all(b"fmt ").unwrap();
        out.write_all(&16u32.to_le_bytes()).unwrap();
        out.write_all(&1u16.to_le_bytes()).unwrap();
        out.write_all(&channels.to_le_bytes()).unwrap();
        out.write_all(&sample_rate.to_le_bytes()).unwrap();
        out.write_all(&byte_rate.to_le_bytes()).unwrap();
        out.write_all(&block_align.to_le_bytes()).unwrap();
        out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        out.write_all(b"data").unwrap();
        out.write_all(&data_len.to_le_bytes()).unwrap();
        for i in 0..frames {
            let t = i as f32 / sample_rate as f32;
            let sample = (0.5 * (t * 440.0 * std::f32::consts::TAU).sin() * i16::MAX as f32) as i16;
            out.write_all(&sample.to_le_bytes()).unwrap();
        }
    }

    fn host_with_world() -> (ScriptHost, tempfile::TempDir, Rc<RefCell<HeadlessWorld>>) {
        let dir = tempfile::tempdir().unwrap();
        let world = Rc::new(RefCell::new(HeadlessWorld::new()));
        let backend: BackendHandle =
            Rc::new(RefCell::new(HeadlessBackend::from_world_rc(world.clone())));
        let mut host = ScriptHost::with_backend(
            HostProfile {
                name: "field-batch",
                config_dir: None,
            },
            backend,
        )
        .expect("host");
        host.load_init().expect("init");
        (host, dir, world)
    }

    #[test]
    fn open_does_not_intern_until_add() {
        let (mut host, dir, _) = host_with_world();
        let path = dir.path().join("take.wav");
        write_sine_wav(&path, 2048, 48_000);
        let path_lua = path.display().to_string().replace('\\', "\\\\");
        let out = host.eval(&format!(
            r#"
            local m = field.media.open("{path_lua}")
            assert(m.frames == 2048, m.frames)
            assert(m.sample_rate == 48000, m.sample_rate)
            assert(#field.media.shared_pool():items() == 0)
            local pooled = field.media.shared_pool():add(m)
            assert(#field.media.shared_pool():items() == 1)
            local again = field.media.shared_pool():add(pooled)
            assert(again.id == pooled.id)
            assert(#field.media.shared_pool():items() == 1)
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
    }

    #[test]
    fn transcode_rejects_unknown_profile() {
        let (mut host, dir, _) = host_with_world();
        let path = dir.path().join("take.wav");
        write_sine_wav(&path, 1024, 48_000);
        let path_lua = path.display().to_string().replace('\\', "\\\\");
        let out = host.eval(&format!(
            r#"
            local m = field.media.open("{path_lua}")
            field.media.transcode(m, "{path_lua}.out.wav", "no-such-profile")
            "#
        ));
        assert!(out.error.is_some(), "expected error");
        let err = out.error.unwrap();
        assert!(err.contains("unknown export profile"), "{err}");
    }

    #[test]
    fn transcode_with_rate_profile_writes_output() {
        let (mut host, dir, _) = host_with_world();
        let path = dir.path().join("take.wav");
        let dest = dir.path().join("out.wav");
        write_sine_wav(&path, 4800, 48_000);
        let path_lua = path.display().to_string().replace('\\', "\\\\");
        let dest_lua = dest.display().to_string().replace('\\', "\\\\");
        let out = host.eval(&format!(
            r#"
            field.exports.define({{
              name = "to-44k",
              encoder = "wav",
              sample_rate = 44100,
              sample_format = "S16",
            }})
            local m = field.media.open("{path_lua}")
            local last_done, last_total = 0, 0
            field.media.transcode(m, "{dest_lua}", "to-44k", function(done, total)
              last_done, last_total = done, total
            end)
            assert(last_total == 4800, last_total)
            assert(last_done == 4800, last_done)
            local out = field.media.open("{dest_lua}")
            assert(out.sample_rate == 44100, out.sample_rate)
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
    }
}
