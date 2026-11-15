// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Action, App, KeyBinding, KeyBindingContextPredicate, Keystroke, OperatingSystem, actions,
};
use schemars::JsonSchema;
use serde::Deserialize;

actions!(
    FieldAssist,
    [
        Open,
        Save,
        SaveAs,
        SaveSession,
        SaveSessionAs,
        CloseSession,
        Close,
        Export,
        About,
        HelpOnlineDocs,
        Settings,
        UserVariables,
        InstallCliTools,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
        TransportHome,
        TransportPrevious,
        TransportStart,
        TransportPlayPause,
        TransportStop,
        TransportNext,
        TransportEnd,
        TransportLoop,
        TransportPreview,
        ViewFitAll,
        ViewFrame,
        ViewZoomIn,
        ViewZoomOut,
        ViewExplorer,
        ViewDetail,
        ViewScript,
        ViewShowExplorer,
        ViewHideExplorer,
        ViewShowDetail,
        ViewHideDetail,
        ViewShowScript,
        ViewHideScript,
        ViewShowMedia,
        ViewToggleMedia,
        ViewWrapMessages,
        ViewFollowPlayhead,
        ViewOverlayEnvelopePeak,
        ViewWaveformPeaks,
        ViewWaveformSpectrum,
        ViewWaveformPeaksSpectrum,
        AnalyzeSelectionOnly,
        AnalyzeEnvelopePeak,
        AnalyzeTransients,
        EditUndo,
        EditRedo,
        EditCut,
        EditCopy,
        EditPaste,
        EditClear,
        EditRemove,
        EditDuplicate,
        EditTrim,
        EditBreakOutRegions,
        EditBreakOutChannels,
        SelectAll,
        SelectNone,
        InvertSelection,
        ZeroCrossing,
        MarkerTypeBlue,
        MarkerTypeYellow,
        MarkerTypePurple,
        SnapToMarker,
        AddMarkerAtHover,
        AddMarker,
        AddNote,
        DeleteMarker,
        CancelWorkflow,
    ]
);

#[derive(Clone, PartialEq, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = FieldAssist)]
pub struct SetActiveMarkerType {
    pub name: String,
}

#[derive(Clone, PartialEq, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = FieldAssist)]
pub struct ToggleSnapMarkerType {
    pub name: String,
}

#[derive(Clone, PartialEq, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = FieldAssist)]
pub struct StartWorkflow {
    pub name: String,
}

const DEFAULT_KEYMAP: &str = include_str!("../assets/keymap.json");

/// Keymap actions apply while the waveform App view is focused, not while
/// typing in the Script panel or other text fields.
const APP_KEY_CONTEXT: &str = "App";

/// Keys that type in text fields apply while the waveform is focused.
const WAVEFORM_KEY_CONTEXT: &str = "Waveform";

/// Those keys also apply while the pointer is over the waveform.
const WAVEFORM_HOVER_KEY_CONTEXT: &str = "WaveformHover";

const KNOWN_COMMANDS: &[&str] = &[
    "file.open",
    "file.save",
    "file.save_as",
    "file.save_session",
    "file.save_session_as",
    "file.close_session",
    "file.close",
    "file.export",
    "file.quit",
    "app.hide",
    "app.hide_others",
    "app.show_all",
    "app.settings",
    "app.variables",
    "app.install_cli_tools",
    "help.online_docs",
    "help.about",
    "view.fit_all",
    "view.frame",
    "view.zoom_in",
    "view.zoom_out",
    "view.show-explorer",
    "view.hide-explorer",
    "view.toggle-explorer",
    "view.show-detail",
    "view.hide-detail",
    "view.toggle-detail",
    "view.show-script",
    "view.hide-script",
    "view.toggle-script",
    "view.show-media",
    "view.toggle-media",
    "view.wrap_messages",
    "view.follow_playhead",
    "view.overlay_envelope_peak",
    "view.waveform_peaks",
    "view.waveform_spectrum",
    "view.waveform_peaks_spectrum",
    "analyze.selection_only",
    "analyze.envelope_peak",
    "analyze.transients",
    "transport.home",
    "transport.previous",
    "transport.start",
    "transport.play_pause",
    "transport.stop",
    "transport.next",
    "transport.end",
    "transport.loop",
    "transport.preview",
    "edit.undo",
    "edit.redo",
    "edit.cut",
    "edit.copy",
    "edit.paste",
    "edit.clear",
    "edit.remove",
    "edit.duplicate",
    "edit.trim",
    "edit.break_out_regions",
    "edit.break_out_channels",
    "selection.select_all",
    "selection.select_none",
    "selection.invert",
    "selection.zero_crossing",
    "selection.marker_type_blue",
    "selection.marker_type_yellow",
    "selection.marker_type_purple",
    "selection.snap_to_marker",
    "selection.add_at_hover",
    "selection.add_marker",
    "selection.add_note",
    "selection.delete_marker",
];

