// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use crate::model::buffer::{ChannelScope, RegionId};
use crate::model::document::BufferDocument;
use crate::model::regions::SELECTION_COLLECTION;
use crate::model::DocumentId;

use super::marker::{
    color_from_value, color_to_lua, list_markers, marker_id_from_lua, parse_add_marker, LuaMarker,
};
use super::region::LuaRegion;
use super::selection::{channels_from_lua, collection_name_from_lua, optional_i64, LuaCollection};
use super::session::{optional_lua_string, string_map_from_lua, string_map_to_lua};
use super::{host_from_lua, with_document};

#[derive(Clone, Copy, Debug)]
pub struct LuaComposition {
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
            let host = host_from_lua(lua)?;
            host.display_name(this.id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))
        });
        fields.add_field_method_get("path", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.path(this.id).map(|path| path.display().to_string()))
        });
        fields.add_field_method_get("id", |lua, this| {
            let host = host_from_lua(lua)?;
            host.composition_id(this.id)
        });
        fields.add_field_method_get("group", |lua, this| {
            let host = host_from_lua(lua)?;
            host.document_group(this.id)
        });
        fields.add_field_method_set("group", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_document_group(this.id, optional_lua_string(value)?)
        });
        fields.add_field_method_get("state", |lua, this| {
            let host = host_from_lua(lua)?;
            host.document_state(this.id)
        });
        fields.add_field_method_set("state", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_document_state(this.id, optional_lua_string(value)?)
        });
        fields.add_field_method_get("properties", |lua, this| {
            let host = host_from_lua(lua)?;
            string_map_to_lua(lua, &host.document_properties(this.id)?)
        });
        fields.add_field_method_set("properties", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_document_properties(this.id, string_map_from_lua(value)?)
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
        fields.add_field_method_get("codec", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc.composition.read().unwrap().codec().map(str::to_string))
            })
        });
        fields.add_field_method_get("bit_depth", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .bit_depth()
                    .map(|bits| bits as i64))
            })
        });
        fields.add_field_method_get("basename", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.file_backed_path(this.id).and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            }))
        });
        fields.add_field_method_get("dirname", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.file_backed_path(this.id).and_then(|path| {
                path.parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
            }))
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
        fields.add_field_method_set("channel_layout", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            match value {
                Value::Nil => host.choose_layout(this.id, None)?,
                Value::String(name) => {
                    let name = name.to_str()?.to_owned();
                    host.choose_layout(this.id, Some(&name))?;
                }
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "channel_layout must be a string or nil, got {}",
                        other.type_name()
                    )))
                }
            }
            after_edit(lua, this.id)
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
        fields.add_field_method_set("monitor_chain", |lua, this, value: Value| {
            with_document(lua, this.id, |doc| {
                let chain = match value {
                    Value::Nil => None,
                    Value::String(name) => {
                        let name = name.to_str()?.to_owned();
                        if name.is_empty() {
                            None
                        } else {
                            Some(name)
                        }
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "monitor_chain must be a string or nil, got {}",
                            other.type_name()
                        )))
                    }
                };
                doc.composition.write().unwrap().set_monitor_chain(chain);
                Ok(())
            })?;
            after_edit(lua, this.id)
        });
        fields.add_field_method_get("playback_channels", |lua, this| {
            with_document(lua, this.id, |doc| {
                let composition = doc.composition.read().unwrap();
                match composition.playback_channels() {
                    None => Ok(Value::Nil),
                    Some(channels) => {
                        let table = lua.create_table()?;
                        for (i, ch) in channels.iter().enumerate() {
                            table.set(i + 1, *ch as i64)?;
                        }
                        Ok(Value::Table(table))
                    }
                }
            })
        });
        fields.add_field_method_set("playback_channels", |lua, this, value: Value| {
            with_document(lua, this.id, |doc| {
                let channels = match value {
                    Value::Nil => None,
                    Value::String(text) if text.to_str()?.eq_ignore_ascii_case("all") => None,
                    Value::Table(table) => {
                        let mut channels = Vec::new();
                        for pair in table.sequence_values::<i64>() {
                            let index = pair?;
                            if index < 0 {
                                return Err(mlua::Error::runtime(
                                    "playback_channels indices must be non-negative",
                                ));
                            }
                            channels.push(index as usize);
                        }
                        Some(channels)
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "playback_channels must be an array, \"all\", or nil, got {}",
                            other.type_name()
                        )))
                    }
                };
                doc.composition
                    .write()
                    .unwrap()
                    .set_playback_channels(channels);
                Ok(())
            })?;
            after_edit(lua, this.id)
        });
        fields.add_field_method_get("duration", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc.composition.read().unwrap().duration_secs())
            })
        });
        fields.add_field_method_get("selection", |_, this| {
            Ok(LuaCollection {
                doc: this.id,
                name: SELECTION_COLLECTION.into(),
            })
        });
        fields.add_field_method_set("selection", |lua, this, value: Value| {
            with_document(lua, this.id, |doc| {
                apply_selection(lua, doc, value)?;
                Ok(())
            })?;
            after_edit(lua, this.id)
        });
        fields.add_field_method_get("position", |lua, this| {
            with_document(lua, this.id, |doc| {
                Ok(doc.current_position.as_ref().map(|pos| pos.sample as i64))
            })
        });
        fields.add_field_method_set("position", |lua, this, sample: i64| {
            with_document(lua, this.id, |doc| {
                let sample = sample.max(0) as usize;
                doc.set_position(sample, ChannelScope::all());
                Ok(())
            })?;
            after_edit(lua, this.id)
        });
        fields.add_field_method_get("regions", |lua, this| {
            with_document(lua, this.id, |doc| {
                let regions: Vec<LuaRegion> = doc
                    .selection
                    .regions
                    .iter()
                    .map(|region| LuaRegion {
                        doc: this.id,
                        collection: SELECTION_COLLECTION.into(),
                        id: region.id,
                    })
                    .collect();
                Ok(regions)
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
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "select",
            |lua, this, (start, stop, channels): (i64, i64, Value)| {
                let channels = channels_from_lua(lua, channels)?;
                with_document(lua, this.id, |doc| {
                    doc.select_range(start.max(0) as usize, stop.max(0) as usize, channels);
                    Ok(())
                })?;
                after_edit(lua, this.id)
            },
        );
        methods.add_method("select_all", |lua, this, ()| {
            with_document(lua, this.id, |doc| {
                doc.select_all();
                Ok(())
            })?;
            after_edit(lua, this.id)
        });
        methods.add_method("clear_selection", |lua, this, ()| {
            with_document(lua, this.id, |doc| {
                doc.clear_selection();
                Ok(())
            })?;
            after_edit(lua, this.id)
        });
        methods.add_method("collection", |lua, this, name: String| {
            if name != SELECTION_COLLECTION {
                with_document(lua, this.id, |doc| {
                    doc.ensure_named_collection(&name);
                    Ok(())
                })?;
                after_edit(lua, this.id)?;
            }
            Ok(LuaCollection { doc: this.id, name })
        });
        methods.add_method("add_region", |lua, this, spec: Table| {
            let start: i64 = spec.get("start")?;
            let stop: i64 = spec.get("stop")?;
            let channels = channels_from_lua(lua, spec.get("channels")?)?;
            let label: Option<String> = spec.get("label")?;
            let collection = collection_name_from_lua(spec.get("collection")?)?;
            let id = with_document(lua, this.id, |doc| {
                Ok(doc.add_labeled_region(
                    start.max(0) as usize,
                    stop.max(0) as usize,
                    channels,
                    label,
                    &collection,
                ))
            })?;
            after_edit(lua, this.id)?;
            Ok(LuaRegion {
                doc: this.id,
                collection,
                id,
            })
        });
        methods.add_method("remove_region", |lua, this, id: i64| {
            let removed = with_document(lua, this.id, |doc| {
                Ok(doc.remove_region(RegionId(id.max(0) as u64)))
            })?;
            after_edit(lua, this.id)?;
            Ok(removed)
        });
        methods.add_method("add_marker", |lua, this, args: mlua::MultiValue| {
            let spec = parse_add_marker(args)?;
            let id = with_document(lua, this.id, |doc| {
                if let Some(color) = spec.color {
                    doc.add_marker_type(&spec.marker_type, color);
                }
                Ok(doc.add_marker(spec.frame, &spec.marker_type, spec.note))
            })?;
            after_edit(lua, this.id)?;
            Ok(id.map(|id| LuaMarker { doc: this.id, id }))
        });
        methods.add_method("remove_marker", |lua, this, target: Value| {
            let id = marker_id_from_lua(target)?;
            let removed = with_document(lua, this.id, |doc| Ok(doc.remove_marker(id)))?;
            after_edit(lua, this.id)?;
            Ok(removed)
        });
        methods.add_method(
            "remove_marker_at",
            |lua, this, (frame, marker_type): (i64, Option<String>)| {
                let frame = frame.max(0) as usize;
                let removed = with_document(lua, this.id, |doc| {
                    Ok(match marker_type.as_deref() {
                        Some(marker_type) => doc.remove_marker_at_type(frame, marker_type),
                        None => doc.remove_marker_at(frame),
                    })
                })?;
                after_edit(lua, this.id)?;
                Ok(removed)
            },
        );
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
                let added =
                    with_document(lua, this.id, |doc| Ok(doc.add_marker_type(&name, color)))?;
                after_edit(lua, this.id)?;
                Ok(added)
            },
        );
        methods.add_method("remove_marker_type", |lua, this, name: String| {
            let removed = with_document(lua, this.id, |doc| Ok(doc.remove_marker_type(&name)))?;
            after_edit(lua, this.id)?;
            Ok(removed)
        });
        methods.add_method("save", |lua, this, ()| {
            host_from_lua(lua)?.save_composition(this.id)
        });
        methods.add_method("close", |lua, this, ()| {
            host_from_lua(lua)?.close_composition(this.id)
        });
        methods.add_method("replace", |lua, this, path: String| {
            host_from_lua(lua)?
                .replace_composition(this.id, &path)
                .map(|id| LuaComposition { id })
        });
        methods.add_method("undo", |lua, this, ()| {
            edit(lua, this.id, |doc| doc.edit_undo())
        });
        methods.add_method("redo", |lua, this, ()| {
            edit(lua, this.id, |doc| doc.edit_redo())
        });
        methods.add_method("cut", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_cut();
                true
            })
        });
        methods.add_method("copy", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_copy();
                true
            })
        });
        methods.add_method("paste", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_paste();
                true
            })
        });
        methods.add_method("clear", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_clear();
                true
            })
        });
        methods.add_method("remove", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_remove();
                true
            })
        });
        methods.add_method("duplicate", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_duplicate();
                true
            })
        });
        methods.add_method("trim", |lua, this, ()| {
            edit(lua, this.id, |doc| {
                doc.edit_trim();
                true
            })
        });
    }
}

fn edit<R>(lua: &Lua, id: DocumentId, f: impl FnOnce(&mut BufferDocument) -> R) -> mlua::Result<R> {
    let result = with_document(lua, id, |doc| Ok(f(doc)))?;
    after_edit(lua, id)?;
    Ok(result)
}

fn after_edit(lua: &Lua, id: DocumentId) -> mlua::Result<()> {
    host_from_lua(lua)?.after_edit(id)
}

fn apply_selection(lua: &Lua, doc: &mut BufferDocument, value: Value) -> mlua::Result<()> {
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
