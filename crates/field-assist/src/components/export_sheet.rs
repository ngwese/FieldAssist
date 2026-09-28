// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use field_audio_io::{
    build_tag_map, encoder, encoders, format_rate, EncodeSpec, PcmFormat, RATE_PRESETS,
};
use field_composition::ExportJob;
use field_scripting::{
    resolve_export_settings, ExportChannels, ExportProfileDef, ExportSourceDefaults,
    ResolvedExportSettings,
};
use field_ui_components::{ChannelSelector, VariableCompletion, VariableRow};
use field_variables::{compose, interpolate, interpolate_strict, VariableEntry, VariableTable};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputEvent,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, rems, App, AppContext as _, Context, DispatchPhase, Entity, ExternalPaths, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, PathPromptOptions, Pixels, Render,
    StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::components::explorer::CompositionDrag;
use crate::components::variables_filter::{self, VariablesFilter};
use crate::components::variables_panel::rows_from_composed;
use crate::model::composition::Composition;

/// Optional export defaults inherited from parent/session properties.
///
/// Applied as an overlay on top of composition/media source defaults when the
/// sheet opens with no profile selected (same layering as a sparse profile).
#[derive(Debug, Clone, Default)]
pub struct ExportPrefs {
    /// Encoder id (`wav`, `flac`, …).
    pub encoder: Option<String>,
    /// PCM sample format when the encoder supports it.
    pub sample_format: Option<PcmFormat>,
    /// Output sample rate.
    pub sample_rate: Option<u32>,
    /// Channel enable mask matching the composition channel count.
    pub channels_selected: Option<Vec<bool>>,
}

/// Layer tables for Export-site compose (source → user → session → composition → export).
#[derive(Clone, Debug, Default)]
pub struct ExportVariableLayers {
    /// Probe / technical source variables.
    pub source: VariableTable,
    /// User-scoped variables.
    pub user: VariableTable,
    /// Session-scoped variables.
    pub session: VariableTable,
    /// Composition-scoped variables.
    pub composition: VariableTable,
}

const LABEL_WIDTH: gpui_kit::Rems = rems(6.5);
const VALUE_WIDTH: gpui_kit::Rems = rems(10.);
/// Cancel / Export action buttons (~2× default xsmall content width).
const ACTION_BUTTON_WIDTH: gpui_kit::Rems = rems(5.);
const CUSTOM_PROFILE_LABEL: &str = "Custom";
const MIN_COLUMN_WIDTH: f32 = 48.;
const RESIZE_HANDLE_WIDTH: f32 = 5.;
const VARIABLES_TABLE_MAX_H: f32 = 200.;
/// Default Location → Directory template when the profile does not set one.
const DEFAULT_DIRECTORY: &str = "${source.parent}";
/// Default Location → Name template when the profile does not set one.
const DEFAULT_FILENAME: &str = "${source.stem}-${sample_rate}-${channel_layout}.${export.encoder}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ExportVarColumn {
    Name,
    Value,
    Scope,
}

impl ExportVarColumn {
    const ALL: [Self; 3] = [Self::Name, Self::Value, Self::Scope];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Value => "value",
            Self::Scope => "scope",
        }
    }

    fn default_width(self) -> Pixels {
        match self {
            Self::Name => px(140.),
            Self::Value => px(220.),
            Self::Scope => px(100.),
        }
    }

    fn stable_id(self) -> u32 {
        match self {
            Self::Name => 0,
            Self::Value => 1,
            Self::Scope => 2,
        }
    }
}

fn default_export_column_widths() -> HashMap<ExportVarColumn, Pixels> {
    ExportVarColumn::ALL
        .into_iter()
        .map(|col| (col, col.default_width()))
        .collect()
}

struct VarResizeDrag {
    column: ExportVarColumn,
    start_x: f32,
    start_width: Pixels,
}

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct ExportSheet {
    /// Registered profiles available in the Profile menu.
    profiles: Vec<ExportProfileDef>,
    /// Selected profile name, or `None` for Custom (manual / prefs-seeded).
    profile_name: Option<String>,
    /// Composition/media defaults captured when the sheet was configured.
    source: ExportSourceDefaults,
    encoder_id: String,
    sample_format: Option<PcmFormat>,
    sample_rate: u32,
    channels_selected: Vec<bool>,
    channel_labels: Vec<String>,
    directory: Entity<VariableCompletion>,
    filename: Entity<VariableCompletion>,
    /// Profile Lua `variables` plus live Format/channel upserts.
    export_vars: VariableTable,
    /// Profile `metadata` templates for tag write-out.
    metadata: BTreeMap<String, String>,
    /// Outer variable layers for Export-site compose.
    layers: ExportVariableLayers,
    variables_open: bool,
    filter: VariablesFilter,
    column_widths: HashMap<ExportVarColumn, Pixels>,
    resize_drag: Option<VarResizeDrag>,
    on_cancel: Option<SheetAction>,
    on_export: Option<SheetAction>,
}

