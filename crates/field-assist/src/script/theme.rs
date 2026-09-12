// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui_kit::{Hsla, Rgba};
use gpui_kit::component::{ActiveTheme as _, ThemeColor};
use mlua::{Lua, Table, UserData, UserDataFields};

use super::access;
use super::marker::color_to_lua;

pub struct LuaTheme;
pub struct LuaThemeNamed;
pub struct LuaThemeSemantic;

impl UserData for LuaTheme {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("named", |_, _| Ok(LuaThemeNamed));
        fields.add_field_method_get("semantic", |_, _| Ok(LuaThemeSemantic));
    }
}

macro_rules! theme_color_getters {
    ($fields:ident, $($name:ident),+ $(,)?) => {
        $($fields.add_field_method_get(stringify!($name), |lua, _| {
            lua_theme_color(lua, current_theme_color().$name)
        });)+
    };
}

impl UserData for LuaThemeNamed {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        theme_color_getters!(
            fields,
            red,
            red_light,
            green,
            green_light,
            blue,
            blue_light,
            yellow,
            yellow_light,
            magenta,
            magenta_light,
            cyan,
            cyan_light,
        );
    }
}

impl UserData for LuaThemeSemantic {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        theme_color_getters!(
            fields,
            accent,
            accent_foreground,
            background,
            border,
            danger,
            danger_active,
            danger_foreground,
            danger_hover,
            drop_target,
            foreground,
            info,
            info_active,
            info_foreground,
            info_hover,
            input,
            link,
            link_active,
            link_hover,
            muted,
            muted_foreground,
            popover,
            popover_foreground,
            primary,
            primary_active,
            primary_foreground,
            primary_hover,
            ring,
            secondary,
            secondary_active,
            secondary_foreground,
            secondary_hover,
            selection,
            success,
            success_active,
            success_foreground,
            success_hover,
            warning,
            warning_active,
            warning_foreground,
            warning_hover,
            chart_1,
            chart_2,
            chart_3,
            chart_4,
            chart_5,
            chart_bullish,
            chart_bearish,
        );
    }
}

fn current_theme_color() -> ThemeColor {
    access::with_view(|_, _, cx| cx.theme().colors).unwrap_or_else(|_| ThemeColor::default())
}

fn hsla_to_rgba(color: Hsla) -> [f32; 4] {
    let rgba = Rgba::from(color);
    [rgba.r, rgba.g, rgba.b, rgba.a]
}

fn lua_theme_color(lua: &Lua, color: Hsla) -> mlua::Result<Table> {
    color_to_lua(lua, hsla_to_rgba(color))
}
