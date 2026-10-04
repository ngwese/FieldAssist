// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Composition userdata and document helpers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_audio_model::{ChannelScope, RegionId, SELECTION_COLLECTION};
use field_composition::{render_outputs, Composition, OutputResult, RenderOutput, RenderPlan};
use field_session::DocumentId;

use crate::export::{apply_overrides, build_job, profile_fields_from_lua, profile_ref_from_value};
use crate::host::host_from_lua;
use crate::marker::{
    color_from_value, color_to_lua, list_markers, marker_id_from_lua, parse_add_marker, LuaMarker,
};
use crate::media::LuaMedia;
use crate::region::LuaRegion;
use crate::selection::{channels_from_lua, collection_name_from_lua, optional_i64, LuaCollection};
use crate::util::{optional_lua_string, string_map_from_lua, string_map_to_lua};
use crate::world::OpenDocument;

/// Open document handle.
#[derive(Clone, Copy, Debug)]
pub struct LuaComposition {
    /// Session document id.
    pub id: DocumentId,
}

impl FromLua for LuaComposition {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(data) => data.borrow::<Self>().map(|this| *this),
            _ => Err(mlua::Error::runtime("expected a composition")),
        }
    }
}

impl UserData for LuaComposition {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |lua, this| {
            host_from_lua(lua)?
                .display_name(this.id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))
        });
        fields.add_field_method_set("name", |lua, this, value: String| {
            host_from_lua(lua)?.set_display_name(this.id, value)
        });
        fields.add_field_method_get("url", |lua, this| {
            Ok(host_from_lua(lua)?
                .document_path(this.id)
                .map(|path| crate::url::LuaUrl::from_path(&path)))
        });
        fields.add_field_method_get("id", |lua, this| {
            host_from_lua(lua)?.composition_id_string(this.id)
        });
        fields.add_field_method_get("group", |lua, this| {
            host_from_lua(lua)?.document_group(this.id)
        });
        fields.add_field_method_set("group", |lua, this, value: Value| {
            host_from_lua(lua)?.set_document_group(this.id, optional_lua_string(value)?)
        });
        fields.add_field_method_get("state", |lua, this| {
            host_from_lua(lua)?.document_state(this.id)
        });
        fields.add_field_method_set("state", |lua, this, value: Value| {
            host_from_lua(lua)?.set_document_state(this.id, optional_lua_string(value)?)
        });
        fields.add_field_method_get("properties", |lua, this| {
            string_map_to_lua(lua, &host_from_lua(lua)?.document_properties(this.id)?)
        });
        fields.add_field_method_set("properties", |lua, this, value: Value| {
            host_from_lua(lua)?.set_document_properties(this.id, string_map_from_lua(value)?)
        });
        fields.add_field_method_get("variables", |_lua, this| {
            Ok(crate::bindings::LuaBindings::composition(this.id))
        });
        fields.add_field_method_set("variables", |lua, this, value: Value| {
            host_from_lua(lua)?.set_composition_variables(
                this.id,
                crate::bindings::variables_table_from_value(value, "composition")?,
            )
        });
        fields.add_field_method_get("frames", |lua, this| {
            with_document(lua, this.id, |doc| Ok(doc.frames() as i64))
        });
        fields.add_field_method_get("sample_rate", |lua, this| {
            with_document(lua, this.id, |doc| Ok(doc.sample_rate() as i64))
        });
        fields.add_field_method_get("channels", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc.composition.read().unwrap().channel_count() as i64)
            })
        });
        fields.add_field_method_get("selection", |_, this| {
            Ok(LuaCollection {
                doc: this.id,
                name: SELECTION_COLLECTION.into(),
            })
        });
        fields.add_field_method_set("selection", |lua, this, value: Value| {
            with_document_mut(lua, this.id, |doc| apply_selection(lua, doc, value))
        });
        fields.add_field_method_get("position", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc.position.as_ref().map(|pos| pos.sample as i64))
            })
        });
        fields.add_field_method_set("position", |lua, this, sample: i64| {
            with_document_mut(lua, this.id, |doc| {
                doc.set_position(sample.max(0) as usize, ChannelScope::all());
                Ok(())
            })
        });
        fields.add_field_method_get("collections", |lua, this| {
            with_document(lua, this.id, |doc| Ok(doc.collection_names()))
        });
        fields.add_field_method_get("markers", |lua, this| {
            with_document(lua, this.id, |doc| Ok(list_markers(doc, this.id)))
        });
        fields.add_field_method_get("marker_types", |lua, this| {
            with_document(lua, this.id, |doc| {
                let table = lua.create_table()?;
                for (i, ty) in doc.marker_types().iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("name", ty.name.clone())?;
                    row.set("color", color_to_lua(lua, ty.color)?)?;
                    table.set(i + 1, row)?;
                }
                Ok(table)
            })
        });
        fields.add_field_method_get("media", |lua, this| {
            with_document(lua, this.id, |doc| {
                let refs = doc.composition.read().unwrap().pool().into_refs();
                let table = lua.create_table_with_capacity(refs.len(), 0)?;
                for (index, media) in refs.into_iter().enumerate() {
                    table.set(index + 1, LuaMedia::pooled(media.id))?;
                }
                Ok(table)
            })
        });
        // ── Desktop-compatible media / channel-layout fields ─────────────────
        fields.add_field_method_get("codec", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc.composition.read().unwrap().codec())
            })
        });
        fields.add_field_method_get("bit_depth", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .bit_depth()
                    .map(|b| b as i64))
            })
        });
        fields.add_field_method_get("basename", |lua, this| {
            with_document(lua, this.id, |doc| {
                let path = doc
                    .path
                    .as_ref()
                    .filter(|p| !p.to_string_lossy().starts_with("memory:"));
                Ok(path.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())))
            })
        });
        fields.add_field_method_get("dirname", |lua, this| {
            with_document(lua, this.id, |doc| {
                let path = doc
                    .path
                    .as_ref()
                    .filter(|p| !p.to_string_lossy().starts_with("memory:"));
                Ok(path.and_then(|p| {
                    p.parent()
                        .map(|parent| parent.to_string_lossy().into_owned())
                        .filter(|s| !s.is_empty())
                }))
            })
        });
        fields.add_field_method_get("duration", |lua, this| {
            with_document(lua, this.id, |doc| {
                let sample_rate = doc.sample_rate();
                let frames = doc.frames();
                if sample_rate == 0 {
                    return Ok(0.0f64);
                }
                Ok(frames as f64 / sample_rate as f64)
            })
        });
        fields.add_field_method_get("channel_layout", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .channel_layout()
                    .map(str::to_string))
            })
        });
        fields.add_field_method_set("channel_layout", |lua, this, value: mlua::Value| {
            let name = optional_lua_string(value)?;
            host_from_lua(lua)?.choose_layout(this.id, name.as_deref())
        });
        fields.add_field_method_get("monitor_chain", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .monitor_chain()
                    .map(str::to_string))
            })
        });
        fields.add_field_method_set("monitor_chain", |lua, this, value: mlua::Value| {
            with_document_mut(lua, this.id, |doc| {
                let chain = optional_lua_string(value)?;
                doc.composition.write().unwrap().set_monitor_chain(chain);
                Ok(())
            })
        });
        fields.add_field_method_get("playback_channels", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .playback_channels()
                    .map(|ch| ch.to_vec()))
            })
        });
        fields.add_field_method_set("playback_channels", |lua, this, value: mlua::Value| {
            with_document_mut(lua, this.id, |doc| {
                let channels: Option<Vec<usize>> = match value {
                    mlua::Value::Nil => None,
                    mlua::Value::String(s) if s.to_str()? == "all" => None,
                    mlua::Value::Table(t) => {
                        let mut ch = Vec::new();
                        for v in t.sequence_values::<mlua::Value>() {
                            match v? {
                                mlua::Value::Integer(i) => ch.push(i.max(0) as usize),
                                mlua::Value::Number(n) => ch.push(n.max(0.0) as usize),
                                other => {
                                    return Err(mlua::Error::runtime(format!(
                                        "playback_channels entries must be integers, got {}",
                                        other.type_name()
                                    )))
                                }
                            }
                        }
                        if ch.is_empty() {
                            None
                        } else {
                            Some(ch)
                        }
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "playback_channels must be a table, \"all\", or nil, got {}",
                            other.type_name()
                        )))
                    }
                };
                doc.composition
                    .write()
                    .unwrap()
                    .set_playback_channels(channels);
                Ok(())
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("variable_resolver", |lua, this, ()| {
            let bindings = crate::variables::composition_site_detached(lua, this.id)?;
            Ok(crate::variables::LuaVariableResolver::new(bindings))
        });
        methods.add_method(
            "select",
            |lua, this, (start, stop, channels): (i64, i64, Value)| {
                let channels = channels_from_lua(lua, channels)?;
                with_document_mut(lua, this.id, |doc| {
                    doc.select_range(start.max(0) as usize, stop.max(0) as usize, channels);
                    Ok(())
                })
            },
        );
        methods.add_method("select_all", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.select_all();
                Ok(())
            })
        });
        methods.add_method("clear_selection", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.clear_selection();
                Ok(())
            })
        });
        methods.add_method("collection", |lua, this, name: String| {
            if name != SELECTION_COLLECTION {
                with_document_mut(lua, this.id, |doc| {
                    doc.ensure_named_collection(&name);
                    Ok(())
                })?;
            }
            Ok(LuaCollection { doc: this.id, name })
        });
        methods.add_method("add_region", |lua, this, spec: Table| {
            let start: i64 = spec.get("start")?;
            let stop: i64 = spec.get("stop")?;
            let channels = channels_from_lua(lua, spec.get("channels")?)?;
            let label: Option<String> = spec.get("label")?;
            let collection = collection_name_from_lua(spec.get("collection")?)?;
            let id = with_document_mut(lua, this.id, |doc| {
                Ok(doc.add_labeled_region(
                    start.max(0) as usize,
                    stop.max(0) as usize,
                    channels,
                    label,
                    &collection,
                ))
            })?;
            Ok(LuaRegion {
                doc: this.id,
                collection,
                id,
            })
        });
        methods.add_method("remove_region", |lua, this, id: i64| {
            with_document_mut(lua, this.id, |doc| {
                Ok(doc.remove_region(RegionId(id.max(0) as u64)))
            })
        });
        methods.add_method("add_marker", |lua, this, args: mlua::MultiValue| {
            let spec = parse_add_marker(args)?;
            let marker_id = with_document_mut(lua, this.id, |doc| {
                if let Some(color) = spec.color {
                    doc.add_marker_type(&spec.marker_type, color);
                }
                Ok(doc.add_marker(spec.frame, &spec.marker_type, spec.note))
            })?;
            Ok(marker_id.map(|id| LuaMarker { doc: this.id, id }))
        });
        methods.add_method("remove_marker", |lua, this, target: Value| {
            let id = marker_id_from_lua(target)?;
            with_document_mut(lua, this.id, |doc| Ok(doc.remove_marker(id)))
        });
        methods.add_method(
            "remove_marker_at",
            |lua, this, (frame, marker_type): (i64, Option<String>)| {
                let frame = frame.max(0) as usize;
                with_document_mut(lua, this.id, |doc| {
                    Ok(match marker_type.as_deref() {
                        Some(marker_type) => doc.remove_marker_at_type(frame, marker_type),
                        None => doc.remove_marker_at(frame),
                    })
                })
            },
        );
        methods.add_method("remove_marker_by_type", |lua, this, marker_type: String| {
            with_document_mut(lua, this.id, |doc| {
                Ok(doc.remove_marker_by_type(&marker_type))
            })
        });
        methods.add_method(
            "marker_at",
            |lua, this, (frame, marker_type): (i64, Option<String>)| {
                let frame = frame.max(0) as u64;
                with_document(lua, this.id, |doc| {
                    let composition = doc.composition.read().unwrap();
                    let markers = composition.markers();
                    let found = match marker_type.as_deref() {
                        Some(marker_type) => markers.get_at_type(frame, marker_type),
                        None => markers.get_at(frame),
                    };
                    Ok(found.map(|marker| LuaMarker {
                        doc: this.id,
                        id: marker.id,
                    }))
                })
            },
        );
        methods.add_method(
            "add_marker_type",
            |lua, this, (name, color): (String, Value)| {
                let color = color_from_value(color)?.ok_or_else(|| {
                    mlua::Error::runtime("add_marker_type needs a color {r,g,b,a}")
                })?;
                with_document_mut(lua, this.id, |doc| Ok(doc.add_marker_type(&name, color)))
            },
        );
        methods.add_method("remove_marker_type", |lua, this, name: String| {
            with_document_mut(lua, this.id, |doc| Ok(doc.remove_marker_type(&name)))
        });
        methods.add_method("save", |lua, this, ()| -> mlua::Result<()> {
            let _ = (lua, this);
            Err(mlua::Error::runtime(
                "composition save is not implemented in the headless host",
            ))
        });
        methods.add_method("close", |lua, this, opts: Value| {
            let discard = match opts {
                Value::Nil => false,
                Value::Table(table) => table.get::<Option<bool>>("discard")?.unwrap_or(false),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "close options must be a table or nil, got {}",
                        other.type_name()
                    )));
                }
            };
            host_from_lua(lua)?.close_composition(this.id, discard)
        });
        methods.add_method("replace", |lua, this, path: Value| {
            let path = crate::fs::path_from_lua(path)?;
            host_from_lua(lua)?
                .replace_composition(this.id, &path.to_string_lossy())
                .map(|id| LuaComposition { id })
        });
        methods.add_method("undo", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| Ok(doc.edit_undo()))
        });
        methods.add_method("redo", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| Ok(doc.edit_redo()))
        });
        methods.add_method("cut", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_cut();
                Ok(true)
            })
        });
        methods.add_method("copy", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_copy();
                Ok(true)
            })
        });
        methods.add_method("paste", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_paste();
                Ok(true)
            })
        });
        methods.add_method("clear", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_clear();
                Ok(true)
            })
        });
        methods.add_method("remove", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_remove();
                Ok(true)
            })
        });
        methods.add_method("duplicate", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_duplicate();
                Ok(true)
            })
        });
        methods.add_method("trim", |lua, this, ()| {
            with_document_mut(lua, this.id, |doc| {
                doc.edit_trim();
                Ok(true)
            })
        });
        methods.add_method("export", |lua, this, arg: Value| {
            crate::export::export_composition(lua, this.id, arg)
        });
        methods.add_method("render", |lua, this, arg: Value| {
            render_composition(lua, this.id, arg, false).map(|_| true)
        });
        methods.add_method("begin_render", |lua, this, arg: Value| {
            render_composition_async(lua, this.id, arg)
        });
    }
}