impl ExportSheet {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let directory = cx.new(|cx| VariableCompletion::new(window, cx));
        let filename = cx.new(|cx| VariableCompletion::new(window, cx));
        let directory_input = directory.read(cx).input().clone();
        let filename_input = filename.read(cx).input().clone();
        cx.subscribe(&directory_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&filename_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        Self {
            profiles: Vec::new(),
            profile_name: None,
            source: ExportSourceDefaults {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S24),
                channel_count: 1,
                display_name: "export".into(),
            },
            encoder_id: "wav".into(),
            sample_format: Some(PcmFormat::S24),
            sample_rate: 48_000,
            channels_selected: vec![true],
            channel_labels: vec!["Mono".into()],
            directory,
            filename,
            export_vars: VariableTable::new(),
            metadata: BTreeMap::new(),
            layers: ExportVariableLayers::default(),
            variables_open: false,
            filter: VariablesFilter::default(),
            column_widths: default_export_column_widths(),
            resize_drag: None,
            on_cancel: None,
            on_export: None,
        }
    }

    /// Register Cancel / Export actions (wired from the host overlay).
    pub fn set_actions(&mut self, on_cancel: SheetAction, on_export: SheetAction) {
        self.on_cancel = Some(on_cancel);
        self.on_export = Some(on_export);
    }

    /// Configure from composition defaults (no inherited prefs).
    #[allow(dead_code)]
    pub fn configure(
        &mut self,
        composition: &Composition,
        profiles: Vec<ExportProfileDef>,
        layers: ExportVariableLayers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.configure_with_prefs(composition, profiles, None, layers, window, cx);
    }

    /// Configure from composition source defaults, then optional session prefs.
    ///
    /// Resolution when opening (no profile selected):
    /// 1. Source defaults from the composition / media
    /// 2. Session/document `export.*` prefs as a sparse overlay (same rules as a profile)
    ///
    /// Choosing a named profile later re-resolves as source ← profile (prefs are
    /// not mixed into a named profile).
    ///
    /// Location Directory / Name default to [`DEFAULT_DIRECTORY`] /
    /// [`DEFAULT_FILENAME`] when the profile does not set them.
    pub fn configure_with_prefs(
        &mut self,
        composition: &Composition,
        profiles: Vec<ExportProfileDef>,
        prefs: Option<&ExportPrefs>,
        layers: ExportVariableLayers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.profiles = profiles;
        self.profile_name = None;
        self.layers = layers;
        self.metadata.clear();
        self.variables_open = false;
        self.source = ExportSourceDefaults::from_composition(composition);
        self.channel_labels = (0..self.source.channel_count)
            .map(|ch| composition.channel_label(ch))
            .collect();

        // Clear location fields so template defaults apply on open.
        self.set_directory_text("", window, cx);
        self.set_filename_text("", window, cx);

        let overlay = prefs.map(prefs_as_profile).unwrap_or_default();
        let resolved = resolve_export_settings(&self.source, &overlay)
            .unwrap_or_else(|_| fallback_resolved(&self.source));
        self.apply_resolved(&resolved, window, cx);
        self.rebuild_export_vars(VariableTable::new(), window, cx);
        self.filter.ensure_search(window, cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Apply a named export profile (source ← profile) and update the sheet.
    pub fn select_profile(
        &mut self,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match name {
            Some(name) => {
                let profile = self.profiles.iter().find(|p| p.name == name).cloned();
                let Some(profile) = profile else {
                    cx.notify();
                    return;
                };
                self.profile_name = Some(name.to_string());
                self.metadata = profile.metadata.clone();
                match resolve_export_settings(&self.source, &profile) {
                    Ok(resolved) => self.apply_resolved(&resolved, window, cx),
                    Err(_) => {
                        cx.notify();
                        return;
                    }
                }
                self.rebuild_export_vars(profile.variables, window, cx);
            }
            None => {
                self.profile_name = None;
                self.metadata.clear();
                let resolved = resolve_export_settings(&self.source, &ExportProfileDef::default())
                    .unwrap_or_else(|_| fallback_resolved(&self.source));
                self.apply_resolved(&resolved, window, cx);
                self.rebuild_export_vars(VariableTable::new(), window, cx);
            }
        }
        cx.notify();
    }

    fn apply_resolved(
        &mut self,
        resolved: &ResolvedExportSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.encoder_id = resolved.encoder_id.clone();
        self.sample_format = resolved.sample_format;
        self.sample_rate = resolved.sample_rate;
        let count = self.source.channel_count;
        self.channels_selected = (0..count)
            .map(|i| resolved.channel_indices.contains(&i))
            .collect();

        let (dir, name) =
            destination_for_sheet(resolved, &self.directory_text(cx), &self.filename_text(cx));
        self.set_directory_text(dir, window, cx);
        self.set_filename_text(name, window, cx);
    }

    fn rebuild_export_vars(
        &mut self,
        profile_vars: VariableTable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channels = self.selected_count();
        self.export_vars = profile_vars;
        upsert_format_export_vars(
            &mut self.export_vars,
            &self.encoder_id,
            self.sample_format,
            self.sample_rate,
            channels,
        );
        self.refresh_completion_tables(window, cx);
    }

    fn sync_format_export_vars(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let channels = self.selected_count();
        upsert_format_export_vars(
            &mut self.export_vars,
            &self.encoder_id,
            self.sample_format,
            self.sample_rate,
            channels,
        );
        self.refresh_completion_tables(window, cx);
    }

    fn refresh_completion_tables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let table = self.compose_rust();
        self.directory.update(cx, |completion, cx| {
            completion.set_table(table.clone(), window, cx);
        });
        self.filename.update(cx, |completion, cx| {
            completion.set_table(table, window, cx);
        });
    }

    fn directory_text(&self, cx: &App) -> String {
        self.directory.read(cx).input().read(cx).value().to_string()
    }

    fn filename_text(&self, cx: &App) -> String {
        self.filename.read(cx).input().read(cx).value().to_string()
    }

    fn set_directory_text(
        &mut self,
        value: impl Into<gpui_kit::SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.directory.read(cx).input().clone();
        input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
    }

    fn set_filename_text(
        &mut self,
        value: impl Into<gpui_kit::SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.filename.read(cx).input().clone();
        input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
    }

    /// Whether either Location field has an open completion menu.
    pub fn completion_menu_open(&self, cx: &App) -> bool {
        self.directory.read(cx).menu_open() || self.filename.read(cx).menu_open()
    }

    /// Dismiss completion menus on Directory / Name without closing the sheet.
    pub fn dismiss_completion_menus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.directory.update(cx, |completion, cx| {
            completion.dismiss(window, cx);
        });
        self.filename.update(cx, |completion, cx| {
            completion.dismiss(window, cx);
        });
    }

    fn selected_count(&self) -> u16 {
        self.channels_selected.iter().filter(|on| **on).count() as u16
    }

    fn spec(&self) -> EncodeSpec {
        EncodeSpec {
            sample_rate: self.sample_rate,
            sample_format: self.sample_format,
            channel_count: self.selected_count().max(1),
        }
    }

    fn set_encoder(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(encoder) = encoder(id) else {
            return;
        };
        self.profile_name = None;
        self.encoder_id = encoder.id().into();
        // Re-snap format against source preference when the encoder changes.
        let preferred = self
            .sample_format
            .or(self.source.sample_format)
            .or(Some(PcmFormat::S24));
        let overlay = ExportProfileDef {
            encoder: Some(self.encoder_id.clone()),
            sample_format: preferred,
            sample_rate: Some(self.sample_rate),
            channels: Some(ExportChannels::Indices(
                self.channels_selected
                    .iter()
                    .enumerate()
                    .filter_map(|(i, on)| on.then_some(i))
                    .collect(),
            )),
            ..ExportProfileDef::default()
        };
        if let Ok(resolved) = resolve_export_settings(&self.source, &overlay) {
            self.sample_format = resolved.sample_format;
        }
        self.sync_format_export_vars(window, cx);
        cx.notify();
    }

    pub fn can_export(&self, cx: &App) -> bool {
        let Some(encoder) = encoder(&self.encoder_id) else {
            return false;
        };
        if self.selected_count() == 0 {
            return false;
        }
        if self.directory_text(cx).trim().is_empty() {
            return false;
        }
        if self.filename_text(cx).trim().is_empty() {
            return false;
        }
        encoder.supports(&self.spec())
    }

    /// Snapshot of layer tables + current export scope (for host Lua resolve).
    pub fn variable_layers(&self) -> (ExportVariableLayers, VariableTable) {
        (self.layers.clone(), self.export_vars.clone())
    }

    /// Rust last-wins Export-site compose (preview / fallback).
    pub fn compose_rust(&self) -> VariableTable {
        compose_export_site(
            &self.layers.source,
            &self.layers.user,
            &self.layers.session,
            &self.layers.composition,
            &self.export_vars,
        )
    }

    /// Soft-interpolated destination path for the Resolved preview row.
    pub fn resolved_path_preview(&self, cx: &App) -> String {
        soft_resolved_path(
            &self.directory_text(cx),
            &self.filename_text(cx),
            &self.compose_rust(),
        )
    }

    /// Build an [`ExportJob`] using a pre-composed Export-site table.
    pub fn job_from_composed(&self, composed: &VariableTable, cx: &App) -> Option<ExportJob> {
        if !self.can_export(cx) {
            return None;
        }
        let directory_raw = self.directory_text(cx);
        let filename_raw = self.filename_text(cx);
        let directory = interpolate_strict(directory_raw.trim(), composed).ok()?;
        let filename = interpolate_strict(filename_raw.trim(), composed).ok()?;
        if directory.is_empty() || filename.is_empty() {
            return None;
        }
        let dest = PathBuf::from(directory).join(filename);
        let tags = build_tag_map(composed, &self.metadata).ok()?;
        Some(ExportJob {
            encoder_id: self.encoder_id.clone(),
            spec: self.spec(),
            channel_indices: self
                .channels_selected
                .iter()
                .enumerate()
                .filter_map(|(i, on)| on.then_some(i))
                .collect(),
            dest,
            tags,
        })
    }

    fn prompt_directory(&self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.set_directory_text(path.to_string_lossy().into_owned(), window, cx);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn apply_dropped_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf())
        };
        self.set_directory_text(dir.to_string_lossy().into_owned(), window, cx);
        cx.notify();
    }

    fn profile_label(&self) -> String {
        match &self.profile_name {
            Some(name) => name.clone(),
            None => CUSTOM_PROFILE_LABEL.into(),
        }
    }

    fn toggle_variables(&mut self, cx: &mut Context<Self>) {
        self.variables_open = !self.variables_open;
        cx.notify();
    }

    fn column_width(&self, col: ExportVarColumn) -> Pixels {
        self.column_widths
            .get(&col)
            .copied()
            .unwrap_or_else(|| col.default_width())
    }

    fn content_width(&self) -> Pixels {
        let cols: f32 = ExportVarColumn::ALL
            .into_iter()
            .map(|col| f32::from(self.column_width(col)) + RESIZE_HANDLE_WIDTH)
            .sum();
        px(cols)
    }

    fn begin_resize(&mut self, column: ExportVarColumn, start_x: Pixels) {
        self.resize_drag = Some(VarResizeDrag {
            column,
            start_x: f32::from(start_x),
            start_width: self.column_width(column),
        });
    }

    fn apply_resize_drag(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.resize_drag.as_ref() else {
            return;
        };
        let column = drag.column;
        let delta = f32::from(x) - drag.start_x;
        let new_width = px((f32::from(drag.start_width) + delta).max(MIN_COLUMN_WIDTH));
        self.column_widths.insert(column, new_width);
        cx.notify();
    }

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resize_drag.take().is_some() {
            cx.notify();
        }
    }

    fn render_variables_table(
        &self,
        rows: &[VariableRow],
        filtered: &[usize],
        muted: Hsla,
        border: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content_width = self.content_width();
        let widths: HashMap<ExportVarColumn, Pixels> = ExportVarColumn::ALL
            .into_iter()
            .map(|col| (col, self.column_width(col)))
            .collect();

        let header = h_flex()
            .id("export-var-header")
            .w(content_width)
            .flex_none()
            .items_center()
            .px_1()
            .py_0p5()
            .children(ExportVarColumn::ALL.into_iter().flat_map(|col| {
                let width = widths[&col];
                [
                    div()
                        .w(width)
                        .flex_none()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(muted)
                        .child(col.label())
                        .into_any_element(),
                    resize_handle(col, cx).into_any_element(),
                ]
            }));

        let body = if filtered.is_empty() {
            div()
                .w(content_width)
                .px_1()
                .py_1()
                .text_xs()
                .text_color(muted)
                .child(if rows.is_empty() {
                    "(no variables)"
                } else {
                    "(no matches)"
                })
                .into_any_element()
        } else {
            v_flex()
                .w(content_width)
                .children(filtered.iter().map(|&ix| {
                    let row = &rows[ix];
                    h_flex()
                        .w(content_width)
                        .px_1()
                        .py_0p5()
                        .items_center()
                        .children(ExportVarColumn::ALL.into_iter().flat_map(|col| {
                            let width = widths[&col];
                            let text = match col {
                                ExportVarColumn::Name => row.name.clone(),
                                ExportVarColumn::Value => row.value.clone(),
                                ExportVarColumn::Scope => row.scope.clone(),
                            };
                            let cell = div()
                                .w(width)
                                .flex_none()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_xs()
                                .when(col == ExportVarColumn::Scope, |el| el.text_color(muted))
                                .child(text);
                            [
                                cell.into_any_element(),
                                div()
                                    .w(px(RESIZE_HANDLE_WIDTH))
                                    .flex_none()
                                    .into_any_element(),
                            ]
                        }))
                }))
                .into_any_element()
        };

        // Match Variables dock / User Variables: fixed header, full-width
        // underline, no left/right table borders; body scrolls separately.
        v_flex()
            .id("export-variables-table")
            .w_full()
            .child(
                div()
                    .id("export-var-header-clip")
                    .w_full()
                    .flex_none()
                    .overflow_x_scroll()
                    .border_b_1()
                    .border_color(border)
                    .child(header),
            )
            .child(
                div()
                    .id("export-var-body")
                    .w_full()
                    .max_h(px(VARIABLES_TABLE_MAX_H))
                    .overflow_y_scroll()
                    .overflow_x_scroll()
                    .child(body),
            )
    }
}

