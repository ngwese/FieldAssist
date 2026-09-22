// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Composition userdata and document helpers.

use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_audio_model::{ChannelScope, RegionId, SELECTION_COLLECTION};
use field_session::DocumentId;

use crate::host::host_from_lua;
use crate::marker::{
    color_from_value, color_to_lua, list_markers, marker_id_from_lua, parse_add_marker, LuaMarker,
};
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
        fields.add_field_method_get("path", |lua, this| {
            Ok(host_from_lua(lua)?
                .document_path(this.id)
                .map(|path| path.display().to_string()))
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
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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
        methods.add_method("close", |lua, this, ()| {
            host_from_lua(lua)?.close_composition(this.id)
        });
        methods.add_method("replace", |lua, this, path: String| {
            host_from_lua(lua)?
                .replace_composition(this.id, &path)
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
    }
}

/// Install `field.composition`.
pub fn bind_composition_module(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let composition = lua.create_table()?;
    composition.set(
        "open",
        lua.create_function(|lua, path: String| {
            host_from_lua(lua)?
                .open_path(&path)
                .map(|id| LuaComposition { id })
        })?,
    )?;
    field.set("composition", composition)?;
    Ok(())
}

/// Read-only access to an open document.
pub fn with_document<R>(
    lua: &Lua,
    id: DocumentId,
    f: impl FnOnce(&OpenDocument) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    host.with_world(|world| {
        let doc = world
            .docs
            .get(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        f(doc)
    })
}

/// Mutable access to an open document.
pub fn with_document_mut<R>(
    lua: &Lua,
    id: DocumentId,
    f: impl FnOnce(&mut OpenDocument) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    host.with_world_mut(|world| {
        let doc = world
            .docs
            .get_mut(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        f(doc)
    })
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
