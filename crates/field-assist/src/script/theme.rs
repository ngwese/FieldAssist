// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeColor, ThemeMode, ThemeRegistry};
use gpui_kit::{Hsla, Rgba};
use mlua::{Lua, Table, UserData, UserDataFields, Value};

use super::access;
use super::backend::backend_from_lua;

pub struct LuaTheme;
pub struct LuaThemeNamed;
pub struct LuaThemeSemantic;

impl UserData for LuaTheme {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("named", |_, _| Ok(LuaThemeNamed));
        fields.add_field_method_get("semantic", |_, _| Ok(LuaThemeSemantic));
        fields.add_field_method_get("name", |lua, _| Ok(backend_from_lua(lua)?.theme_name()));
        fields.add_field_method_set("name", |lua, _, value: Value| {
            let name = match value {
                Value::String(s) => s.to_str()?.to_owned(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "theme.name must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            backend_from_lua(lua)?.set_theme_name(&name)
        });
        fields.add_field_method_get("mode", |lua, _| Ok(backend_from_lua(lua)?.theme_mode()));
        fields.add_field_method_set("mode", |lua, _, value: Value| {
            let mode = match value {
                Value::String(s) => s.to_str()?.to_owned(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "theme.mode must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            backend_from_lua(lua)?.set_theme_mode(&mode)
        });
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

pub(super) fn themes_table(lua: &Lua) -> mlua::Result<Table> {
    let names = backend_from_lua(lua)?.theme_names();
    let table = lua.create_table_with_capacity(names.len(), 0)?;
    for (index, name) in names.into_iter().enumerate() {
        table.set(index + 1, name)?;
    }
    Ok(table)
}

pub(crate) fn parse_theme_mode(value: &str) -> Result<ThemeMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "light" => Ok(ThemeMode::Light),
        "dark" => Ok(ThemeMode::Dark),
        _ => Err(format!(
            "theme.mode must be \"light\" or \"dark\", got {value:?}"
        )),
    }
}

/// Apply theme mode from any App context (Settings UI, Lua, etc.).
pub(crate) fn apply_theme_mode_in_app(
    mode: ThemeMode,
    window: Option<&mut gpui_kit::Window>,
    cx: &mut gpui_kit::App,
) {
    Theme::change(mode, window, cx);
    crate::app::apply_muted_chrome(cx);
    refresh_open_windows(cx);
}

/// Apply a registered theme by name from any App context.
pub(crate) fn apply_theme_name_in_app(
    name: &str,
    window: Option<&mut gpui_kit::Window>,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    let (config, mode) = {
        let registry = ThemeRegistry::global(cx);
        let Some(config) = registry.themes().get(name).cloned() else {
            let available = registry
                .sorted_themes()
                .into_iter()
                .map(|theme| theme.name.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("unknown theme {name:?}; available: {available}"));
        };
        let mode = config.mode;
        (config, mode)
    };
    {
        let theme = Theme::global_mut(cx);
        if mode.is_dark() {
            theme.dark_theme = config;
        } else {
            theme.light_theme = config;
        }
    }
    Theme::change(mode, window, cx);
    crate::app::apply_muted_chrome(cx);
    refresh_open_windows(cx);
    Ok(())
}

fn refresh_open_windows(cx: &mut gpui_kit::App) {
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, _| {
            window.refresh();
        });
    }
}

pub(crate) fn apply_theme_mode(mode: ThemeMode) -> Result<(), String> {
    access::with_view(|_, window, cx| {
        apply_theme_mode_in_app(mode, Some(window), cx);
    })
}

pub(crate) fn apply_theme_name(name: &str) -> Result<(), String> {
    access::with_view(|_, window, cx| apply_theme_name_in_app(name, Some(window), cx))?
}

pub(super) fn live_theme_names() -> Result<Vec<String>, String> {
    access::with_view(|_, _, cx| {
        ThemeRegistry::global(cx)
            .sorted_themes()
            .into_iter()
            .map(|theme| theme.name.to_string())
            .collect()
    })
}

pub(super) fn live_theme_name() -> Result<String, String> {
    access::with_view(|_, _, cx| Theme::global(cx).theme_name().to_string())
}

pub(super) fn live_theme_mode() -> Result<String, String> {
    access::with_view(|_, _, cx| Theme::global(cx).mode.name().to_string())
}

fn current_theme_color() -> ThemeColor {
    access::with_view(|_, _, cx| cx.theme().colors).unwrap_or_else(|_| ThemeColor::default())
}

fn hsla_to_rgba(color: Hsla) -> [f32; 4] {
    let rgba = Rgba::from(color);
    [rgba.r, rgba.g, rgba.b, rgba.a]
}

fn lua_theme_color(lua: &Lua, color: Hsla) -> mlua::Result<Table> {
    let color = hsla_to_rgba(color);
    let table = lua.create_table_with_capacity(4, 0)?;
    for (index, value) in color.into_iter().enumerate() {
        table.set(index + 1, value)?;
    }
    Ok(table)
}
