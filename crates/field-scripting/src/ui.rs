// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field.ui` constructors and default palette tables.

use mlua::{Lua, Table, UserData, UserDataFields};

use crate::marker::color_to_lua;
use crate::workflow_toolbar;

macro_rules! color_fields {
    ($ty:ty, $($field:ident = $value:ident),+ $(,)?) => {
        impl UserData for $ty {
            fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
                $(fields.add_field_method_get(stringify!($field), |lua, _| {
                    color_to_lua(lua, $value)
                });)+
            }
        }
    };
}

struct UiNamed;
struct UiSemantic;

const RED: [f32; 4] = [0.937, 0.267, 0.267, 1.0];
const RED_LIGHT: [f32; 4] = [0.992, 0.878, 0.878, 1.0];
const GREEN: [f32; 4] = [0.133, 0.773, 0.369, 1.0];
const GREEN_LIGHT: [f32; 4] = [0.863, 0.988, 0.906, 1.0];
const BLUE: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const BLUE_LIGHT: [f32; 4] = [0.878, 0.906, 0.992, 1.0];
const YELLOW: [f32; 4] = [0.918, 0.702, 0.031, 1.0];
const YELLOW_LIGHT: [f32; 4] = [0.996, 0.976, 0.839, 1.0];
const MAGENTA: [f32; 4] = [0.925, 0.282, 0.600, 1.0];
const MAGENTA_LIGHT: [f32; 4] = [0.992, 0.878, 0.925, 1.0];
const CYAN: [f32; 4] = [0.024, 0.714, 0.831, 1.0];
const CYAN_LIGHT: [f32; 4] = [0.820, 0.980, 0.992, 1.0];

color_fields!(
    UiNamed,
    red = RED,
    red_light = RED_LIGHT,
    green = GREEN,
    green_light = GREEN_LIGHT,
    blue = BLUE,
    blue_light = BLUE_LIGHT,
    yellow = YELLOW,
    yellow_light = YELLOW_LIGHT,
    magenta = MAGENTA,
    magenta_light = MAGENTA_LIGHT,
    cyan = CYAN,
    cyan_light = CYAN_LIGHT,
);

const ACCENT: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const ACCENT_FOREGROUND: [f32; 4] = [0.976, 0.980, 0.984, 1.0];
const BACKGROUND: [f32; 4] = [0.067, 0.094, 0.153, 1.0];
const BORDER: [f32; 4] = [0.216, 0.255, 0.333, 1.0];
const DANGER: [f32; 4] = [0.937, 0.267, 0.267, 1.0];
const DANGER_ACTIVE: [f32; 4] = [0.863, 0.149, 0.149, 1.0];
const DANGER_FOREGROUND: [f32; 4] = [0.976, 0.980, 0.984, 1.0];
const DANGER_HOVER: [f32; 4] = [0.863, 0.149, 0.149, 1.0];
const DROP_TARGET: [f32; 4] = [0.231, 0.510, 0.965, 0.35];
const FOREGROUND: [f32; 4] = [0.898, 0.906, 0.922, 1.0];
const INFO: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const INFO_ACTIVE: [f32; 4] = [0.145, 0.388, 0.922, 1.0];
const INFO_FOREGROUND: [f32; 4] = [0.976, 0.980, 0.984, 1.0];
const INFO_HOVER: [f32; 4] = [0.145, 0.388, 0.922, 1.0];
const INPUT: [f32; 4] = [0.122, 0.161, 0.216, 1.0];
const LINK: [f32; 4] = [0.376, 0.647, 0.980, 1.0];
const LINK_ACTIVE: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const LINK_HOVER: [f32; 4] = [0.576, 0.773, 0.992, 1.0];
const MUTED: [f32; 4] = [0.122, 0.161, 0.216, 1.0];
const MUTED_FOREGROUND: [f32; 4] = [0.580, 0.639, 0.722, 1.0];
const POPOVER: [f32; 4] = [0.067, 0.094, 0.153, 1.0];
const POPOVER_FOREGROUND: [f32; 4] = [0.898, 0.906, 0.922, 1.0];
const PRIMARY: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const PRIMARY_ACTIVE: [f32; 4] = [0.145, 0.388, 0.922, 1.0];
const PRIMARY_FOREGROUND: [f32; 4] = [0.976, 0.980, 0.984, 1.0];
const PRIMARY_HOVER: [f32; 4] = [0.145, 0.388, 0.922, 1.0];
const RING: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const SECONDARY: [f32; 4] = [0.216, 0.255, 0.333, 1.0];
const SECONDARY_ACTIVE: [f32; 4] = [0.278, 0.333, 0.412, 1.0];
const SECONDARY_FOREGROUND: [f32; 4] = [0.898, 0.906, 0.922, 1.0];
const SECONDARY_HOVER: [f32; 4] = [0.278, 0.333, 0.412, 1.0];
const SELECTION: [f32; 4] = [0.231, 0.510, 0.965, 0.35];
const SUCCESS: [f32; 4] = [0.133, 0.773, 0.369, 1.0];
const SUCCESS_ACTIVE: [f32; 4] = [0.086, 0.639, 0.290, 1.0];
const SUCCESS_FOREGROUND: [f32; 4] = [0.976, 0.980, 0.984, 1.0];
const SUCCESS_HOVER: [f32; 4] = [0.086, 0.639, 0.290, 1.0];
const WARNING: [f32; 4] = [0.918, 0.702, 0.031, 1.0];
const WARNING_ACTIVE: [f32; 4] = [0.769, 0.584, 0.024, 1.0];
const WARNING_FOREGROUND: [f32; 4] = [0.067, 0.094, 0.153, 1.0];
const WARNING_HOVER: [f32; 4] = [0.769, 0.584, 0.024, 1.0];
const CHART_1: [f32; 4] = [0.231, 0.510, 0.965, 1.0];
const CHART_2: [f32; 4] = [0.133, 0.773, 0.369, 1.0];
const CHART_3: [f32; 4] = [0.918, 0.702, 0.031, 1.0];
const CHART_4: [f32; 4] = [0.925, 0.282, 0.600, 1.0];
const CHART_5: [f32; 4] = [0.024, 0.714, 0.831, 1.0];
const CHART_BULLISH: [f32; 4] = [0.133, 0.773, 0.369, 1.0];
const CHART_BEARISH: [f32; 4] = [0.937, 0.267, 0.267, 1.0];

