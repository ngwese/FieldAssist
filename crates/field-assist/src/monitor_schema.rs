// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Map Faust UI JSON into gpui-free [`ParamUiNode`] trees.

use std::collections::HashMap;

use field_ui_components::ParamUiNode;

use crate::monitor::{menu_items_from_meta, meta_value, parse_ui_json, FaustUiNode};

/// Faust monitor controls split for the Monitor panel layout.
#[derive(Clone, Debug, Default)]
pub struct MonitorParamLayout {
    /// Slash-prefix sections (Headphones, M-S, Ambisonics, …).
    pub sections: Vec<ParamUiNode>,
    /// `Meter/Input*` bargraphs.
    pub input_meters: Vec<ParamUiNode>,
    /// `Meter/Output*` bargraphs.
    pub output_meters: Vec<ParamUiNode>,
    /// `Output/*` controls (e.g. Gain) for the Output footer.
    pub output_params: Vec<ParamUiNode>,
}

/// Parse Faust UI JSON and regroup into panel layout buckets.
pub fn param_ui_layout_from_json(json: &str) -> Option<MonitorParamLayout> {
    let root = parse_ui_json(json).ok()?;
    Some(layout_from_nodes(root.ui.iter().map(map_node).collect()))
}

fn layout_from_nodes(nodes: Vec<ParamUiNode>) -> MonitorParamLayout {
    let mut input_meters = Vec::new();
    let mut output_meters = Vec::new();
    let mut output_params = Vec::new();
    let mut section_order: Vec<String> = Vec::new();
    let mut section_items: HashMap<String, Vec<ParamUiNode>> = HashMap::new();
    let mut unsectioned = Vec::new();

    for mut node in flatten_leaves(nodes) {
        let Some(label) = label_of(&node).map(str::to_string) else {
            continue;
        };
        if label.starts_with("Meter/Input") {
            input_meters.push(node);
            continue;
        }
        if label.starts_with("Meter/Output") {
            output_meters.push(node);
            continue;
        }
        if let Some(rest) = label.strip_prefix("Output/") {
            set_label(&mut node, rest.to_string());
            output_params.push(node);
            continue;
        }
        match label.split_once('/') {
            Some((prefix, rest)) if !prefix.is_empty() && !rest.is_empty() => {
                set_label(&mut node, rest.to_string());
                if !section_order.iter().any(|s| s == prefix) {
                    section_order.push(prefix.to_string());
                }
                section_items
                    .entry(prefix.to_string())
                    .or_default()
                    .push(node);
            }
            _ => unsectioned.push(node),
        }
    }

    let mut sections: Vec<ParamUiNode> = section_order
        .into_iter()
        .filter_map(|section| {
            let items = section_items.remove(&section)?;
            Some(ParamUiNode::Group {
                label: section,
                items,
            })
        })
        .collect();
    if !unsectioned.is_empty() {
        sections.push(ParamUiNode::Group {
            label: String::new(),
            items: unsectioned,
        });
    }

    MonitorParamLayout {
        sections,
        input_meters,
        output_meters,
        output_params,
    }
}

fn flatten_leaves(nodes: Vec<ParamUiNode>) -> Vec<ParamUiNode> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            ParamUiNode::Group { items, .. } => out.extend(flatten_leaves(items)),
            ParamUiNode::Other => {}
            other => out.push(other),
        }
    }
    out
}

fn label_of(node: &ParamUiNode) -> Option<&str> {
    match node {
        ParamUiNode::Slider { label, .. }
        | ParamUiNode::Menu { label, .. }
        | ParamUiNode::Checkbox { label, .. }
        | ParamUiNode::Bargraph { label, .. }
        | ParamUiNode::Group { label, .. } => Some(label.as_str()),
        ParamUiNode::Other => None,
    }
}

fn set_label(node: &mut ParamUiNode, label: String) {
    match node {
        ParamUiNode::Slider { label: l, .. }
        | ParamUiNode::Menu { label: l, .. }
        | ParamUiNode::Checkbox { label: l, .. }
        | ParamUiNode::Bargraph { label: l, .. } => *l = label,
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regroups_slash_prefixes_and_extracts_meters_and_output() {
        let json = r#"{
            "name": "probe",
            "filename": "probe.dsp",
            "inputs": 2,
            "outputs": 2,
            "ui": [{
                "type": "vgroup",
                "label": "probe",
                "items": [
                    {"type": "hslider", "label": "Headphones/Amount", "address": "/probe/Headphones_Amount", "init": 0.35, "min": 0, "max": 1, "step": 0.01},
                    {"type": "checkbox", "label": "Headphones/Crossfeed", "address": "/probe/Headphones_Crossfeed", "init": 0},
                    {"type": "hslider", "label": "Output/Gain", "address": "/probe/Output_Gain", "init": 0, "min": -90, "max": 12, "step": 0.1, "meta": [{"unit": "dB"}]},
                    {"type": "hbargraph", "label": "Meter/Input L", "address": "/probe/Meter_Input_L", "min": -90, "max": 6, "meta": [{"unit": "dB"}]},
                    {"type": "hbargraph", "label": "Meter/Output R", "address": "/probe/Meter_Output_R", "min": -90, "max": 6, "meta": [{"unit": "dB"}]}
                ]
            }]
        }"#;
        let layout = param_ui_layout_from_json(json).expect("layout");
        assert_eq!(layout.sections.len(), 1);
        match &layout.sections[0] {
            ParamUiNode::Group { label, items } => {
                assert_eq!(label, "Headphones");
                assert_eq!(items.len(), 2);
                assert_eq!(label_of(&items[0]), Some("Amount"));
                assert_eq!(label_of(&items[1]), Some("Crossfeed"));
            }
            other => panic!("expected Headphones group, got {other:?}"),
        }
        assert_eq!(layout.output_params.len(), 1);
        assert_eq!(label_of(&layout.output_params[0]), Some("Gain"));
        assert_eq!(layout.input_meters.len(), 1);
        assert_eq!(layout.output_meters.len(), 1);
    }
}