// ── Render helpers ────────────────────────────────────────────────────────────

/// Completed render output entry: `(path_string, status_tag, detail)`.
///
/// `status_tag` is `"ok"`, `"skipped"`, or `"failed"`.
pub struct RenderOutputEntry {
    /// Destination path as a UTF-8 string.
    pub path: String,
    /// One of `"ok"`, `"skipped"`, `"failed"`.
    pub status: &'static str,
    /// Additional detail for skipped/failed entries.
    pub detail: String,
}

impl From<(PathBuf, OutputResult)> for RenderOutputEntry {
    fn from((path, result): (PathBuf, OutputResult)) -> Self {
        let path = path.to_string_lossy().into_owned();
        match result {
            OutputResult::Written => Self {
                path,
                status: "ok",
                detail: String::new(),
            },
            OutputResult::Skipped(msg) => Self {
                path,
                status: "skipped",
                detail: msg,
            },
            OutputResult::Failed(msg) => Self {
                path,
                status: "failed",
                detail: msg,
            },
        }
    }
}

/// Background render job for UI `:defer` polling.
pub struct LuaRenderJob {
    done: Arc<AtomicU64>,
    total: u64,
    finished: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    outputs: Arc<Mutex<Vec<RenderOutputEntry>>>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
}

impl LuaRenderJob {
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

impl UserData for LuaRenderJob {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("done", |_, this| {
            Ok(this.done.load(Ordering::Relaxed) as i64)
        });
        fields.add_field_method_get("total", |_, this| Ok(this.total as i64));
        fields.add_field_method_get("finished", |_, this| Ok(this.is_finished()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("progress", |_, this, ()| {
            Ok((this.done.load(Ordering::Relaxed) as i64, this.total as i64))
        });
        methods.add_method("error", |_, this, ()| Ok(this.take_error()));
        methods.add_method_mut("join", |_, this, ()| {
            this.join_thread();
            if let Some(err) = this.take_error() {
                return Err(mlua::Error::runtime(format!("render failed: {err}")));
            }
            Ok(())
        });
        // Returns `{ { path, status, detail }, … }` once finished.
        methods.add_method("results", |lua, this, ()| {
            let entries = this.outputs.lock().unwrap();
            let table = lua.create_table_with_capacity(entries.len(), 0)?;
            for (i, entry) in entries.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("path", entry.path.clone())?;
                row.set("status", entry.status)?;
                if !entry.detail.is_empty() {
                    row.set("detail", entry.detail.clone())?;
                }
                table.set(i + 1, row)?;
            }
            Ok(table)
        });
    }
}

/// Parse a Lua render-output spec table into a [`RenderOutput`] + collect DSP.
///
/// Each output spec may have:
/// - `path` (string, required)
/// - `profile` (name string, profile userdata, or options table, required)
/// - `chain` (string, optional): `"foa"` | `"foa_fuma"` | `"ms"`
/// - `params` (table `string → number`, optional): DSP param overrides
fn parse_render_output(
    lua: &Lua,
    spec: Table,
    composition: &Composition,
    composition_id: Option<DocumentId>,
) -> mlua::Result<RenderOutput> {
    let host = host_from_lua(lua)?;

    // path ─────────────────────────────────────────────────────────────────
    let path_val: Value = spec.get("path")?;
    let path = crate::fs::path_from_lua(path_val)?;

    // profile ──────────────────────────────────────────────────────────────
    let profile_val: Value = spec.get("profile")?;
    let mut profile = match profile_val {
        Value::Nil => {
            return Err(mlua::Error::runtime(
                "render output requires a `profile` (name or options table)",
            ))
        }
        Value::String(name) => profile_ref_from_value(&host, Value::String(name))?,
        Value::UserData(ud) => profile_ref_from_value(&host, Value::UserData(ud))?,
        Value::Table(table) => {
            let profile_field: Value = table.get("profile")?;
            if !matches!(profile_field, Value::Nil) {
                apply_overrides(profile_ref_from_value(&host, profile_field)?, &table)?
            } else {
                profile_fields_from_lua(table, String::new())?
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
            "render output `profile` must be a name, profile userdata, or options table, got {}",
            other.type_name()
        )))
        }
    };

    // Force the destination path from the output spec (overrides profile path).
    profile.path = Some(path);

    // Build ExportJob (resolves variables, tags, spec, dest).
    let job = build_job(lua, &host, composition, composition_id, profile)?;

    // chain + params ───────────────────────────────────────────────────────
    let chain: Option<String> = match spec.get::<Value>("chain")? {
        Value::Nil => None,
        Value::String(s) => Some(s.to_str()?.to_owned()),
        other => {
            return Err(mlua::Error::runtime(format!(
                "render output `chain` must be a string or nil, got {}",
                other.type_name()
            )))
        }
    };

    let params: HashMap<String, f32> = match spec.get::<Value>("params")? {
        Value::Nil => HashMap::new(),
        Value::Table(table) => {
            let mut map = HashMap::new();
            for pair in table.pairs::<String, Value>() {
                let (key, value) = pair?;
                let v = match value {
                    Value::Number(n) => n as f32,
                    Value::Integer(n) => n as f32,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "render params values must be numbers, got {} for key `{key}`",
                            other.type_name()
                        )))
                    }
                };
                map.insert(key, v);
            }
            map
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "render output `params` must be a table or nil, got {}",
                other.type_name()
            )))
        }
    };

    // Offline DSP (via backend) ────────────────────────────────────────────
    let dsp = chain.as_deref().and_then(|chain| {
        host.with_backend(|backend| {
            backend.create_offline_dsp(chain, job.spec.sample_rate, &params)
        })
    });

    // Profile/export job channel_count reflects the composition (pre-DSP).
    // After FOA/M/S mixdown the encoder must use the DSP output channel count.
    let mut spec = job.spec;
    if let Some(ref dsp) = dsp {
        spec.channel_count = dsp.num_outputs() as u16;
    }

    Ok(RenderOutput {
        path: job.dest,
        encoder_id: job.encoder_id,
        spec,
        channel_indices: job.channel_indices,
        tags: job.tags,
        dsp,
    })
}