/// Command IDs that keymap, menus, and Lua `app:command` share.
#[cfg(test)]
pub fn known_commands() -> &'static [&'static str] {
    KNOWN_COMMANDS
}

pub fn is_known_command(command_id: &str) -> bool {
    KNOWN_COMMANDS.contains(&command_id)
}

/// Run a command by keymap ID. Lua and the menu/key path share this table.
pub fn dispatch(command_id: &str, cx: &mut App) -> Result<(), String> {
    validate_command_id(command_id)?;
    if command_id.starts_with("analyze.")
        && !crate::settings::feature_enabled(field_features::Feature::AnalysisOps, cx)
    {
        return Err(format!(
            "command `{command_id}` requires the Analysis Ops experimental flag"
        ));
    }
    if let Some(result) = crate::script::try_invoke_command(command_id) {
        return result;
    }
    crate::app::dispatch_command(command_id, cx)
}

pub fn validate_command_id(command_id: &str) -> Result<(), String> {
    if !is_known_command(command_id) {
        return Err(format!("unknown command `{command_id}`"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Macos,
    Linux,
    Windows,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct KeymapFile {
    #[serde(default)]
    bindings: HashMap<String, String>,
    #[serde(default)]
    macos: HashMap<String, String>,
    #[serde(default)]
    linux: HashMap<String, String>,
    #[serde(default)]
    windows: HashMap<String, String>,
}

pub fn install_keybindings(cx: &mut App) {
    let bindings = load_keybindings(cx);
    cx.bind_keys(bindings);
}

fn load_keybindings(cx: &App) -> Vec<KeyBinding> {
    resolved_bindings()
        .into_iter()
        .flat_map(|(keystrokes, command_id)| bindings_for(&command_id, &keystrokes, cx))
        .collect()
}

fn resolved_bindings() -> HashMap<String, String> {
    let platform = current_platform();
    let mut map = match parse_and_flatten(DEFAULT_KEYMAP, platform) {
        Ok(map) => map,
        Err(err) => {
            eprintln!("FieldAssist: failed to parse default keymap: {err}");
            HashMap::new()
        }
    };
    match read_user_keymap() {
        None => {}
        Some(Err(err)) => {
            eprintln!("FieldAssist: failed to read user keymap: {err}");
        }
        Some(Ok(text)) => match parse_and_flatten(&text, platform) {
            Ok(user) => merge_bindings(&mut map, user),
            Err(err) => {
                eprintln!("FieldAssist: failed to parse user keymap, using defaults: {err}");
            }
        },
    }
    map
}

fn parse_and_flatten(json: &str, platform: Platform) -> Result<HashMap<String, String>, String> {
    let file: KeymapFile =
        serde_json::from_str(json).map_err(|err| format!("invalid keymap JSON: {err}"))?;
    Ok(flatten_keymap(file, platform))
}

fn flatten_keymap(file: KeymapFile, platform: Platform) -> HashMap<String, String> {
    let mut map = file.bindings;
    let overlay = match platform {
        Platform::Macos => file.macos,
        Platform::Linux => file.linux,
        Platform::Windows => file.windows,
    };
    merge_bindings(&mut map, overlay);
    map
}

fn merge_bindings(base: &mut HashMap<String, String>, overlay: HashMap<String, String>) {
    base.extend(overlay);
}

fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

fn read_user_keymap() -> Option<Result<String, String>> {
    let path = user_keymap_path()?;
    if !path.is_file() {
        return None;
    }
    Some(std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display())))
}

pub fn user_config_dir() -> Option<PathBuf> {
    field_scripting::user_config_dir()
}

fn user_keymap_path() -> Option<PathBuf> {
    Some(user_config_dir()?.join("keymap.json"))
}

fn valid_keystrokes(spec: &str, os: OperatingSystem) -> bool {
    let mut parts = spec.split_whitespace().peekable();
    if parts.peek().is_none() {
        return false;
    }
    parts.all(|part| Keystroke::parse_with_os(part, os).is_ok())
}

#[cfg(test)]
fn os_for(platform: Platform) -> OperatingSystem {
    match platform {
        Platform::Macos => OperatingSystem::Mac,
        Platform::Linux => OperatingSystem::Linux,
        Platform::Windows => OperatingSystem::Windows,
    }
}

fn contexts_for(command_id: &str) -> &'static [&'static str] {
    match command_id {
        "view.fit_all"
        | "view.frame"
        | "selection.add_marker"
        | "selection.add_note"
        | "selection.delete_marker"
        | "transport.play_pause" => &[WAVEFORM_KEY_CONTEXT, WAVEFORM_HOVER_KEY_CONTEXT],
        _ => &[APP_KEY_CONTEXT],
    }
}

fn bindings_for(command_id: &str, keystrokes: &str, cx: &App) -> Vec<KeyBinding> {
    contexts_for(command_id)
        .iter()
        .filter_map(|context| binding_in(command_id, keystrokes, context, cx))
        .collect()
}

/// Whether a keymap entry can resolve to an action and parse for `os`.
///
/// Used by tests without constructing a GPUI [`App`] for [`KeyBinding::load`].
#[cfg(test)]
fn can_bind(command_id: &str, keystrokes: &str, os: OperatingSystem) -> bool {
    action_for(command_id).is_some() && valid_keystrokes(keystrokes, os)
}

fn binding_in(command_id: &str, keystrokes: &str, context: &str, cx: &App) -> Option<KeyBinding> {
    if !is_known_command(command_id) {
        eprintln!("FieldAssist: unknown command {command_id:?} bound to {keystrokes:?}");
        return None;
    }
    let os = cx.platform_info().os;
    if !valid_keystrokes(keystrokes, os) {
        eprintln!("FieldAssist: invalid keystroke {keystrokes:?} for {command_id}");
        return None;
    }
    let action = action_for(command_id)?;
    let predicate = match KeyBindingContextPredicate::parse(context) {
        Ok(predicate) => Rc::new(predicate),
        Err(err) => {
            eprintln!("FieldAssist: invalid key context {context:?}: {err}");
            return None;
        }
    };
    match KeyBinding::load(keystrokes, action, Some(predicate), false, None, cx) {
        Ok(binding) => Some(binding),
        Err(err) => {
            eprintln!(
                "FieldAssist: failed to load keybinding {keystrokes:?} for {command_id}: {err}"
            );
            None
        }
    }
}

fn action_for(command_id: &str) -> Option<Box<dyn Action>> {
    Some(match command_id {
        "file.open" => Box::new(Open),
        "file.save" => Box::new(Save),
        "file.save_as" => Box::new(SaveAs),
        "file.save_session" => Box::new(SaveSession),
        "file.save_session_as" => Box::new(SaveSessionAs),
        "file.close_session" => Box::new(CloseSession),
        "file.close" => Box::new(Close),
        "file.export" => Box::new(Export),
        "file.quit" => Box::new(Quit),
        "app.hide" => Box::new(Hide),
        "app.hide_others" => Box::new(HideOthers),
        "app.show_all" => Box::new(ShowAll),
        "app.settings" => Box::new(Settings),
        "app.variables" => Box::new(UserVariables),
        "app.install_cli_tools" => Box::new(InstallCliTools),
        "help.online_docs" => Box::new(HelpOnlineDocs),
        "help.about" => Box::new(About),
        "view.fit_all" => Box::new(ViewFitAll),
        "view.frame" => Box::new(ViewFrame),
        "view.zoom_in" => Box::new(ViewZoomIn),
        "view.zoom_out" => Box::new(ViewZoomOut),
        "view.show-explorer" => Box::new(ViewShowExplorer),
        "view.hide-explorer" => Box::new(ViewHideExplorer),
        "view.toggle-explorer" => Box::new(ViewExplorer),
        "view.show-detail" => Box::new(ViewShowDetail),
        "view.hide-detail" => Box::new(ViewHideDetail),
        "view.toggle-detail" => Box::new(ViewDetail),
        "view.show-script" => Box::new(ViewShowScript),
        "view.hide-script" => Box::new(ViewHideScript),
        "view.toggle-script" => Box::new(ViewScript),
        "view.show-media" => Box::new(ViewShowMedia),
        "view.toggle-media" => Box::new(ViewToggleMedia),
        "view.wrap_messages" => Box::new(ViewWrapMessages),
        "view.follow_playhead" => Box::new(ViewFollowPlayhead),
        "transport.home" => Box::new(TransportHome),
        "transport.previous" => Box::new(TransportPrevious),
        "transport.start" => Box::new(TransportStart),
        "transport.play_pause" => Box::new(TransportPlayPause),
        "transport.stop" => Box::new(TransportStop),
        "transport.next" => Box::new(TransportNext),
        "transport.end" => Box::new(TransportEnd),
        "transport.loop" => Box::new(TransportLoop),
        "transport.preview" => Box::new(TransportPreview),
        "edit.undo" => Box::new(EditUndo),
        "edit.redo" => Box::new(EditRedo),
        "edit.cut" => Box::new(EditCut),
        "edit.copy" => Box::new(EditCopy),
        "edit.paste" => Box::new(EditPaste),
        "edit.clear" => Box::new(EditClear),
        "edit.remove" => Box::new(EditRemove),
        "edit.duplicate" => Box::new(EditDuplicate),
        "edit.trim" => Box::new(EditTrim),
        "edit.break_out_regions" => Box::new(EditBreakOutRegions),
        "edit.break_out_channels" => Box::new(EditBreakOutChannels),
        "selection.select_all" => Box::new(SelectAll),
        "selection.select_none" => Box::new(SelectNone),
        "selection.invert" => Box::new(InvertSelection),
        "selection.zero_crossing" => Box::new(ZeroCrossing),
        "selection.marker_type_blue" => Box::new(MarkerTypeBlue),
        "selection.marker_type_yellow" => Box::new(MarkerTypeYellow),
        "selection.marker_type_purple" => Box::new(MarkerTypePurple),
        "selection.snap_to_marker" => Box::new(SnapToMarker),
        "selection.add_at_hover" => Box::new(AddMarkerAtHover),
        "selection.add_marker" => Box::new(AddMarker),
        "selection.add_note" => Box::new(AddNote),
        "selection.delete_marker" => Box::new(DeleteMarker),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_parses_and_command_ids_resolve() {
        for platform in [Platform::Macos, Platform::Linux, Platform::Windows] {
            let map = parse_and_flatten(DEFAULT_KEYMAP, platform).expect("default keymap parses");
            let os = os_for(platform);
            assert!(!map.is_empty());
            for (keystrokes, command_id) in &map {
                assert!(
                    is_known_command(command_id),
                    "unknown command {command_id:?} on {keystrokes:?}"
                );
                assert!(
                    can_bind(command_id, keystrokes, os),
                    "failed to bind {command_id:?} to {keystrokes:?} on {platform:?}"
                );
            }
        }
    }

    #[test]
    fn default_keymap_binds_requested_view_and_transport_keys() {
        let map = parse_and_flatten(DEFAULT_KEYMAP, Platform::Macos).unwrap();
        assert_eq!(map.get("a").map(String::as_str), Some("view.fit_all"));
        assert_eq!(map.get("f").map(String::as_str), Some("view.frame"));
        assert_eq!(
            map.get("m").map(String::as_str),
            Some("selection.add_marker")
        );
        assert_eq!(map.get("n").map(String::as_str), Some("selection.add_note"));
        assert_eq!(
            map.get("d").map(String::as_str),
            Some("selection.delete_marker")
        );
        assert_eq!(
            map.get("secondary-a").map(String::as_str),
            Some("selection.select_all")
        );
        assert_eq!(
            map.get("secondary-=").map(String::as_str),
            Some("view.zoom_in")
        );
        assert_eq!(
            map.get("secondary-+").map(String::as_str),
            Some("view.zoom_in")
        );
        assert_eq!(
            map.get("secondary-shift-=").map(String::as_str),
            Some("view.zoom_in")
        );
        assert_eq!(
            map.get("secondary--").map(String::as_str),
            Some("view.zoom_out")
        );
        assert_eq!(
            map.get("secondary-0").map(String::as_str),
            Some("view.fit_all")
        );
        assert_eq!(
            map.get("space").map(String::as_str),
            Some("transport.play_pause")
        );
        assert_eq!(map.get("cmd-o").map(String::as_str), Some("file.open"));
        assert_eq!(map.get("cmd-s").map(String::as_str), Some("file.save"));
        assert_eq!(map.get("cmd-,").map(String::as_str), Some("app.settings"));
        assert_eq!(
            map.get("cmd-shift-s").map(String::as_str),
            Some("file.save_as")
        );
        assert_eq!(map.get("cmd-w").map(String::as_str), Some("file.close"));
        assert_eq!(map.get("cmd-z").map(String::as_str), Some("edit.undo"));
        assert_eq!(map.get("cmd-x").map(String::as_str), Some("edit.cut"));
        assert_eq!(map.get("cmd-c").map(String::as_str), Some("edit.copy"));
        assert_eq!(map.get("cmd-v").map(String::as_str), Some("edit.paste"));
        assert_eq!(map.get("backspace").map(String::as_str), Some("edit.clear"));
        assert_eq!(
            map.get("shift-backspace").map(String::as_str),
            Some("edit.remove")
        );
        assert_eq!(
            map.get("shift-delete").map(String::as_str),
            Some("edit.remove")
        );
        assert!(map.get("ctrl-o").is_none());
    }

    #[test]
    fn settings_accelerator_per_platform() {
        let macos = parse_and_flatten(DEFAULT_KEYMAP, Platform::Macos).unwrap();
        assert_eq!(macos.get("cmd-,").map(String::as_str), Some("app.settings"));
        for platform in [Platform::Linux, Platform::Windows] {
            let map = parse_and_flatten(DEFAULT_KEYMAP, platform).unwrap();
            assert_eq!(
                map.get("ctrl-,").map(String::as_str),
                Some("app.settings"),
                "{platform:?}"
            );
        }
    }

    #[test]
    fn platform_overlay_replaces_shared_keys() {
        let json = r#"{
            "bindings": { "o": "file.open" },
            "linux": { "o": "file.quit" }
        }"#;
        let linux = parse_and_flatten(json, Platform::Linux).unwrap();
        let macos = parse_and_flatten(json, Platform::Macos).unwrap();
        assert_eq!(linux.get("o").map(String::as_str), Some("file.quit"));
        assert_eq!(macos.get("o").map(String::as_str), Some("file.open"));
    }

    #[test]
    fn merge_overlay_wins_per_keystroke() {
        let mut base = parse_and_flatten(DEFAULT_KEYMAP, Platform::Linux).unwrap();
        let user = parse_and_flatten(
            r#"{ "bindings": { "a": "view.frame", "g": "view.fit_all" } }"#,
            Platform::Linux,
        )
        .unwrap();
        merge_bindings(&mut base, user);
        assert_eq!(base.get("a").map(String::as_str), Some("view.frame"));
        assert_eq!(base.get("g").map(String::as_str), Some("view.fit_all"));
        assert_eq!(
            base.get("space").map(String::as_str),
            Some("transport.play_pause")
        );
    }

    #[test]
    fn unknown_command_ids_are_skipped() {
        assert!(!is_known_command("not.a.command"));
        assert!(action_for("not.a.command").is_none());
        assert!(!can_bind("not.a.command", "a", OperatingSystem::Mac));
    }

    #[test]
    fn help_online_docs_is_a_known_command() {
        assert!(is_known_command("help.online_docs"));
        assert!(can_bind("help.online_docs", "f12", OperatingSystem::Mac));
        assert!(!is_known_command("not.a.command"));
    }

    #[test]
    fn keymap_bindings_are_scoped_to_the_app_view() {
        assert_eq!(contexts_for("edit.clear"), &[APP_KEY_CONTEXT]);
        assert_eq!(contexts_for("transport.stop"), &[APP_KEY_CONTEXT]);
        assert!(can_bind("edit.clear", "backspace", OperatingSystem::Mac));
        assert!(can_bind("transport.stop", "k", OperatingSystem::Mac));
    }

    #[test]
    fn letter_keys_are_scoped_to_the_waveform() {
        for (command_id, keystrokes) in [
            ("view.fit_all", "a"),
            ("view.frame", "f"),
            ("selection.add_marker", "m"),
            ("selection.add_note", "n"),
            ("selection.delete_marker", "d"),
            ("transport.play_pause", "space"),
        ] {
            let contexts = contexts_for(command_id);
            assert_eq!(contexts.len(), 2, "{command_id}");
            assert!(contexts.contains(&WAVEFORM_KEY_CONTEXT), "{command_id}");
            assert!(
                contexts.contains(&WAVEFORM_HOVER_KEY_CONTEXT),
                "{command_id}"
            );
            assert!(
                can_bind(command_id, keystrokes, OperatingSystem::Mac),
                "{command_id}"
            );
        }
    }

    #[test]
    fn play_pause_space_is_bound_to_waveform_hover() {
        // Hover-without-click relies on WaveformHover; if that context drops
        // after the first Space (GPUI default hover modality), toggle breaks
        // until the mouse moves (issue #15).
        let contexts = contexts_for("transport.play_pause");
        assert_eq!(contexts.len(), 2);
        assert!(
            contexts.contains(&WAVEFORM_HOVER_KEY_CONTEXT),
            "expected WaveformHover binding, got {contexts:?}"
        );
        assert!(
            contexts.contains(&WAVEFORM_KEY_CONTEXT),
            "expected Waveform binding, got {contexts:?}"
        );
        assert!(can_bind(
            "transport.play_pause",
            "space",
            OperatingSystem::Mac
        ));
    }

    #[test]
    fn dispatch_rejects_unknown_command_ids() {
        assert_eq!(KNOWN_COMMANDS.len(), known_commands().len());
        for id in known_commands() {
            assert!(is_known_command(id));
            assert!(validate_command_id(id).is_ok());
        }
        let err = validate_command_id("not.a.command").unwrap_err();
        assert!(err.contains("unknown command"));
    }

    #[test]
    fn pane_view_commands_are_known_and_bindable() {
        for id in [
            "view.show-explorer",
            "view.hide-explorer",
            "view.toggle-explorer",
            "view.show-detail",
            "view.hide-detail",
            "view.toggle-detail",
            "view.show-script",
            "view.hide-script",
            "view.toggle-script",
            "view.show-media",
            "view.toggle-media",
        ] {
            assert!(is_known_command(id), "{id}");
            assert!(can_bind(id, "f12", OperatingSystem::Mac), "{id}");
        }
        assert!(!is_known_command("view.explorer"));
        assert!(!is_known_command("view.detail"));
        assert!(!is_known_command("view.script"));
    }

    #[test]
    fn invalid_json_is_an_error() {
        let err = parse_and_flatten("{not json", Platform::Macos).unwrap_err();
        assert!(err.contains("invalid keymap JSON"));
    }
}
