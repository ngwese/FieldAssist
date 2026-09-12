// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host-neutral parameter UI tree for monitor / DSP panels.

use std::collections::HashMap;

/// A node in a host-supplied parameter UI tree.
#[derive(Clone, Debug)]
pub enum ParamUiNode {
    /// Nested group of controls.
    Group {
        /// Group label.
        label: String,
        /// Child nodes.
        items: Vec<ParamUiNode>,
    },
    /// Continuous slider / numeric control.
    Slider {
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
        /// Optional unit suffix (for example `"dB"`).
        unit: Option<String>,
        /// Whether the slider uses a logarithmic scale.
        logarithmic: bool,
    },
    /// Discrete menu mapped to float values.
    Menu {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Initial value.
        init: f32,
        /// `(label, value)` choices.
        items: Vec<(String, f32)>,
    },
    /// Boolean checkbox (0 / 1).
    Checkbox {
        /// Widget label.
        label: String,
        /// Parameter address.
        address: String,
        /// Initial value.
        init: f32,
    },
    /// Read-only meter / bargraph.
    Bargraph {
        /// Widget label.
        label: String,
        /// Meter address.
        address: String,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
        /// Optional unit suffix.
        unit: Option<String>,
    },
    /// Unsupported / ignored node.
    Other,
}

impl ParamUiNode {
    /// Collect parameter and meter addresses under this tree.
    pub fn collect_addresses(nodes: &[ParamUiNode], out: &mut Vec<String>) {
        for node in nodes {
            match node {
                ParamUiNode::Group { items, .. } => Self::collect_addresses(items, out),
                ParamUiNode::Slider { address, .. }
                | ParamUiNode::Menu { address, .. }
                | ParamUiNode::Checkbox { address, .. }
                | ParamUiNode::Bargraph { address, .. } => out.push(address.clone()),
                ParamUiNode::Other => {}
            }
        }
    }
}

/// One monitor-chain menu choice (`None` id = Direct).
#[derive(Clone, Debug)]
pub struct ChainChoice {
    /// Chain id, or `None` for Direct / bypass.
    pub id: Option<String>,
    /// Display label.
    pub label: String,
}

/// Snapshot of monitor UI state for one render.
#[derive(Clone, Debug)]
pub struct MonitorSnapshot {
    /// Current chain display label.
    pub chain_label: String,
    /// Current chain id (`None` = Direct).
    pub chain_id: Option<String>,
    /// Available chain choices including Direct.
    pub chain_choices: Vec<ChainChoice>,
    /// Whether monitor parameters are pinned.
    pub params_pinned: bool,
    /// Channel labels for playback selection.
    pub channel_labels: Vec<String>,
    /// Selected playback channel indices (`None` = all).
    pub playback_channels: Option<Vec<usize>>,
    /// Expected input count for the active chain, when known.
    pub expected_inputs: Option<usize>,
    /// Identity key for the parameter schema (rebuild sliders when it changes).
    pub schema_id: u64,
    /// Parameter UI tree.
    pub params_ui: Vec<ParamUiNode>,
    /// Live parameter values by address.
    pub live_params: HashMap<String, f32>,
    /// Live meter values by address.
    pub meters: HashMap<String, f32>,
    /// Selected output device name (`None` = system default).
    pub output_device: Option<String>,
    /// Available output device names.
    pub output_devices: Vec<String>,
}