fn resize_handle(column: ExportVarColumn, cx: &mut Context<ExportSheet>) -> impl IntoElement {
    h_flex()
        .id(("export-var-resize", column.stable_id()))
        .w(px(RESIZE_HANDLE_WIDTH))
        .flex_none()
        .self_stretch()
        .occlude()
        .cursor_col_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                this.begin_resize(column, e.position.x);
                cx.notify();
            }),
        )
}

fn install_resize_listeners(entity: Entity<ExportSheet>, window: &mut Window) {
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            entity.update(cx, |this, cx| {
                if this.resize_drag.is_some() {
                    this.apply_resize_drag(event.position.x, cx);
                }
            });
        }
    });
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            entity.update(cx, |this, cx| {
                this.end_resize(cx);
            });
        }
    });
}

/// Upsert Format/channel-driven leaves into the export variable table.
pub fn upsert_format_export_vars(
    table: &mut VariableTable,
    encoder_id: &str,
    sample_format: Option<PcmFormat>,
    sample_rate: u32,
    channels: u16,
) {
    let extension = encoder(encoder_id)
        .map(|enc| enc.extension().to_string())
        .unwrap_or_default();
    let sample_format_label = sample_format
        .filter(|_| {
            encoder(encoder_id)
                .map(|enc| enc.capabilities().stores_sample_format())
                .unwrap_or(false)
        })
        .map(PcmFormat::label)
        .unwrap_or("");
    for (name, value) in [
        ("encoder", encoder_id.to_string()),
        ("sample_format", sample_format_label.to_string()),
        ("sample_rate", sample_rate.to_string()),
        ("channels", channels.to_string()),
        ("extension", extension),
    ] {
        table.upsert(VariableEntry::new("export", name, value));
    }
}