/// Parse a render plan from a Lua options table.
fn parse_render_plan(
    lua: &Lua,
    arg: Value,
    composition: &Composition,
    composition_id: Option<DocumentId>,
) -> mlua::Result<RenderPlan> {
    let options = match arg {
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "render expects an options table, got {}",
                other.type_name()
            )))
        }
    };

    let outputs_val: Value = options.get("outputs")?;
    let outputs_table = match outputs_val {
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "render `outputs` must be a table, got {}",
                other.type_name()
            )))
        }
    };

    let mut outputs: Vec<RenderOutput> = Vec::new();
    for pair in outputs_table.sequence_values::<Value>() {
        let entry = pair?;
        match entry {
            Value::Table(spec) => {
                outputs.push(parse_render_output(lua, spec, composition, composition_id)?);
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "render outputs entries must be tables, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if outputs.is_empty() {
        return Err(mlua::Error::runtime("render requires at least one output"));
    }

    Ok(RenderPlan::new(outputs))
}

/// Synchronous render: build the plan and run on the calling thread.
fn render_composition(
    lua: &Lua,
    id: DocumentId,
    arg: Value,
    _return_results: bool,
) -> mlua::Result<()> {
    let host = host_from_lua(lua)?;
    host.with_backend(|backend| {
        backend.with_open_document(id, &mut |doc| {
            let composition = doc.composition.read().unwrap();
            let mut plan = parse_render_plan(lua, arg.clone(), &composition, Some(id))?;
            // Ensure parent directories exist.
            for output in &plan.outputs {
                if let Some(parent) = output.path.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|err| {
                            mlua::Error::runtime(format!(
                                "create directory {}: {err}",
                                parent.display()
                            ))
                        })?;
                    }
                }
            }
            render_outputs(&composition, &mut plan, None, 0)
                .map_err(|err| mlua::Error::runtime(format!("render failed: {err:#}")))?;
            Ok(())
        })
    })?;
    Ok(())
}

