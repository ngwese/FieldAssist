// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Platform user config directory shared by FieldAssist and field-batch.

use std::path::PathBuf;

/// FieldAssist / field-batch user config directory, if resolvable.
///
/// | OS | Path |
/// | --- | --- |
/// | macOS | `~/Library/Application Support/FieldAssist/` |
/// | Windows | `%APPDATA%\FieldAssist\` |
/// | Linux | `$XDG_CONFIG_HOME/FieldAssist/` or `~/.config/FieldAssist/` |
pub fn user_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("FieldAssist")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("FieldAssist"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("FieldAssist"));
        }
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("FieldAssist"))
    }
}

#[cfg(test)]
mod tests {
    use super::user_config_dir;

    #[test]
    fn user_config_dir_ends_with_field_assist() {
        let Some(dir) = user_config_dir() else {
            return;
        };
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("FieldAssist")
        );
    }
}
