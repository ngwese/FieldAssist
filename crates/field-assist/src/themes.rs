//! Vendored GPUI theme packs from [gpui-kit](https://github.com/longbridge/gpui-kit)
//! `themes/` at tag **v0.6.1** (Apache-2.0; see `assets/themes/LICENSE-APACHE`).
//!
//! Call [`register_bundled`] after `gpui_kit::init` so Default Light/Dark already
//! exist; extra packs then appear in `app.themes`.

use gpui_kit::component::ThemeRegistry;
use gpui_kit::App;

const BUNDLED_THEME_SETS: &[&str] = &[
    include_str!("../assets/themes/adventure.json"),
    include_str!("../assets/themes/alduin.json"),
    include_str!("../assets/themes/asciinema.json"),
    include_str!("../assets/themes/aurora.json"),
    include_str!("../assets/themes/ayu.json"),
    include_str!("../assets/themes/catppuccin.json"),
    include_str!("../assets/themes/everforest.json"),
    include_str!("../assets/themes/fahrenheit.json"),
    include_str!("../assets/themes/flexoki.json"),
    include_str!("../assets/themes/gruvbox.json"),
    include_str!("../assets/themes/harper.json"),
    include_str!("../assets/themes/hybrid.json"),
    include_str!("../assets/themes/jellybeans.json"),
    include_str!("../assets/themes/kibble.json"),
    include_str!("../assets/themes/macos-classic.json"),
    include_str!("../assets/themes/mellifluous.json"),
    include_str!("../assets/themes/molokai.json"),
    include_str!("../assets/themes/solarized.json"),
    include_str!("../assets/themes/spaceduck.json"),
    include_str!("../assets/themes/tokyonight.json"),
    include_str!("../assets/themes/twilight.json"),
];

/// Register vendored theme packs into the global [`ThemeRegistry`].
///
/// Call after `gpui_kit::init` so the default Light/Dark themes already exist.
pub fn register_bundled(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for content in BUNDLED_THEME_SETS {
        registry
            .load_themes_from_str(content)
            .expect("bundled theme JSON must parse");
    }
}

#[cfg(test)]
mod tests {
    use super::BUNDLED_THEME_SETS;

    #[test]
    fn bundled_theme_sets_are_valid_json_with_themes() {
        let mut names = Vec::new();
        for content in BUNDLED_THEME_SETS {
            let value: serde_json::Value = serde_json::from_str(content).expect("theme set JSON");
            let themes = value
                .get("themes")
                .and_then(|t| t.as_array())
                .expect("themes array");
            assert!(!themes.is_empty());
            for theme in themes {
                let name = theme
                    .get("name")
                    .and_then(|n| n.as_str())
                    .expect("theme name");
                names.push(name.to_string());
            }
        }
        assert!(
            names.len() >= 36,
            "expected at least 36 named themes, got {}",
            names.len()
        );
        assert!(names.iter().any(|n| n == "Catppuccin Mocha"));
        assert!(names.iter().any(|n| n == "Tokyo Night"));
    }
}
