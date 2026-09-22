// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Lua region handles.

use mlua::{UserData, UserDataFields};

use field_audio_model::RegionId;
use field_session::DocumentId;

use crate::composition::with_document;
use crate::selection::channels_to_lua;
use crate::world::OpenDocument;

/// Handle to one region in a document collection.
#[derive(Clone, Debug)]
pub struct LuaRegion {
    /// Document id.
    pub doc: DocumentId,
    /// Collection name.
    pub collection: String,
    /// Region id.
    pub id: RegionId,
}

impl UserData for LuaRegion {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id.0 as i64));
        fields.add_field_method_get("start", |lua, this| {
            with_document(lua, this.doc, |doc| {
                Ok(lookup_region(doc, this.id)?.start as i64)
            })
        });
        fields.add_field_method_get("stop", |lua, this| {
            with_document(lua, this.doc, |doc| {
                Ok(lookup_region(doc, this.id)?.end as i64)
            })
        });
        fields.add_field_method_get("channels", |lua, this| {
            with_document(lua, this.doc, |doc| {
                let region = lookup_region(doc, this.id)?;
                channels_to_lua(lua, &region.channels)
            })
        });
        fields.add_field_method_get("label", |lua, this| {
            with_document(lua, this.doc, |doc| Ok(lookup_region(doc, this.id)?.label))
        });
        fields.add_field_method_get("collection", |_, this| Ok(this.collection.clone()));
    }
}

fn lookup_region(doc: &OpenDocument, id: RegionId) -> mlua::Result<field_audio_model::Region> {
    doc.find_region(id)
        .map(|(_, region)| region)
        .ok_or_else(|| mlua::Error::runtime("region no longer exists"))
}
