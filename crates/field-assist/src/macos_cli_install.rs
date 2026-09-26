//! SPDX-FileCopyrightText: 2026 Greg Wuller
//! SPDX-License-Identifier: MIT
//!
//! Install `field-play` / `field-batch` symlinks into `/usr/local/bin`.
//!
//! Runs an elevated `osascript` shell script so the password prompt is the
//! standard macOS authorization dialog. Feedback uses `NSAlert` so it works
//! with no editor window open.

use std::path::{Path, PathBuf};
use std::process::Command;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertStyle};
use objc2_foundation::NSString;

/// Symlink the CLI tools beside this executable into `/usr/local/bin`.
pub fn install_cli_tools() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("FieldAssist: Install CLI Tools requires the main thread");
        return;
    };

    match resolve_cli_binaries() {
        Ok((play, batch)) => match run_elevated_install(&play, &batch) {
            Ok(()) => show_alert(
                mtm,
                NSAlertStyle::Informational,
                "CLI Tools Installed",
                &format!(
                    "Installed symlinks:\n\
                     /usr/local/bin/field-play\n\
                     /usr/local/bin/field-batch\n\n\
                     They point at this copy of FieldAssist:\n\
                     {}\n\
                     {}",
                    play.display(),
                    batch.display()
                ),
            ),
            Err(err) => show_alert(
                mtm,
                NSAlertStyle::Warning,
                "Could Not Install CLI Tools",
                &err,
            ),
        },
        Err(err) => show_alert(mtm, NSAlertStyle::Warning, "CLI Tools Unavailable", &err),
    }
}

fn resolve_cli_binaries() -> Result<(PathBuf, PathBuf), String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Could not locate the FieldAssist executable:\n{err}"))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let dir = exe
        .parent()
        .ok_or_else(|| "Could not locate the FieldAssist MacOS directory.".to_string())?;
    let play = dir.join("field-play");
    let batch = dir.join("field-batch");
    if !play.is_file() || !batch.is_file() {
        return Err(
            "field-play and field-batch were not found next to FieldAssist.\n\n\
             Install CLI Tools works from the packaged .app (for example after \
             opening the release DMG). Development builds from `cargo run` do \
             not include those binaries in the same directory."
                .to_string(),
        );
    }
    Ok((play, batch))
}

fn applescript_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn run_elevated_install(play: &Path, batch: &Path) -> Result<(), String> {
    let play_s = applescript_string(play);
    let batch_s = applescript_string(batch);
    let script = format!(
        r#"set playBin to quoted form of "{play_s}"
set batchBin to quoted form of "{batch_s}"
do shell script "mkdir -p /usr/local/bin && ln -sfn " & playBin & " /usr/local/bin/field-play && ln -sfn " & batchBin & " /usr/local/bin/field-batch" with administrator privileges"#
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|err| format!("Failed to run osascript:\n{err}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else if !stdout.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        format!("osascript exited with {}", output.status)
    };

    // User cancelled the password dialog: AppleScript error -128.
    if detail.contains("-128") || detail.to_lowercase().contains("user canceled") {
        return Err("Installation was cancelled.".to_string());
    }
    Err(detail)
}

fn show_alert(mtm: MainThreadMarker, style: NSAlertStyle, title: &str, body: &str) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(style);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(body));
    let _ = alert.runModal();
}
