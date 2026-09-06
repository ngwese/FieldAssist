// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

mod chain;
mod dsp;
#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    unused_variables,
    unused_mut,
    clippy::all
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/monitor_dsp.rs"));
}
mod host;
mod schema;

pub use chain::MonitorChain;
#[allow(unused_imports)]
pub use dsp::{create_dsp, IdentityDsp, MonitorDsp};
pub use host::MonitorHost;
pub use schema::{meta_value, parse_ui_json, FaustUiNode, FaustUiRoot};
