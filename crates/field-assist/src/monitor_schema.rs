// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Map Faust UI JSON into gpui-free [`ParamUiNode`] trees.

use field_ui_components::ParamUiNode;

use crate::monitor::{menu_items_from_meta, meta_value, parse_ui_json, FaustUiNode, FaustUiRoot};

/// Parse Faust UI JSON into a [`ParamUiNode`] tree.
pub fn param_ui_from_json(json: &str) -> Option<Vec<ParamUiNode>> {
    let root = parse_ui_json(json).ok()?;
    Some(map_root(&root))
}

fn map_root(root: &FaustUiRoot) -> Vec<ParamUiNode> {
    root.ui.iter().map(map_node).collect()
}

fn map_node(node: &FaustUiNode) -> ParamUiNode {
    match node {
        FaustUiNode::VGroup { label, items }
        | FaustUiNode::HGroup { label, items }
        | FaustUiNode::TGroup { label, items } => ParamUiNode::Group {
            label: label.clone(),
            items: items.iter().map(map_node).collect(),
        },
        FaustUiNode::NEntry {
            label,
            address,
            init,
            meta,
            ..
        } if menu_items_from_meta(meta).is_some() => ParamUiNode::Menu {
            label: label.clone(),
            address: address.clone(),
            init: *init,
            items: menu_items_from_meta(meta).unwrap_or_default(),
        },
        FaustUiNode::HSlider {
            label,
            address,
            init,
            min,
            max,
            step,
            meta,
        }
        | FaustUiNode::VSlider {
            label,
            address,
            init,
            min,
            max,
            step,
            meta,
        }
        | FaustUiNode::NEntry {
            label,
            address,
            init,
            min,
            max,
            step,
            meta,
        } => ParamUiNode::Slider {
            label: label.clone(),
            address: address.clone(),
            init: *init,
            min: *min,
            max: *max,
            step: *step,
            unit: meta_value(meta, "unit").map(str::to_string),
            logarithmic: *min > 0.0
                && meta_value(meta, "scale").is_some_and(|scale| scale.eq_ignore_ascii_case("log")),
        },
        FaustUiNode::Checkbox {
            label,
            address,
            init,
            ..
        } => ParamUiNode::Checkbox {
            label: label.clone(),
            address: address.clone(),
            init: *init,
        },
        FaustUiNode::HBargraph {
            label,
            address,
            min,
            max,
            meta,
        }
        | FaustUiNode::VBargraph {
            label,
            address,
            min,
            max,
            meta,
        } => ParamUiNode::Bargraph {
            label: label.clone(),
            address: address.clone(),
            min: *min,
            max: *max,
            unit: meta_value(meta, "unit").map(str::to_string),
        },
        FaustUiNode::Button { .. } | FaustUiNode::Other => ParamUiNode::Other,
    }
}

/// Collect live parameter addresses from a UI tree.
pub fn collect_param_addresses(nodes: &[ParamUiNode]) -> Vec<String> {
    let mut out = Vec::new();
    ParamUiNode::collect_addresses(nodes, &mut out);
    out
}