/// Async render: read audio synchronously then process on a background thread.
fn render_composition_async(lua: &Lua, id: DocumentId, arg: Value) -> mlua::Result<LuaRenderJob> {
    let host = host_from_lua(lua)?;

    // Collect data from the composition on the calling thread.
    let mut planes_out: Option<Vec<Vec<f32>>> = None;
    let mut source_rate_out: u32 = 0;
    let mut frames_out: u64 = 0;
    let mut plan_out: Option<RenderPlan> = None;

    host.with_backend(|backend| {
        backend.with_open_document(id, &mut |doc| {
            let composition = doc.composition.read().unwrap();
            let plan = parse_render_plan(lua, arg.clone(), &composition, Some(id))?;

            // Ensure parent directories exist.
            for output in &plan.outputs {
                if let Some(parent) = output.path.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|err| {
                            mlua::Error::runtime(format!(
                                "create directory {}: {err}",
                                parent.display()
                            ))
                        })?;
                    }
                }
            }

            // Read audio amortized before handing off to background thread.
            let frames = composition.frames();
            let ch = composition.channel_count();
            frames_out = frames;
            source_rate_out = composition.sample_rate();
            let mut planes = vec![vec![0.0f32; frames as usize]; ch];
            {
                let mut refs: Vec<&mut [f32]> =
                    planes.iter_mut().map(|p| p.as_mut_slice()).collect();
                composition
                    .read_planar(0, frames, &mut refs)
                    .map_err(|err| {
                        mlua::Error::runtime(format!("read composition for render: {err:#}"))
                    })?;
            }

            planes_out = Some(planes);
            plan_out = Some(plan);
            Ok(())
        })
    })?;

    let planes = planes_out.ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
    let plan = plan_out.ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
    let source_rate = source_rate_out;
    let n_outputs = plan.outputs.len();
    let frame_progress = field_composition::FrameProgress::new(frames_out, n_outputs);
    let done = frame_progress.done.clone();
    let total = frame_progress.total;

    let finished = Arc::new(AtomicBool::new(false));
    let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let outputs: Arc<Mutex<Vec<RenderOutputEntry>>> = Arc::new(Mutex::new(Vec::new()));

    let finished_w = finished.clone();
    let error_w = error.clone();
    let outputs_w = outputs.clone();

    let join = thread::Builder::new()
        .name("fa-render".into())
        .spawn(move || {
            let mut plan = plan;
            match field_composition::render_outputs_from_planes_with_progress(
                planes,
                source_rate,
                &mut plan,
                None,
                0,
                Some(&frame_progress),
            ) {
                Ok(results) => {
                    let mut entries: Vec<RenderOutputEntry> =
                        results.outputs.into_iter().map(Into::into).collect();
                    outputs_w.lock().unwrap().append(&mut entries);
                }
                Err(err) => {
                    *error_w.lock().unwrap() = Some(format!("{err:#}"));
                }
            }
            finished_w.store(true, Ordering::Release);
        })
        .map_err(|err| mlua::Error::runtime(format!("spawn render thread: {err}")))?;

    Ok(LuaRenderJob {
        done,
        total,
        finished,
        error,
        outputs,
        join: Mutex::new(Some(join)),
    })
}

