// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Timeline marker userdata and helpers.

use mlua::{FromLua, Lua, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_audio_model::{default_marker_type, Marker, MarkerId};
use field_session::DocumentId;

use crate::composition::with_document;
use crate::selection::optional_i64;
use crate::world::OpenDocument;

/// Handle to one composition marker.
#[derive(Clone, Copy, Debug)]
pub struct LuaMarker {
    /// Document id.
    pub doc: DocumentId,
    /// Marker id.
    pub id: MarkerId,
}

impl FromLua for LuaMarker {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(data) => data.borrow::<Self>().map(|this| *this),
            _ => Err(mlua::Error::runtime("expected a marker")),
        }
    }
}

impl UserData for LuaMarker {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id.0 as i64));
        fields.add_field_method_get("frame", |lua, this| {
            with_marker(lua, this, |marker| Ok(marker.frame as i64))
        });
        fields.add_field_method_get("sample", |lua, this| {
            with_marker(lua, this, |marker| Ok(marker.frame as i64))
        });
        fields.add_field_method_get("type", |lua, this| {
            with_marker(lua, this, |marker| Ok(marker.marker_type.clone()))
        });
        fields.add_field_method_get("color", |lua, this| {
            with_document(lua, this.doc, |doc| {
                let composition = doc.composition.read().unwrap();
                let marker = composition
                    .markers()
                    .get(this.id)
                    .ok_or_else(|| mlua::Error::runtime("marker no longer exists"))?;
                color_to_lua(lua, composition.resolved_marker_color(&marker.marker_type))
            })
        });
        fields.add_field_method_get("note", |lua, this| {
            with_marker(lua, this, |marker| Ok(marker.note.clone()))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |lua, this, ()| {
            with_document_mut(lua, this.doc, |doc| Ok(doc.remove_marker(this.id)))
        });
    }
}

/// Arguments for `add_marker`.
pub struct AddMarkerArgs {
    /// Frame index.
    pub frame: usize,
    /// Marker type name.
    pub marker_type: String,
    /// Optional type color when creating a new type.
    pub color: Option<[f32; 4]>,
    /// Optional note text.
    pub note: Option<String>,
}

/// Parse `add_marker` call styles.
pub fn parse_add_marker(args: MultiValue) -> mlua::Result<AddMarkerArgs> {
    let mut iter = args.into_iter();
    let first = iter
        .next()
        .ok_or_else(|| mlua::Error::runtime("add_marker needs a frame or a table"))?;
    match first {
        Value::Table(spec) => spec_from_table(spec),
        other => {
            let frame = optional_i64(other)?
                .ok_or_else(|| mlua::Error::runtime("add_marker needs a frame or a table"))?;
            let marker_type = match iter.next() {
                None | Some(Value::Nil) => default_marker_type().to_string(),
                Some(Value::String(s)) => s.to_str()?.to_string(),
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "marker type must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            Ok(AddMarkerArgs {
                frame: frame.max(0) as usize,
                marker_type,
                color: None,
                note: None,
            })
        }
    }
}

/// Resolve marker id from userdata or integer.
pub fn marker_id_from_lua(value: Value) -> mlua::Result<MarkerId> {
    match value {
        Value::UserData(data) => Ok(data.borrow::<LuaMarker>()?.id),
        other => {
            let id = optional_i64(other)?
                .ok_or_else(|| mlua::Error::runtime("expected a marker or marker id"))?;
            Ok(MarkerId(id.max(0) as u64))
        }
    }
}

fn spec_from_table(spec: Table) -> mlua::Result<AddMarkerArgs> {
    let frame = optional_i64(spec.get("frame")?)?
        .or(optional_i64(spec.get("sample")?)?)
        .ok_or_else(|| mlua::Error::runtime("add_marker needs frame"))?;
    let marker_type = spec
        .get::<Option<String>>("type")?
        .or(spec.get::<Option<String>>("kind")?)
        .unwrap_or_else(|| default_marker_type().to_string());
    let note: Option<String> = spec.get("note")?;
    Ok(AddMarkerArgs {
        frame: frame.max(0) as usize,
        marker_type,
        color: color_from_value(spec.get("color")?)?,
        note,
    })
}

/// Parse optional RGBA table.
pub fn color_from_value(value: Value) -> mlua::Result<Option<[f32; 4]>> {
    match value {
        Value::Nil => Ok(None),
        Value::Table(table) => {
            let r: f64 = table.get(1)?;
            let g: f64 = table.get(2)?;
            let b: f64 = table.get(3)?;
            let a: f64 = table.get(4).unwrap_or(1.0);
            Ok(Some([r as f32, g as f32, b as f32, a as f32]))
        }
        other => Err(mlua::Error::runtime(format!(
            "color must be a table of four numbers, got {}",
            other.type_name()
        ))),
    }
}

/// Write RGBA to a 1-based Lua array table.
pub fn color_to_lua(lua: &Lua, color: [f32; 4]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (i, component) in color.iter().enumerate() {
        table.set(i + 1, *component)?;
    }
    Ok(table)
}

fn with_marker<R>(
    lua: &Lua,
    marker: &LuaMarker,
    f: impl FnOnce(&Marker) -> mlua::Result<R>,
) -> mlua::Result<R> {
    with_document(lua, marker.doc, |doc| {
        let composition = doc.composition.read().unwrap();
        let found = composition
            .markers()
            .get(marker.id)
            .ok_or_else(|| mlua::Error::runtime("marker no longer exists"))?;
        f(found)
    })
}

fn with_document_mut<R>(
    lua: &Lua,
    id: DocumentId,
    f: impl FnOnce(&mut OpenDocument) -> mlua::Result<R>,
) -> mlua::Result<R> {
    crate::composition::with_document_mut(lua, id, f)
}

/// List markers as Lua handles.
pub fn list_markers(doc: &OpenDocument, composition_id: DocumentId) -> Vec<LuaMarker> {
    doc.composition
        .read()
        .unwrap()
        .markers()
        .iter()
        .map(|marker| LuaMarker {
            doc: composition_id,
            id: marker.id,
        })
        .collect()
}