/// Soft-interpolate directory + name and join (Resolved preview).
pub fn soft_resolved_path(directory: &str, filename: &str, composed: &VariableTable) -> String {
    let dir = interpolate(directory.trim(), composed);
    let name = interpolate(filename.trim(), composed);
    if dir.is_empty() {
        return name;
    }
    if name.is_empty() {
        return dir;
    }
    PathBuf::from(dir).join(name).to_string_lossy().into_owned()
}

/// Rust last-wins Export-site compose.
pub fn compose_export_site(
    source: &VariableTable,
    user: &VariableTable,
    session: &VariableTable,
    composition: &VariableTable,
    export: &VariableTable,
) -> VariableTable {
    compose(&[
        source.clone(),
        user.clone(),
        session.clone(),
        composition.clone(),
        export.clone(),
    ])
}

fn prefs_as_profile(prefs: &ExportPrefs) -> ExportProfileDef {
    let channels = prefs.channels_selected.as_ref().map(|mask| {
        ExportChannels::Indices(
            mask.iter()
                .enumerate()
                .filter_map(|(i, on)| on.then_some(i))
                .collect(),
        )
    });
    ExportProfileDef {
        encoder: prefs.encoder.clone(),
        sample_format: prefs.sample_format,
        sample_rate: prefs.sample_rate,
        channels,
        ..ExportProfileDef::default()
    }
}