/// Install `field.composition`.
pub fn bind_composition_module(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let composition = lua.create_table()?;
    composition.set(
        "open",
        lua.create_function(|lua, path: Value| {
            let path = crate::fs::path_from_lua(path)?;
            host_from_lua(lua)?
                .open_path(&path.to_string_lossy())
                .map(|id| LuaComposition { id })
        })?,
    )?;
    field.set("composition", composition)?;
    Ok(())
}

/// Read-only access to an open document.
///
/// Delegates to [`crate::backend::ScriptBackend::with_open_document`].
pub fn with_document<R>(
    lua: &Lua,
    id: DocumentId,
    f: impl FnOnce(&OpenDocument) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    let mut f = Some(f);
    let mut result: Option<mlua::Result<R>> = None;
    host.with_backend(|backend| {
        backend.with_open_document(id, &mut |doc| {
            result = Some(f.take().unwrap()(doc));
            Ok(())
        })
    })?;
    result.unwrap_or_else(|| Err(mlua::Error::runtime("composition is not open")))
}

/// Mutable access to an open document.
///
/// Delegates to [`crate::backend::ScriptBackend::with_open_document_mut`].
pub fn with_document_mut<R>(
    lua: &Lua,
    id: DocumentId,
    f: impl FnOnce(&mut OpenDocument) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    let mut f = Some(f);
    let mut result: Option<mlua::Result<R>> = None;
    host.with_backend_mut(|backend| {
        backend.with_open_document_mut(id, &mut |doc| {
            result = Some(f.take().unwrap()(doc));
            Ok(())
        })
    })?;
    result.unwrap_or_else(|| Err(mlua::Error::runtime("composition is not open")))
}

