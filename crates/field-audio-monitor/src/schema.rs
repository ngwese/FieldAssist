// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Faust UI JSON parsing helpers for building monitor panels.

use std::collections::HashMap;

use serde::Deserialize;

/// Root object from Faust `-json` UI descriptions.
#[derive(Debug, Clone, Deserialize)]
pub struct FaustUiRoot {
    /// DSP name.
    pub name: String,
    /// Input count.
    pub inputs: u32,
    /// Output count.
    pub outputs: u32,
    /// Top-level UI nodes.
    #[serde(default)]
    pub ui: Vec<FaustUiNode>,
}

/// A node in the Faust UI tree.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum FaustUiNode {
    /// Vertical group.
    #[serde(rename = "vgroup")]
    VGroup {
        /// Group label.
        label: String,
        /// Child nodes.
        #[serde(default)]
        items: Vec<FaustUiNode>,
    },
    /// Horizontal group.
    #[serde(rename = "hgroup")]
    HGroup {
        /// Group label.
        label: String,
        /// Child nodes.
        #[serde(default)]
        items: Vec<FaustUiNode>,
    },
    /// Tab group.
    #[serde(rename = "tgroup")]
    TGroup {
        /// Group label.
        label: String,
        /// Child nodes.
        #[serde(default)]
        items: Vec<FaustUiNode>,
    },
    /// Horizontal slider.
    #[serde(rename = "hslider")]
    HSlider {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Initial value.
        init: f32,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
        /// Step.
        step: f32,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Vertical slider.
    #[serde(rename = "vslider")]
    VSlider {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Initial value.
        init: f32,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
        /// Step.
        step: f32,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Numeric entry.
    #[serde(rename = "nentry")]
    NEntry {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Initial value.
        init: f32,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
        /// Step.
        step: f32,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Checkbox.
    #[serde(rename = "checkbox")]
    Checkbox {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Initial value.
        #[serde(default)]
        init: f32,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Button.
    #[serde(rename = "button")]
    Button {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Horizontal bargraph (meter).
    #[serde(rename = "hbargraph")]
    HBargraph {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Vertical bargraph (meter).
    #[serde(rename = "vbargraph")]
    VBargraph {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
        /// Faust metadata entries.
        #[serde(default)]
        meta: Vec<HashMap<String, String>>,
    },
    /// Unrecognized node type.
    #[serde(other)]
    Other,
}

impl FaustUiNode {
    /// Child items for group nodes.
    pub fn items(&self) -> &[FaustUiNode] {
        match self {
            Self::VGroup { items, .. }
            | Self::HGroup { items, .. }
            | Self::TGroup { items, .. } => items,
            _ => &[],
        }
    }

    /// Label for group nodes.
    pub fn group_label(&self) -> Option<&str> {
        match self {
            Self::VGroup { label, .. }
            | Self::HGroup { label, .. }
            | Self::TGroup { label, .. } => Some(label.as_str()),
            _ => None,
        }
    }

    /// Parameter address for leaf widgets.
    pub fn address(&self) -> Option<&str> {
        match self {
            Self::HSlider { address, .. }
            | Self::VSlider { address, .. }
            | Self::NEntry { address, .. }
            | Self::Checkbox { address, .. }
            | Self::Button { address, .. }
            | Self::HBargraph { address, .. }
            | Self::VBargraph { address, .. } => Some(address.as_str()),
            _ => None,
        }
    }

    /// Whether this node is a passive meter.
    pub fn is_passive(&self) -> bool {
        matches!(self, Self::HBargraph { .. } | Self::VBargraph { .. })
    }
}

/// Parse Faust UI JSON.
pub fn parse_ui_json(json: &str) -> Result<FaustUiRoot, serde_json::Error> {
    serde_json::from_str(json)
}

/// Depth-first widget addresses in Faust ParamIndex order.
pub fn collect_addresses(root: &FaustUiRoot) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for node in &root.ui {
        collect_node(node, &mut out);
    }
    out
}

fn collect_node(node: &FaustUiNode, out: &mut Vec<(String, bool)>) {
    if let Some(address) = node.address() {
        out.push((address.to_string(), node.is_passive()));
    }
    for child in node.items() {
        collect_node(child, out);
    }
}

/// Look up a metadata value by key.
pub fn meta_value<'a>(meta: &'a [HashMap<String, String>], key: &str) -> Option<&'a str> {
    meta.iter()
        .find_map(|entry| entry.get(key).map(String::as_str))
}

/// Faust `[style:menu{'Label':value;...}]` entries, in listed order.
pub fn parse_menu_style(style: &str) -> Option<Vec<(String, f32)>> {
    let rest = style.trim().strip_prefix("menu")?.trim_start();
    let inner = rest.strip_prefix('{')?.strip_suffix('}')?;
    let mut items = Vec::new();
    for part in inner.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (label, value) = part.split_once(':')?;
        let label = label
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_string();
        if label.is_empty() {
            return None;
        }
        let value: f32 = value.trim().parse().ok()?;
        items.push((label, value));
    }
    (!items.is_empty()).then_some(items)
}

/// Parse menu items from Faust widget metadata.
pub fn menu_items_from_meta(meta: &[HashMap<String, String>]) -> Option<Vec<(String, f32)>> {
    parse_menu_style(meta_value(meta, "style")?)
}

#[allow(dead_code)]
/// Flatten groups and controls into a depth-first list.
pub fn flatten_controls(root: &FaustUiRoot) -> Vec<FaustUiNode> {
    let mut out = Vec::new();
    for node in &root.ui {
        flatten_node(node, &mut out);
    }
    out
}

fn flatten_node(node: &FaustUiNode, out: &mut Vec<FaustUiNode>) {
    match node {
        FaustUiNode::VGroup { items, .. }
        | FaustUiNode::HGroup { items, .. }
        | FaustUiNode::TGroup { items, .. } => {
            out.push(node.clone());
            for child in items {
                flatten_node(child, out);
            }
        }
        FaustUiNode::Other => {}
        _ => out.push(node.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slider_address() {
        let json = r#"{
            "name": "probe",
            "inputs": 1,
            "outputs": 2,
            "ui": [{
                "type": "vgroup",
                "label": "probe",
                "items": [{
                    "type": "hslider",
                    "label": "Gain",
                    "address": "/probe/Gain",
                    "init": 0,
                    "min": -90,
                    "max": 12,
                    "step": 0.1
                }]
            }]
        }"#;
        let root = parse_ui_json(json).unwrap();
        let addresses = collect_addresses(&root);
        assert_eq!(addresses, vec![("/probe/Gain".into(), false)]);
    }

    #[test]
    fn parses_faust_menu_style() {
        let items = parse_menu_style("menu{'Up':0;'Down':1;'Endfire':2}").unwrap();
        assert_eq!(
            items,
            vec![
                ("Up".into(), 0.0),
                ("Down".into(), 1.0),
                ("Endfire".into(), 2.0),
            ]
        );
        assert!(parse_menu_style("knob").is_none());
    }
}