color_fields!(
    UiSemantic,
    accent = ACCENT,
    accent_foreground = ACCENT_FOREGROUND,
    background = BACKGROUND,
    border = BORDER,
    danger = DANGER,
    danger_active = DANGER_ACTIVE,
    danger_foreground = DANGER_FOREGROUND,
    danger_hover = DANGER_HOVER,
    drop_target = DROP_TARGET,
    foreground = FOREGROUND,
    info = INFO,
    info_active = INFO_ACTIVE,
    info_foreground = INFO_FOREGROUND,
    info_hover = INFO_HOVER,
    input = INPUT,
    link = LINK,
    link_active = LINK_ACTIVE,
    link_hover = LINK_HOVER,
    muted = MUTED,
    muted_foreground = MUTED_FOREGROUND,
    popover = POPOVER,
    popover_foreground = POPOVER_FOREGROUND,
    primary = PRIMARY,
    primary_active = PRIMARY_ACTIVE,
    primary_foreground = PRIMARY_FOREGROUND,
    primary_hover = PRIMARY_HOVER,
    ring = RING,
    secondary = SECONDARY,
    secondary_active = SECONDARY_ACTIVE,
    secondary_foreground = SECONDARY_FOREGROUND,
    secondary_hover = SECONDARY_HOVER,
    selection = SELECTION,
    success = SUCCESS,
    success_active = SUCCESS_ACTIVE,
    success_foreground = SUCCESS_FOREGROUND,
    success_hover = SUCCESS_HOVER,
    warning = WARNING,
    warning_active = WARNING_ACTIVE,
    warning_foreground = WARNING_FOREGROUND,
    warning_hover = WARNING_HOVER,
    chart_1 = CHART_1,
    chart_2 = CHART_2,
    chart_3 = CHART_3,
    chart_4 = CHART_4,
    chart_5 = CHART_5,
    chart_bullish = CHART_BULLISH,
    chart_bearish = CHART_BEARISH,
);

/// Install `field.ui` (toolbar constructors + palette tables).
pub fn bind_ui(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let ui = workflow_toolbar::ui_namespace(lua)?;
    ui.set("named", UiNamed)?;
    ui.set("semantic", UiSemantic)?;
    field.set("ui", ui)?;
    Ok(())
}