fn apply_selection(lua: &Lua, doc: &mut OpenDocument, value: Value) -> mlua::Result<()> {
    match value {
        Value::Nil => {
            doc.clear_selection();
            Ok(())
        }
        Value::UserData(data) => {
            let collection = data.borrow::<LuaCollection>()?;
            if collection.name != SELECTION_COLLECTION {
                doc.adopt_collection_as_selection(&collection.name);
            }
            Ok(())
        }
        Value::Table(table) => {
            let kind: String = table.get("kind").unwrap_or_else(|_| "region".into());
            let start = optional_i64(table.get("start")?)?;
            let stop = optional_i64(table.get("stop")?)?;
            let channels = channels_from_lua(lua, table.get("channels")?)?;
            match kind.as_str() {
                "none" => {
                    doc.clear_selection();
                    Ok(())
                }
                "position" => {
                    let sample = start
                        .ok_or_else(|| mlua::Error::runtime("position selection needs start"))?;
                    doc.clear_selection();
                    doc.set_position(sample.max(0) as usize, channels);
                    Ok(())
                }
                "region" => {
                    let start = start
                        .ok_or_else(|| mlua::Error::runtime("region selection needs start"))?;
                    let stop = stop.unwrap_or(start);
                    doc.select_range(start.max(0) as usize, stop.max(0) as usize, channels);
                    Ok(())
                }
                kind => Err(mlua::Error::runtime(format!(
                    "unknown selection kind {kind}"
                ))),
            }
        }
        other => Err(mlua::Error::runtime(format!(
            "selection must be a table, collection, or nil, got {}",
            other.type_name()
        ))),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Write;
    use std::path::Path;
    use std::rc::Rc;

    use crate::backend::{BackendHandle, HeadlessBackend};
    use crate::host::{HostProfile, ScriptHost};
    use crate::world::HeadlessWorld;

    fn write_sine_wav(path: &Path, frames: u32, channels: u16, sample_rate: u32) {
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
            for _ in 0..channels {
                let t = i as f32 / sample_rate as f32;
                let s = (0.5 * (t * 440.0 * std::f32::consts::TAU).sin() * i16::MAX as f32) as i16;
                out.write_all(&s.to_le_bytes()).unwrap();
            }
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

    // ── 3.1: render builds a one-output plan and writes FLAC ─────────────────

    #[test]
    fn render_identity_output_writes_flac() {
        let (mut host, dir, _) = host_with_world();
        let src = dir.path().join("take.wav");
        let dest = dir.path().join("render-out.flac");
        write_sine_wav(&src, 4096, 2, 48_000);

        let src_lua = src.display().to_string().replace('\\', "\\\\");
        let dest_lua = dest.display().to_string().replace('\\', "\\\\");
        let out = host.eval(&format!(
            r#"
            field.exports.define({{ name = "test-flac", encoder = "flac" }})
            local c = field.composition.open("{src_lua}")
            c:render({{
              outputs = {{
                {{ path = "{dest_lua}", profile = "test-flac" }},
              }},
            }})
            return "ok"
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(dest.exists(), "render output not written");
    }

    // ── 3.1: begin_render returns a pollable job ──────────────────────────────

    #[test]
    fn begin_render_returns_job_and_writes_output() {
        let (mut host, dir, _) = host_with_world();
        let src = dir.path().join("take2.wav");
        let dest = dir.path().join("render-async-out.flac");
        write_sine_wav(&src, 2048, 1, 44_100);

        let src_lua = src.display().to_string().replace('\\', "\\\\");
        let dest_lua = dest.display().to_string().replace('\\', "\\\\");
        let out = host.eval(&format!(
            r#"
            field.exports.define({{ name = "test-flac44", encoder = "flac" }})
            local c = field.composition.open("{src_lua}")
            local job = c:begin_render({{
              outputs = {{
                {{ path = "{dest_lua}", profile = "test-flac44" }},
              }},
            }})
            assert(type(job) == "userdata", "expected job userdata")
            job:join()
            assert(job.finished, "job not finished after join")
            assert(job.total == 2048, "expected frame total, got " .. tostring(job.total))
            assert(job.done >= 1, "frame progress not updated")
            local results = job:results()
            assert(#results >= 1, "no results")
            assert(results[1].status == "ok", results[1].status)
            return "ok"
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(dest.exists(), "async render output not written");
    }

    // ── 3.3: app:confirm returns true by default ──────────────────────────────

    #[test]
    fn confirm_returns_true_by_default() {
        let (mut host, _dir, _) = host_with_world();
        let out = host.eval(
            r#"
            local ok = app:confirm("Delete?", "This removes files.")
            assert(ok == true, "default confirm should be true")
            return "ok"
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
    }

    // ── 3.3: push_confirm_response controls the result ────────────────────────

    #[test]
    fn confirm_uses_queued_responses() {
        let (mut host, _dir, _) = host_with_world();
        host.push_confirm_response(false);
        host.push_confirm_response(true);
        let out = host.eval(
            r#"
            local a = app:confirm("First?", "")
            local b = app:confirm("Second?", "")
            local c = app:confirm("Third?", "")  -- queue empty → true
            assert(a == false, "first should be false")
            assert(b == true,  "second should be true")
            assert(c == true,  "third (empty queue) should be true")
            return "ok"
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
    }
}