fn fallback_resolved(source: &ExportSourceDefaults) -> ResolvedExportSettings {
    ResolvedExportSettings {
        encoder_id: "wav".into(),
        sample_format: source.sample_format.or(Some(PcmFormat::S24)),
        sample_rate: source.sample_rate,
        channel_indices: (0..source.channel_count).collect(),
        path: None,
        directory: None,
        filename: None,
    }
}

/// Build directory + filename strings for the sheet inputs from resolved settings.
fn destination_for_sheet(
    resolved: &ResolvedExportSettings,
    current_directory: &str,
    current_filename: &str,
) -> (String, String) {
    if let Some(path) = &resolved.path {
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| DEFAULT_FILENAME.to_string());
        return (dir, name);
    }
    let dir = resolved
        .directory
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let cur = current_directory.trim();
            (!cur.is_empty()).then(|| cur.to_string())
        })
        .unwrap_or_else(|| DEFAULT_DIRECTORY.to_string());
    let name = resolved
        .filename
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let cur = current_filename.trim();
            (!cur.is_empty()).then(|| cur.to_string())
        })
        .unwrap_or_else(|| DEFAULT_FILENAME.to_string());
    (dir, name)
}

impl Render for ExportSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        install_resize_listeners(cx.entity(), window);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let drop_highlight = theme.secondary;
        let spec = self.spec();
        let encoder_id = self.encoder_id.clone();
        let current_encoder = encoder(&encoder_id);
        let caps = current_encoder.map(|enc| enc.capabilities());
        let stores_format = caps.is_some_and(|caps| caps.stores_sample_format());
        let format_label = if stores_format {
            self.sample_format
                .map(PcmFormat::label)
                .unwrap_or("n/a")
                .to_string()
        } else {
            "n/a".into()
        };
        let rate_label = format_rate(self.sample_rate);
        let codec_label = current_encoder
            .map(|enc| enc.label().to_string())
            .unwrap_or_else(|| encoder_id.clone());
        let rate_choices = rate_choices(self.sample_rate);
        let profile_label = self.profile_label();
        let profile_name = self.profile_name.clone();
        let profiles = self.profiles.clone();
        let can_export = self.can_export(cx);
        let resolved_preview = self.resolved_path_preview(cx);
        let composed_rows = rows_from_composed(&self.compose_rust());
        self.filter.sync_scopes_from_rows(&composed_rows);
        let filtered = self.filter.filtered_indices(&composed_rows, cx);
        let variables_open = self.variables_open;

        let profile_menu = {
            let this = cx.entity();
            let profile_name = profile_name.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu.item(
                    PopupMenuItem::new(CUSTOM_PROFILE_LABEL)
                        .checked(profile_name.is_none())
                        .on_click({
                            let this = this.clone();
                            move |_, window, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.select_profile(None, window, cx);
                                });
                            }
                        }),
                );
                for profile in &profiles {
                    let checked = profile_name.as_deref() == Some(profile.name.as_str());
                    let profile_id = profile.name.clone();
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(profile.name.clone())
                            .checked(checked)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.select_profile(Some(&profile_id), window, cx);
                                });
                            }),
                    );
                }
                menu
            }
        };

        let profile_description = self
            .profile_name
            .as_ref()
            .and_then(|name| {
                self.profiles
                    .iter()
                    .find(|p| &p.name == name)
                    .map(|p| p.description.clone())
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_default();

        let codec_menu = {
            let this = cx.entity();
            let encoder_id = encoder_id.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                for encoder in encoders() {
                    let disabled = !encoder.supports(&spec);
                    let id = encoder.id().to_string();
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(encoder.label())
                            .disabled(disabled)
                            .checked(id == encoder_id)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.set_encoder(&id, window, cx);
                                });
                            }),
                    );
                }
                menu
            }
        };

        let format_menu = {
            let this = cx.entity();
            let encoder_id = encoder_id.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                let Some(encoder) = encoder(&encoder_id) else {
                    return menu;
                };
                let caps = encoder.capabilities();
                if !caps.stores_sample_format() {
                    return menu.label("Vorbis does not store PCM format");
                }
                for format in PcmFormat::ALL {
                    let mut probe = spec;
                    probe.sample_format = Some(format);
                    let disabled = !encoder.supports(&probe);
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format.label())
                            .disabled(disabled)
                            .checked(spec.sample_format == Some(format))
                            .on_click(move |_, window, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.profile_name = None;
                                    sheet.sample_format = Some(format);
                                    sheet.sync_format_export_vars(window, cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            }
        };

        let rate_menu = {
            let this = cx.entity();
            let encoder_id = encoder_id.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                let Some(encoder) = encoder(&encoder_id) else {
                    return menu;
                };
                for rate in rate_choices.iter().copied() {
                    let mut probe = spec;
                    probe.sample_rate = rate;
                    let disabled = !encoder.supports(&probe);
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format_rate(rate))
                            .disabled(disabled)
                            .checked(spec.sample_rate == rate)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |sheet, cx| {
                                    sheet.profile_name = None;
                                    sheet.sample_rate = rate;
                                    sheet.sync_format_export_vars(window, cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            }
        };

        let entity = cx.entity().clone();
        let filter_bar = variables_filter::filter_bar(
            &self.filter,
            &composed_rows,
            entity,
            border,
            false,
            |this, scope, cx| {
                this.filter.toggle_scope(&scope);
                cx.notify();
            },
            |_, _, _| {},
            |_, _, _| {},
        );
        let variables_table =
            self.render_variables_table(&composed_rows, &filtered, muted, border, cx);

        v_flex()
            .gap_3()
            .w_full()
            .child(
                h_flex()
                    .gap_2()
                    .w_full()
                    .items_center()
                    .child(
                        div()
                            .w(LABEL_WIDTH)
                            .flex_none()
                            .flex()
                            .justify_end()
                            .child(div().text_xs().text_color(muted).child("Profile")),
                    )
                    .child(div().w(VALUE_WIDTH).flex_none().child(dropdown(
                        "export-profile",
                        profile_label,
                        false,
                        profile_menu,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(muted)
                            .child(profile_description),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_none()
                            .child(
                                div().w(ACTION_BUTTON_WIDTH).flex_none().child(
                                    Button::new("export-cancel")
                                        .outline()
                                        .xsmall()
                                        .w_full()
                                        .label("Cancel")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let Some(on_cancel) = this.on_cancel.clone() else {
                                                return;
                                            };
                                            // Defer so AppView can update without nesting
                                            // inside this sheet's click borrow.
                                            window.defer(cx, move |window, cx| {
                                                on_cancel(window, cx);
                                            });
                                        })),
                                ),
                            )
                            .child(
                                div().w(ACTION_BUTTON_WIDTH).flex_none().child(
                                    Button::new("export-go")
                                        .primary()
                                        .xsmall()
                                        .w_full()
                                        .label("Export")
                                        .disabled(!can_export)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let Some(on_export) = this.on_export.clone() else {
                                                return;
                                            };
                                            window.defer(cx, move |window, cx| {
                                                on_export(window, cx);
                                            });
                                        })),
                                ),
                            ),
                    ),
            )
            .child(div().w_full().border_b_1().border_color(border))
            .child(
                h_flex()
                    .gap_4()
                    .w_full()
                    .items_start()
                    .child(
                        div().flex_none().child(section(
                            "Format",
                            v_flex()
                                .gap_1()
                                .child(form_row(
                                    "Type",
                                    muted,
                                    Some(VALUE_WIDTH),
                                    dropdown("export-type", codec_label, false, codec_menu),
                                ))
                                .child(form_row(
                                    "Format",
                                    muted,
                                    Some(VALUE_WIDTH),
                                    dropdown(
                                        "export-format",
                                        format_label,
                                        !stores_format,
                                        format_menu,
                                    ),
                                ))
                                .child(form_row(
                                    "Sample Rate",
                                    muted,
                                    Some(VALUE_WIDTH),
                                    dropdown("export-rate", rate_label, false, rate_menu),
                                )),
                        )),
                    )
                    .child(
                        div().flex_1().min_w_0().child(section(
                            "Channels",
                            ChannelSelector::new("export-ch")
                                .channels(self.channel_labels.iter().enumerate().map(
                                    |(i, label)| {
                                        let checked =
                                            self.channels_selected.get(i).copied().unwrap_or(false);
                                        (label.clone(), checked)
                                    },
                                ))
                                .on_toggle(cx.listener(|this, &(i, enabled), window, cx| {
                                    this.profile_name = None;
                                    if let Some(slot) = this.channels_selected.get_mut(i) {
                                        *slot = enabled;
                                    }
                                    this.sync_format_export_vars(window, cx);
                                    cx.notify();
                                })),
                        )),
                    ),
            )
            .child(section(
                "Location",
                v_flex()
                    .gap_1()
                    .w_full()
                    .child(form_row(
                        "Directory",
                        muted,
                        None,
                        h_flex()
                            .gap_2()
                            .w_full()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .can_drop(|data, _, _| {
                                        data.downcast_ref::<ExternalPaths>().is_some()
                                            || data
                                                .downcast_ref::<CompositionDrag>()
                                                .is_some_and(|drag| drag.path.is_some())
                                    })
                                    .drag_over::<ExternalPaths>(move |style, _, _, _| {
                                        style.bg(drop_highlight)
                                    })
                                    .drag_over::<CompositionDrag>(move |style, _, _, _| {
                                        style.bg(drop_highlight)
                                    })
                                    .on_drop(cx.listener(
                                        |this, paths: &ExternalPaths, window, cx| {
                                            if let Some(path) = paths.paths().first() {
                                                this.apply_dropped_path(path, window, cx);
                                            }
                                        },
                                    ))
                                    .on_drop(cx.listener(
                                        |this, drag: &CompositionDrag, window, cx| {
                                            if let Some(path) = drag.path.as_ref() {
                                                this.apply_dropped_path(path, window, cx);
                                            }
                                        },
                                    ))
                                    .child(self.directory.clone()),
                            )
                            .child(
                                div().w(ACTION_BUTTON_WIDTH).flex_none().child(
                                    Button::new("browse-dir")
                                        .outline()
                                        .xsmall()
                                        .w_full()
                                        .label("Browse…")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.prompt_directory(window, cx);
                                        })),
                                ),
                            ),
                    ))
                    .child(form_row("Name", muted, None, self.filename.clone()))
                    .child(form_row(
                        "Resolved",
                        muted,
                        None,
                        div()
                            .w_full()
                            .text_xs()
                            .text_color(muted)
                            .child(resolved_preview),
                    )),
            ))
            .child(
                v_flex()
                    .gap_1()
                    .w_full()
                    .child(
                        h_flex()
                            .id("export-variables-toggle")
                            .gap_1()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_variables(cx);
                            }))
                            .child(div().text_xs().font_semibold().child("Variables"))
                            .child(
                                Icon::new(if variables_open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall(),
                            ),
                    )
                    .when(variables_open, |el| {
                        el.child(
                            v_flex()
                                .gap_1()
                                .w_full()
                                .child(filter_bar)
                                .child(variables_table),
                        )
                    }),
            )
    }
}

