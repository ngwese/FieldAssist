// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

const EXTRA_ICONS: &[(&str, &[u8])] = &[
    (
        "icons/app-mark.svg",
        include_bytes!("../assets/logo/04-bands.svg"),
    ),
    (
        "icons/chevrons-left.svg",
        include_bytes!("../assets/icons/chevrons-left.svg"),
    ),
    (
        "icons/chevrons-right.svg",
        include_bytes!("../assets/icons/chevrons-right.svg"),
    ),
    (
        "icons/pause.svg",
        include_bytes!("../assets/icons/pause.svg"),
    ),
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    (
        "icons/repeat.svg",
        include_bytes!("../assets/icons/repeat.svg"),
    ),
    (
        "icons/skip-back.svg",
        include_bytes!("../assets/icons/skip-back.svg"),
    ),
    (
        "icons/skip-forward.svg",
        include_bytes!("../assets/icons/skip-forward.svg"),
    ),
    (
        "icons/monitor-speaker.svg",
        include_bytes!("../assets/icons/monitor-speaker.svg"),
    ),
    ("icons/ear.svg", include_bytes!("../assets/icons/ear.svg")),
    ("icons/pin.svg", include_bytes!("../assets/icons/pin.svg")),
];

/// App icons first, then the Lucide subset shipped by gpui-component-assets.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(bytes) = extra_icon(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_component_assets::Assets.list(path)?;
        for (icon_path, _) in EXTRA_ICONS {
            if icon_path.starts_with(path) {
                let name: SharedString = (*icon_path).into();
                if !items.contains(&name) {
                    items.push(name);
                }
            }
        }
        Ok(items)
    }
}

fn extra_icon(path: &str) -> Option<&'static [u8]> {
    EXTRA_ICONS
        .iter()
        .find(|(icon_path, _)| *icon_path == path)
        .map(|(_, bytes)| *bytes)
}