fn section(title: &'static str, child: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().font_semibold().child(title))
        .child(child)
}

fn form_row(
    label: &'static str,
    muted: Hsla,
    value_width: Option<gpui_kit::Rems>,
    control: impl IntoElement,
) -> impl IntoElement {
    let row = h_flex().gap_2().items_center().child(
        div()
            .w(LABEL_WIDTH)
            .flex_none()
            .flex()
            .justify_end()
            .child(div().text_xs().text_color(muted).child(label)),
    );
    match value_width {
        Some(width) => row.child(div().w(width).flex_none().child(control)),
        None => row.w_full().child(div().flex_1().min_w_0().child(control)),
    }
}

fn dropdown(
    id: &'static str,
    value: String,
    disabled: bool,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    Button::new(id)
        .outline()
        .xsmall()
        .w_full()
        .label(value)
        .disabled(disabled)
        .dropdown_menu(menu)
}

fn rate_choices(current: u32) -> Vec<u32> {
    let mut rates = RATE_PRESETS.to_vec();
    if current > 0 && !rates.contains(&current) {
        rates.push(current);
        rates.sort_unstable();
    }
    rates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_format_leaves_match_sheet_controls() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new("export", "title", "from-profile"));
        upsert_format_export_vars(&mut table, "wav", Some(PcmFormat::S24), 48_000, 2);
        assert_eq!(
            table.get_by_name("encoder").map(|e| e.value.as_str()),
            Some("wav")
        );
        assert_eq!(
            table.get_by_name("sample_format").map(|e| e.value.as_str()),
            Some("S24")
        );
        assert_eq!(
            table.get_by_name("sample_rate").map(|e| e.value.as_str()),
            Some("48000")
        );
        assert_eq!(
            table.get_by_name("channels").map(|e| e.value.as_str()),
            Some("2")
        );
        assert_eq!(
            table.get_by_name("extension").map(|e| e.value.as_str()),
            Some("wav")
        );
        assert_eq!(
            table.get_by_name("title").map(|e| e.value.as_str()),
            Some("from-profile")
        );
    }

    #[test]
    fn soft_resolved_path_interpolates_templates() {
        let mut composed = VariableTable::new();
        composed.upsert(VariableEntry::new("source", "basename", "take01"));
        composed.upsert(VariableEntry::new("export", "extension", "flac"));
        let path = soft_resolved_path(
            "/out/${source.basename}",
            "${source.basename}.${export.extension}",
            &composed,
        );
        let expected = PathBuf::from("/out/take01")
            .join("take01.flac")
            .to_string_lossy()
            .into_owned();
        assert_eq!(path, expected);
    }
}
