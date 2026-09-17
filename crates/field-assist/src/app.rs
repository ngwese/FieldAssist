// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use cpal::Device;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    dock::{
        panel_handle, DockArea, DockEvent, DockLayout, DockPlacement, InsertTarget, NodeId,
        PaneRef, PanelId, PanelStyle,
    },
    h_flex, v_flex, ActiveTheme as _, Disableable as _, GlobalState, IconName, Root,
    Selectable as _, Sizable as _, StyledExt as _, Theme, ThemeMode, TitleBar, WindowExt as _,
};
use gpui_kit::{
    div, hsla, img, point, prelude::FluentBuilder as _, px, rems, size, AnyWindowHandle, App,
    AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, Global,
    InteractiveElement as _, IntoElement, KeyContext, Menu, MenuItem, ParentElement as _,
    PathPromptOptions, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowId, WindowOptions,
};

use crate::assets::AppAssets;
use crate::commands::{
    install_keybindings, About, AddMarker, AddMarkerAtHover, AnalyzeEnvelopePeak,
    AnalyzeSelectionOnly, AnalyzeTransients, CancelWorkflow, Close, DeleteMarker, EditBreakOut,
    EditClear, EditCopy, EditCut, EditDuplicate, EditPaste, EditRedo, EditRemove, EditTrim,
    EditUndo, InvertSelection, MarkerTypeBlue, MarkerTypePurple, MarkerTypeYellow, Open, Quit,
    Render as RenderFile, Save, SaveAs, SaveSession, SaveSessionAs, SelectAll, SelectNone,
    SetActiveMarkerType, Settings, SnapToMarker, StartWorkflow, ToggleSnapMarkerType, TransportEnd,
    TransportHome, TransportLoop, TransportNext, TransportPlayPause, TransportPreview,
    TransportPrevious, TransportStart, TransportStop, ViewDetail, ViewExplorer, ViewFitAll,
    ViewFrame, ViewHideDetail, ViewHideExplorer, ViewHideScript, ViewOverlayEnvelopePeak,
    ViewScript, ViewShowDetail, ViewShowExplorer, ViewShowScript, ViewWaveformPeaks,
    ViewWaveformPeaksSpectrum, ViewWaveformSpectrum, ViewZoomIn, ViewZoomOut,
};
use crate::components::about::AboutView;
use crate::components::empty_pane::EmptyPane;
use crate::components::explorer::{ExplorerEvent, ExplorerPanel, InfoMediaRow};
use crate::components::header_meta::HeaderMeta;
use crate::components::quit_unsaved::{QuitUnsavedAction, QuitUnsavedList};
use crate::components::render_sheet::RenderSheet;
use crate::components::status_bar::file_status_from_composition;
use crate::components::workflow_bar::WorkflowBar;
use crate::components::workspace::WorkspacePanel;
use crate::dock_titles::{
    DETAIL_DOCK_TAB_TITLES, DETAIL_TAB_HISTORY, DETAIL_TAB_MARKER, DETAIL_TAB_MONITOR,
    DETAIL_TAB_REGIONS, EXPLORER_DOCK_TAB_TITLES,
};
use crate::lineage::{ExplorerLineageFlags, LineageNode, LineageTree};
use crate::model::composition::{
    default_marker_type, AnalysisBlockOutcome, AnalysisKind, Composition, EditId,
    DEFAULT_MARKER_TYPES, MARKER_TYPE_BLUE, MARKER_TYPE_PURPLE, MARKER_TYPE_YELLOW,
};
use crate::model::{
    is_facomp_path, is_fasession_path, Buffer, BufferDocument, ChannelScope, DocumentId, MarkerId,
    RegionId, Session, SessionDocksUi, SessionUi, SessionWindowUi,
};
use crate::monitor::MonitorChain;
use crate::monitor_schema::{collect_param_addresses, param_ui_layout_from_json};
use crate::playback::{
    list_output_devices, output_device_name, resolve_output_device, should_refresh_monitor_ui,
    PlaybackFaultFlusher, PlaybackFaultLevel, PlaybackSession, TransportState,
};
use crate::progress::ProgressState;
use crate::script::{
    DropLayout, EvalOutput, LogEntry, LogLevel as ScriptLogLevel, ResumeWorkflow, ScriptHost,
    ToolbarItem,
};
use field_ui_components::{
    content_foreground, tool_dock_min_size, AppMenuBar, CenterTabBarHandler, ChainChoice,
    CompactDockSkin, ContentForeground, EditsPanel, LayoutPicker, LogLevel, LogLine, MarkersPanel,
    MessagesPanel, MonitorCallbacks, MonitorPanel, MonitorSnapshot, RegionsPanel, ReplOutput,
    ReplPanel, SessionStatusBar, ToggleZeroCrossing, WaveformDisplay,
};

struct OpenTarget(Entity<AppView>);

impl Global for OpenTarget {}

/// Launch-time state kept for recreating the editor after the last window closes
/// (macOS QuitMode::Default).
struct AppLaunchState {
    pending_opens: Arc<Mutex<Vec<PathBuf>>>,
    output_device_spec: Option<String>,
    /// Set after the deferred first-window open finishes. Until then, Dock /
    /// Finder `on_reopen` must not create an empty editor (it races document
    /// opens at cold launch and drops the CLI preload seed).
    launch_ui_ready: Arc<std::sync::atomic::AtomicBool>,
}

impl Global for AppLaunchState {}

#[derive(Clone, Copy)]
struct EditorWindow(AnyWindowHandle);

impl Global for EditorWindow {}

#[derive(Clone, Copy)]
struct AboutWindow(AnyWindowHandle);

impl Global for AboutWindow {}

enum MainWindowSeed {
    Empty,
    Preloaded {
        composition: Composition,
        load_elapsed: Option<f64>,
    },
    Session(PathBuf),
}

fn to_repl_output(output: &EvalOutput) -> ReplOutput {
    ReplOutput {
        prints: output.prints.clone(),
        result: output.result.clone(),
        error: output.error.clone(),
    }
}

fn to_log_lines(entries: Vec<LogEntry>) -> Vec<LogLine> {
    entries
        .into_iter()
        .map(|entry| LogLine {
            level: match entry.level {
                ScriptLogLevel::Info => LogLevel::Info,
                ScriptLogLevel::Warn => LogLevel::Warn,
                ScriptLogLevel::Error => LogLevel::Error,
            },
            topic: entry.topic,
            text: entry.message,
        })
        .collect()
}

#[derive(Clone)]
struct DocumentViews {
    composition: Arc<RwLock<Composition>>,
    buffer: Arc<RwLock<Buffer>>,
    document: Entity<BufferDocument>,
    waveform: Entity<WaveformDisplay<BufferDocument>>,
    workspace: Entity<WorkspacePanel>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterWrite {
    None,
    Close,
    ContinuePending,
    Replace,
}

#[derive(Clone)]
enum PendingContinue {
    Quit,
    LoadSession(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterSessionWrite {
    None,
    Continue,
}

pub struct AppView {
    session: Session,
    views: HashMap<DocumentId, DocumentViews>,
    dock_area: Entity<DockArea>,
    explorer: Entity<ExplorerPanel>,
    edits: Entity<EditsPanel<BufferDocument>>,
    markers: Entity<MarkersPanel<BufferDocument>>,
    regions: Entity<RegionsPanel<BufferDocument>>,
    monitor: Entity<MonitorPanel>,
    header_meta: Entity<HeaderMeta>,
    empty_editors: Entity<EmptyPane>,
    repl: Entity<ReplPanel>,
    messages: Entity<MessagesPanel>,
    script: ScriptHost,
    idle_composition: Arc<RwLock<Composition>>,
    playback: PlaybackSession,
    playback_faults: PlaybackFaultFlusher,
    app_menu_bar: Option<Entity<AppMenuBar>>,
    pending_opens: Arc<Mutex<Vec<PathBuf>>>,
    pending_load: Arc<
        Mutex<
            Vec<(
                DocumentId,
                u64,
                f64,
                Result<(Composition, Vec<String>), String>,
            )>,
        >,
    >,
    pending_render: Arc<Mutex<Vec<(DocumentId, u64, Result<(), String>)>>>,
    pending_analysis: Arc<Mutex<Vec<DocumentId>>>,
    /// Info lines produced by analysis workers (drained onto the Messages panel).
    pending_analysis_logs: Arc<Mutex<Vec<LogLine>>>,
    pending_loaded_scripts: Vec<(DocumentId, f64)>,
    render_sheet: Entity<RenderSheet>,
    render_sheet_open: bool,
    quit_save_queue: Vec<DocumentId>,
    pending_continue: Option<PendingContinue>,
    focus_handle: FocusHandle,
    last_progress: Option<ProgressState>,
    script_dock_size: Pixels,
    detail_dock_min_size: Pixels,
    explorer_dock_min_size: Pixels,
    last_waveform_over: bool,
    active_marker_type: String,
    add_marker_at_hover: bool,
    preview_enabled: bool,
    output_device: Option<String>,
    /// Cached CPAL output names for the monitor dropdown (refreshed on demand).
    output_devices_cache: Vec<String>,
    drop_layout: Option<Arc<DropLayout>>,
    pending_replace: Option<(DocumentId, PathBuf)>,
    workflow_bar: Option<(String, Vec<ToolbarItem>)>,
    workflow_bar_view: Entity<WorkflowBar>,
    restoring_session: bool,
    /// View → Peaks / Spectrum / Peaks + Spectrum (app-wide, not per document).
    waveform_representation: field_ui_components::WaveformRepresentation,
}

impl AppView {
    fn new(
        composition: Arc<RwLock<Composition>>,
        buffer: Arc<RwLock<Buffer>>,
        source_path: Option<PathBuf>,
        initial_load_elapsed: Option<f64>,
        playback: PlaybackSession,
        output_device: Option<String>,
        pending_opens: Arc<Mutex<Vec<PathBuf>>>,
        session_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app = cx.weak_entity();
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(33))
                .await;
            let still_alive = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.drain_pending_loaded_scripts(window, cx);
                    this.drain_pending_opens(window, cx);
                    this.drain_pending_load(window, cx);
                    this.drain_pending_render(window, cx);
                    this.drain_pending_analysis(window, cx);
                    this.drain_analysis_requests(cx);
                    this.flush_playback_faults(cx);
                    if let Some(views) = this.active_views() {
                        views.document.update(cx, |doc, cx| {
                            if this.playback.poll(doc) {
                                cx.notify();
                            }
                        });
                        let progress = views.document.read(cx).progress.snapshot();
                        if progress != this.last_progress {
                            this.last_progress = progress;
                            views.waveform.update(cx, |_, cx| cx.notify());
                        }
                        let transport = this.playback.transport_state();
                        views.workspace.update(cx, |workspace, cx| {
                            workspace.sync_transport(transport, this.playback.looping(), cx);
                        });
                        this.header_meta.update(cx, |meta, cx| {
                            meta.set_transport(transport, cx);
                        });
                        // Avoid spinning MonitorPanel (~30 Hz) when idle/hidden;
                        // meters are static once envelopes settle after stop.
                        if should_refresh_monitor_ui(
                            this.monitor_tab_visible(cx),
                            transport == TransportState::Playing,
                            this.playback.input_meters_active(),
                        ) {
                            this.monitor.update(cx, |_, cx| cx.notify());
                        }
                    }
                })
            });
            if !matches!(still_alive, Ok(Ok(()))) {
                break;
            }
        })
        .detach();

        let has_initial = source_path.is_some() || composition.read().unwrap().frames() > 0;
        let idle_composition = if has_initial {
            Arc::new(RwLock::new(Composition::new(44100, 2)))
        } else {
            composition.clone()
        };
        let mut session = Session::new();
        let mut views = HashMap::new();
        let first_workspace = if has_initial {
            let first_id = session.push(source_path);
            let first = Self::make_views(
                first_id,
                composition,
                buffer,
                field_ui_components::WaveformRepresentation::Peaks,
                app.clone(),
                cx,
            );
            let workspace = first.workspace.clone();
            views.insert(first_id, first);
            Some(workspace)
        } else {
            None
        };

        let explorer = cx.new(|cx| ExplorerPanel::new(cx));
        explorer.update(cx, |explorer, _| {
            let app_for_handler = app.clone();
            explorer.set_handler(Rc::new(move |event, window, cx| {
                let _ =
                    app_for_handler.update(cx, |this, cx| this.handle_explorer(event, window, cx));
            }));
            let app_for_info = app.clone();
            explorer.set_info_provider(Rc::new(move |id, cx| {
                app_for_info
                    .upgrade()
                    .map(|app| app.read(cx).info_media_rows(id))
                    .unwrap_or_default()
            }));
        });
        let empty_editors = cx.new(|cx| {
            EmptyPane::new("EmptyEditorsPanel", "No open editors", app.clone(), cx)
                .with_message("Drop an audio file here or use File → Open…")
        });
        let initial_target = session.active().and_then(|id| views.get(&id).cloned());
        let header_meta = cx.new(|cx| HeaderMeta::new(cx));
        let edits = cx.new(|cx| EditsPanel::new(DETAIL_TAB_HISTORY, cx));
        let markers = cx.new(|cx| MarkersPanel::new(DETAIL_TAB_MARKER, cx));
        let regions = cx.new(|cx| RegionsPanel::new(DETAIL_TAB_REGIONS, cx));
        let monitor = cx.new(|cx| MonitorPanel::new(DETAIL_TAB_MONITOR, cx));
        {
            let app_for_snap = app.clone();
            let app_for_cb = app.clone();
            monitor.update(cx, |monitor, cx| {
                let snapshot = Rc::new(move |cx: &App| {
                    app_for_snap
                        .upgrade()
                        .and_then(|app| app.read(cx).monitor_snapshot(cx))
                });
                let callbacks = MonitorCallbacks {
                    set_chain: Rc::new({
                        let app = app_for_cb.clone();
                        move |chain, window, cx| {
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, cx| {
                                    this.set_monitor_chain(chain, window, cx);
                                });
                            }
                        }
                    }),
                    set_param: Rc::new({
                        let app = app_for_cb.clone();
                        move |address, value, cx| {
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, _| {
                                    this.set_monitor_param(address, value);
                                });
                            }
                        }
                    }),
                    toggle_pin: Rc::new({
                        let app = app_for_cb.clone();
                        move |_window, cx| {
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, cx| {
                                    this.toggle_monitor_params_pin(cx);
                                });
                            }
                        }
                    }),
                    set_playback_channel: Rc::new({
                        let app = app_for_cb.clone();
                        move |index, enabled, window, cx| {
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, cx| {
                                    this.set_playback_channel(index, enabled, window, cx);
                                });
                            }
                        }
                    }),
                    select_output: Rc::new({
                        let app = app_for_cb.clone();
                        move |spec, window, cx| {
                            if let Some(app) = app.upgrade() {
                                app.update(cx, |this, cx| {
                                    this.select_output_device(spec, window, cx);
                                });
                            }
                        }
                    }),
                };
                monitor.set_host(snapshot, callbacks, cx);
            });
        }
        if let Some(views) = initial_target {
            Self::bind_list_panels(&edits, &markers, &regions, &views, cx);
            header_meta.update(cx, |meta, cx| {
                meta.set_target(Some(views.document), Some(views.waveform), cx);
            });
        }
        let script = ScriptHost::new().expect("lua runtime");
        let repl = cx.new(|cx| ReplPanel::new("Script", window, cx));
        let messages = cx.new(|cx| MessagesPanel::new(cx));
        let render_sheet = cx.new(|cx| RenderSheet::new(window, cx));
        cx.observe(&render_sheet, |_, _, cx| cx.notify()).detach();
        repl.update(cx, |repl, _| {
            let app = app.clone();
            repl.set_handler(Rc::new(move |code, window, cx| {
                let _ = app.update(cx, |this, cx| this.eval_lua(&code, window, cx));
            }));
        });
        let (dock_area, skin) = CompactDockSkin::dock_area("main-dock", Some(1), window, cx);
        let explorer_handle = panel_handle(explorer.clone());
        let center_handle = match first_workspace {
            Some(workspace) => panel_handle(workspace),
            None => panel_handle(empty_editors.clone()),
        };
        let edits_handle = panel_handle(edits.clone());
        let markers_handle = panel_handle(markers.clone());
        let regions_handle = panel_handle(regions.clone());
        let monitor_handle = panel_handle(monitor.clone());
        let detail_dock_min_size = tool_dock_min_size(DETAIL_DOCK_TAB_TITLES, window, cx);
        let detail_dock_size = detail_dock_min_size.max(px(260.));
        let explorer_dock_min_size = tool_dock_min_size(EXPLORER_DOCK_TAB_TITLES, window, cx);
        let explorer_dock_size = explorer_dock_min_size.max(px(220.));
        dock_area.update(cx, |area, cx| {
            area.set_center(DockLayout::tabs().panel_view(center_handle, cx), window, cx);
            area.set_dock(
                DockPlacement::Left,
                DockLayout::tabs().panel_view(explorer_handle, cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Left, explorer_dock_size, window, cx);
            area.set_dock_collapsible(DockPlacement::Left, true, window, cx);
            area.toggle_dock(DockPlacement::Left, window, cx);
            area.set_dock(
                DockPlacement::Right,
                DockLayout::tabs()
                    .panel_view(markers_handle, cx)
                    .panel_view(regions_handle, cx)
                    .panel_view(edits_handle, cx)
                    .panel_view(monitor_handle, cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Right, detail_dock_size, window, cx);
            area.set_dock_collapsible(DockPlacement::Right, true, window, cx);
            area.toggle_dock(DockPlacement::Right, window, cx);
        });
        skin.set_panel_style(PanelStyle::TabBar, cx);
        skin.set_toggle_button_visible(false, cx);
        // Clamp live while the splitter is dragged. gpui-component only floors
        // at PANEL_MIN_SIZE; without this the pane can shrink below our tab
        // floor and snap back on mouse-up.
        cx.observe_in(&dock_area, window, |this, _, window, cx| {
            this.enforce_tool_dock_min_sizes(window, cx);
        })
        .detach();
        cx.subscribe_in(
            &dock_area,
            window,
            |this, _, event: &DockEvent, window, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.enforce_tool_dock_min_sizes(window, cx);
                    this.sync_tabs_from_layout(window, cx);
                }
            },
        )
        .detach();

        let workflow_bar_view = cx.new(|_| WorkflowBar::new(app.clone()));
        let mut this = Self {
            session,
            views,
            dock_area,
            explorer,
            edits,
            markers,
            regions,
            monitor,
            header_meta,
            empty_editors,
            repl,
            messages,
            script,
            idle_composition,
            playback,
            playback_faults: PlaybackFaultFlusher::default(),
            app_menu_bar: (!cfg!(target_os = "macos")).then(|| AppMenuBar::new(cx)),
            pending_opens,
            pending_load: Arc::new(Mutex::new(Vec::new())),
            pending_render: Arc::new(Mutex::new(Vec::new())),
            pending_analysis: Arc::new(Mutex::new(Vec::new())),
            pending_analysis_logs: Arc::new(Mutex::new(Vec::new())),
            pending_loaded_scripts: Vec::new(),
            render_sheet,
            render_sheet_open: false,
            quit_save_queue: Vec::new(),
            pending_continue: None,
            focus_handle: cx.focus_handle(),
            last_progress: None,
            script_dock_size: px(160.),
            detail_dock_min_size,
            explorer_dock_min_size,
            last_waveform_over: false,
            active_marker_type: default_marker_type().to_string(),
            add_marker_at_hover: true,
            preview_enabled: false,
            output_device,
            output_devices_cache: Vec::new(),
            drop_layout: None,
            pending_replace: None,
            workflow_bar: None,
            workflow_bar_view,
            restoring_session: false,
            waveform_representation: field_ui_components::WaveformRepresentation::Peaks,
        };
        this.load_init_lua(window, cx);
        this.refresh_output_devices_cache();
        if let Some(fault) = this.playback.output_fault().map(str::to_string) {
            this.messages.update(cx, |panel, cx| {
                panel.append(
                    vec![LogLine {
                        level: LogLevel::Error,
                        topic: "output".into(),
                        text: fault.into(),
                    }],
                    cx,
                );
            });
        }
        if let Some(path) = session_path {
            if let Err(err) = this.replace_session_from_path(&path, window, cx) {
                this.show_load_error(&err, window, cx);
            }
        }
        if let Some(id) = this.session.active() {
            this.fire_document_scripts(id, initial_load_elapsed.unwrap_or(0.0), window, cx);
        }
        this.refresh_explorer(cx);
        if let Some(id) = this.session.active() {
            this.request_waveform_analysis(id, cx);
        }
        this
    }

    fn make_views(
        id: DocumentId,
        composition: Arc<RwLock<Composition>>,
        buffer: Arc<RwLock<Buffer>>,
        waveform_representation: field_ui_components::WaveformRepresentation,
        app: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> DocumentViews {
        let document = cx.new(|_| {
            let mut doc = BufferDocument::with_shared(composition.clone(), buffer.clone());
            doc.waveform_representation = waveform_representation;
            doc
        });
        cx.observe(&document, move |this, entity, cx| {
            if this.session.active() == Some(id) {
                entity.update(cx, |doc, _| {
                    this.playback.sync_from_document(doc);
                });
                // Skip while an analysis/render job owns progress — re-entering
                // overview analysis from paint notifies raced Transient Selection
                // Only jobs.
                if entity.read(cx).progress.snapshot().is_none() {
                    this.request_waveform_analysis(id, cx);
                }
            }
        })
        .detach();
        let waveform = cx.new(|cx| WaveformDisplay::new(document.clone(), cx));
        cx.observe(&waveform, |this, waveform, cx| {
            let over = waveform.read(cx).pointer_over();
            if this.last_waveform_over == over {
                return;
            }
            this.last_waveform_over = over;
            cx.notify();
        })
        .detach();
        let workspace = cx
            .new(|cx| WorkspacePanel::new(id, document.clone(), waveform.clone(), app.clone(), cx));
        workspace.update(cx, |workspace, _| {
            workspace.set_on_activated(Rc::new(move |id, window, cx| {
                let _ = app.update(cx, |this, cx| {
                    if this.restoring_session {
                        return;
                    }
                    this.focus_document(id, window, cx);
                });
            }));
        });
        DocumentViews {
            composition,
            buffer,
            document,
            waveform,
            workspace,
        }
    }

    fn active_views(&self) -> Option<DocumentViews> {
        let id = self.session.active()?;
        self.views.get(&id).cloned()
    }

    fn composition_title(composition: &Composition) -> SharedString {
        composition.display_name().into()
    }

    fn display_title(&self, id: DocumentId, _cx: &App) -> SharedString {
        if let Some(views) = self.views.get(&id) {
            let composition = views.composition.read().unwrap();
            if composition.display_title().is_some() {
                return composition.display_name().into();
            }
        }
        if let Some(path) = self
            .session
            .get(id)
            .and_then(|doc| doc.project_path.as_ref().or(doc.source_path.as_ref()))
        {
            if let Some(name) = path.file_name() {
                let name = name.to_string_lossy();
                if !name.is_empty() {
                    return name.into_owned().into();
                }
            }
        }
        self.views
            .get(&id)
            .map(|views| Self::composition_title(&views.composition.read().unwrap()))
            .unwrap_or_else(|| crate::APP_NAME.into())
    }

    /// Directory + display-title basename when the composition was renamed in
    /// memory; otherwise the existing project path.
    fn save_path_for_document(&self, id: DocumentId) -> Option<PathBuf> {
        let doc = self.session.get(id)?;
        let title = self.views.get(&id).and_then(|views| {
            views
                .composition
                .read()
                .ok()?
                .display_title()
                .map(str::to_string)
        });
        if let Some(title) = title {
            let dir = doc
                .project_path
                .as_ref()
                .or(doc.source_path.as_ref())
                .and_then(|path| path.parent())?;
            return Some(dir.join(field_composition::normalize_facomp_file_name(&title)));
        }
        doc.project_path.clone()
    }

    pub(crate) fn refresh_explorer(&self, cx: &mut Context<Self>) {
        let tree = self.lineage_tree(cx);
        let ordered: Vec<DocumentId> = self.session.documents().iter().map(|d| d.id).collect();
        let mut by_group: HashMap<Option<String>, Vec<DocumentId>> = HashMap::new();
        for doc in self.session.documents() {
            by_group.entry(doc.group.clone()).or_default().push(doc.id);
        }
        let mut flags: HashMap<DocumentId, ExplorerLineageFlags> = HashMap::new();
        for ids in by_group.values() {
            flags.extend(tree.explorer_flags(ids));
        }
        let by_id: HashMap<DocumentId, _> = self
            .session
            .documents()
            .iter()
            .map(|doc| (doc.id, doc))
            .collect();
        let docs: Vec<_> = ordered
            .into_iter()
            .filter_map(|id| {
                let doc = by_id.get(&id)?;
                let modified = self
                    .views
                    .get(&doc.id)
                    .is_some_and(|views| views.composition.read().unwrap().is_modified());
                let row = flags.get(&doc.id).copied().unwrap_or(ExplorerLineageFlags {
                    depth: 0,
                    parent: None,
                    detached: false,
                    has_children: false,
                });
                Some((
                    doc.id,
                    self.display_title(doc.id, cx),
                    modified,
                    doc.group.clone(),
                    doc.file_path().map(PathBuf::from),
                    row.depth,
                    row.parent,
                    row.detached,
                    row.has_children,
                ))
            })
            .collect();
        let active = self.session.active();
        self.explorer.update(cx, |explorer, cx| {
            explorer.set_documents(&docs, self.session.groups(), active, cx);
        });
    }

    fn info_media_rows(&self, id: DocumentId) -> Vec<InfoMediaRow> {
        let Some(views) = self.views.get(&id) else {
            return Vec::new();
        };
        let composition = views.composition.read().unwrap();
        let mut media: Vec<_> = composition.pool().iter().collect();
        media.sort_by_key(|m| m.id.0);
        media
            .into_iter()
            .map(|m| {
                let path = if !m.path.as_os_str().is_empty() {
                    m.path.display().to_string()
                } else {
                    m.url.clone()
                };
                let duration_secs = if m.sample_rate > 0 {
                    m.frame_count as f64 / f64::from(m.sample_rate)
                } else {
                    0.0
                };
                InfoMediaRow {
                    path,
                    sample_rate: m.sample_rate,
                    channel_count: m.channel_count,
                    duration_secs,
                    size_bytes: m.size_bytes,
                }
            })
            .collect()
    }

    fn lineage_tree(&self, _cx: &App) -> LineageTree {
        let nodes: Vec<LineageNode> = self
            .session
            .documents()
            .iter()
            .filter_map(|doc| {
                let views = self.views.get(&doc.id)?;
                let composition = views.composition.read().unwrap();
                Some(LineageNode {
                    document: doc.id,
                    composition: composition.id(),
                    parent: composition.parent_id(),
                })
            })
            .collect();
        LineageTree::from_nodes(&nodes)
    }

    /// Share decode caches with open parents after a load or break-out.
    fn adopt_shared_media_from_lineage(&mut self, _cx: &App) {
        let tree = self.lineage_tree(_cx);
        let ids: Vec<DocumentId> = self.session.documents().iter().map(|d| d.id).collect();
        for id in ids {
            let Some(parent_id) = tree.parent(id) else {
                continue;
            };
            let Some(parent_views) = self.views.get(&parent_id).cloned() else {
                continue;
            };
            let Some(child_views) = self.views.get(&id).cloned() else {
                continue;
            };
            let parent = parent_views.composition.read().unwrap();
            child_views
                .composition
                .write()
                .unwrap()
                .adopt_shared_media(&parent);
        }
    }

    fn break_out_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent_id) = self.session.active() else {
            return;
        };
        let Some(parent_views) = self.views.get(&parent_id).cloned() else {
            return;
        };
        let spans = parent_views.document.read(cx).selection_spans();
        let spans: Vec<(u64, u64)> = spans.into_iter().filter(|(_, len)| *len > 0).collect();
        if spans.is_empty() {
            return;
        }
        let parent_group = self
            .session
            .get(parent_id)
            .and_then(|doc| doc.group.clone());
        let parent_name = self.display_title(parent_id, cx).to_string();
        let tree = self.lineage_tree(cx);
        let ordered: Vec<DocumentId> = self.session.documents().iter().map(|d| d.id).collect();
        let mut sibling_names: Vec<String> = tree
            .children(parent_id, &ordered)
            .iter()
            .map(|id| self.display_title(*id, cx).to_string())
            .collect();
        let mut children = Vec::new();
        {
            let parent = parent_views.composition.read().unwrap();
            for &(start, len) in &spans {
                match parent.break_out(start, len) {
                    Ok(mut child) => {
                        let name = crate::break_out_name::next_break_out_name(
                            &parent_name,
                            &sibling_names,
                        );
                        child.set_display_title(&name);
                        sibling_names.push(name);
                        children.push(child);
                    }
                    Err(err) => {
                        self.show_save_error(&format!("Break out failed: {err:#}"), window, cx);
                        return;
                    }
                }
            }
        }
        let mut last_id = None;
        let mut insert_at = self
            .session
            .documents()
            .iter()
            .filter(|doc| doc.group == parent_group)
            .position(|doc| doc.id == parent_id)
            .map(|i| i + 1)
            .unwrap_or_else(|| {
                self.session
                    .documents()
                    .iter()
                    .filter(|doc| doc.group == parent_group)
                    .count()
            });
        for child in children {
            let composition = Arc::new(RwLock::new(child));
            let buffer = Arc::new(RwLock::new(Buffer::empty()));
            let id = self.add_document(composition, buffer, None, window, cx);
            self.session
                .place_document(id, parent_group.clone(), insert_at);
            insert_at += 1;
            self.pin_tab(id, cx);
            last_id = Some(id);
        }
        self.adopt_shared_media_from_lineage(cx);
        if let Some(id) = last_id {
            self.session.focus(id);
            self.apply_active(window, cx);
        }
        self.refresh_explorer(cx);
        cx.notify();
    }

    /// Suggested path for an unsaved break-out child next to its parent.
    fn suggested_break_out_path(&self, id: DocumentId, cx: &App) -> Option<PathBuf> {
        let tree = self.lineage_tree(cx);
        let parent_id = tree.parent(id)?;
        let parent_path = self.session.get(parent_id)?.file_path()?;
        let dir = parent_path.parent()?;
        if let Some(title) = self.views.get(&id).and_then(|views| {
            views
                .composition
                .read()
                .ok()?
                .display_title()
                .map(str::to_string)
        }) {
            return Some(dir.join(field_composition::normalize_facomp_file_name(&title)));
        }
        let parent_name = self.display_title(parent_id, cx).to_string();
        let ordered: Vec<DocumentId> = self.session.documents().iter().map(|d| d.id).collect();
        let sibling_names: Vec<String> = tree
            .children(parent_id, &ordered)
            .iter()
            .filter(|sid| **sid != id)
            .map(|sid| self.display_title(*sid, cx).to_string())
            .collect();
        let name = crate::break_out_name::next_break_out_name(&parent_name, &sibling_names);
        Some(dir.join(field_composition::normalize_facomp_file_name(&name)))
    }

    /// Write untitled compositions to suggested paths so session save can
    /// record file URLs (break-out children first).
    fn ensure_untitled_documents_saved(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let untitled: Vec<DocumentId> = self
            .session
            .documents()
            .iter()
            .filter(|doc| doc.file_path().is_none())
            .map(|doc| doc.id)
            .collect();
        for id in &untitled {
            let path = self
                .suggested_break_out_path(*id, cx)
                .or_else(|| {
                    let dir = self.suggested_save_directory(*id, cx);
                    let name = self
                        .views
                        .get(id)
                        .map(|views| views.composition.read().unwrap().suggested_facomp_name())
                        .unwrap_or_else(|| "untitled.facomp".into());
                    Some(dir.join(name))
                })
                .ok_or_else(|| "Cannot save untitled composition without a path.".to_string())?;
            let Some(views) = self.views.get(id) else {
                return Err("Composition is not open.".into());
            };
            views
                .composition
                .write()
                .unwrap()
                .save_to_path(&path)
                .map_err(|err| format!("{err:#}"))?;
            self.session.set_project_path(*id, path);
        }
        if !untitled.is_empty() {
            self.refresh_explorer(cx);
            self.update_window_title(window, cx);
        }
        Ok(())
    }

    fn update_window_title(&self, window: &mut Window, cx: &App) {
        let title = self
            .session
            .active()
            .map(|id| self.display_title(id, cx))
            .unwrap_or_else(|| crate::APP_NAME.into());
        window.set_window_title(&title);
    }

    fn center_panel_ids(&self, cx: &App) -> HashSet<u64> {
        self.dock_area
            .read(cx)
            .layout(DockPlacement::Center)
            .map(|tree| tree.panels().map(|id| id.as_u64()).collect())
            .unwrap_or_default()
    }

    fn empty_editors_id(&self) -> u64 {
        PanelId::from(self.empty_editors.entity_id()).as_u64()
    }

    fn ensure_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.center_panel_ids(cx).contains(&self.empty_editors_id()) {
            return;
        }
        let panel = self.empty_editors.clone();
        self.dock_area.update(cx, |area, cx| {
            area.add_panel(panel, DockPlacement::Center, None, window, cx);
        });
    }

    fn remove_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.tab_open_count() == 0 {
            return;
        }
        if !self.center_panel_ids(cx).contains(&self.empty_editors_id()) {
            return;
        }
        let panel = self.empty_editors.clone();
        self.dock_area.update(cx, |area, cx| {
            area.remove_panel(panel, window, cx);
        });
    }

    fn sync_tabs_from_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let center = self.center_panel_ids(cx);
        let ids: Vec<DocumentId> = self.session.documents().iter().map(|doc| doc.id).collect();
        for id in ids {
            let open = self.views.get(&id).is_some_and(|views| {
                center.contains(&PanelId::from(views.workspace.entity_id()).as_u64())
            });
            self.session.set_tab_open(id, open);
        }
        if self.session.tab_open_count() == 0 {
            self.ensure_placeholder(window, cx);
        } else {
            self.remove_placeholder(window, cx);
        }
        self.refresh_explorer(cx);
    }

    fn apply_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drain_pending_load(window, cx);
        self.drain_pending_analysis(window, cx);
        if let Some(views) = self.active_views() {
            self.playback.bind_composition(views.composition.clone());
            views.document.update(cx, |doc, _| {
                self.playback.sync_from_document(doc);
            });
            self.playback
                .load_monitor_for_document(views.document.read(cx));
            Self::bind_list_panels(&self.edits, &self.markers, &self.regions, &views, cx);
            self.monitor.update(cx, |_, cx| cx.notify());
            self.header_meta.update(cx, |meta, cx| {
                meta.set_target(
                    Some(views.document.clone()),
                    Some(views.waveform.clone()),
                    cx,
                );
            });
            if let Some(id) = self.session.active() {
                self.request_waveform_analysis(id, cx);
            }
            views.waveform.update(cx, |view, cx| {
                view.bump_paint_epoch(cx);
            });
        } else {
            let idle = self.idle_composition.clone();
            self.playback.bind_composition(idle.clone());
            self.playback.reload(&Buffer::empty());
            self.playback.load_session_monitor(&idle.read().unwrap());
            self.edits.update(cx, |edits, cx| edits.clear_target(cx));
            self.markers
                .update(cx, |markers, cx| markers.clear_target(cx));
            self.regions
                .update(cx, |regions, cx| regions.clear_target(cx));
            self.monitor.update(cx, |_, cx| cx.notify());
            self.header_meta.update(cx, |meta, cx| {
                meta.set_target(None, None, cx);
            });
        }
        self.refresh_explorer(cx);
        self.update_window_title(window, cx);
        self.sync_view_menus(cx);
        window.refresh();
        cx.notify();
    }

    fn bind_list_panels(
        edits: &Entity<EditsPanel<BufferDocument>>,
        markers: &Entity<MarkersPanel<BufferDocument>>,
        regions: &Entity<RegionsPanel<BufferDocument>>,
        views: &DocumentViews,
        cx: &mut App,
    ) {
        let document = views.document.clone();
        let waveform = views.waveform.clone();
        edits.update(cx, |edits, cx| {
            let document = document.clone();
            let waveform_hover = waveform.clone();
            let waveform_click = waveform.clone();
            edits.set_target(
                document.clone(),
                Rc::new(move |id, _window, cx| {
                    waveform_hover.update(cx, |view, cx| {
                        view.set_hovered_edit(id, cx);
                    });
                }),
                Rc::new(move |id, _window, cx| {
                    waveform_click.update(cx, |view, cx| {
                        view.scroll_edit_into_view(id, cx);
                    });
                }),
                Rc::new(move |id, _window, cx| {
                    document.update(cx, |doc, cx| {
                        doc.jump_to_edit(EditId(id));
                        cx.notify();
                    });
                }),
                cx,
            );
        });
        let document = views.document.clone();
        let waveform = views.waveform.clone();
        markers.update(cx, |markers, cx| {
            let document_select = document.clone();
            let document_delete = document.clone();
            let waveform_select = waveform.clone();
            markers.set_target(
                document.clone(),
                Rc::new(move |_id, frame, _window, cx| {
                    document_select.update(cx, |doc, cx| {
                        doc.set_position(frame as usize, ChannelScope::all());
                        cx.notify();
                    });
                    waveform_select.update(cx, |view, cx| {
                        view.scroll_sample_into_view(frame as f64, cx);
                    });
                }),
                Rc::new(move |id, _window, cx| {
                    document_delete.update(cx, |doc, cx| {
                        doc.remove_marker(MarkerId(id));
                        cx.notify();
                    });
                }),
                cx,
            );
        });
        let document = views.document.clone();
        let waveform = views.waveform.clone();
        regions.update(cx, |regions, cx| {
            let document_select = document.clone();
            let document_delete = document.clone();
            let waveform_select = waveform.clone();
            regions.set_target(
                document.clone(),
                Rc::new(move |_collection, _id, start, _window, cx| {
                    document_select.update(cx, |doc, cx| {
                        doc.set_position(start, ChannelScope::all());
                        cx.notify();
                    });
                    waveform_select.update(cx, |view, cx| {
                        view.scroll_sample_into_view(start as f64, cx);
                    });
                }),
                Rc::new(move |_collection, id, _window, cx| {
                    document_delete.update(cx, |doc, cx| {
                        doc.remove_region(RegionId(id));
                        cx.notify();
                    });
                }),
                cx,
            );
        });
    }

    fn monitor_snapshot(&self, cx: &App) -> Option<MonitorSnapshot> {
        let views = self.active_views()?;
        let doc = views.document.read(cx);
        let composition = doc.composition.read().unwrap();
        let channel_count = composition.channel_count();
        let channel_labels: Vec<String> = (0..channel_count)
            .map(|ch| composition.channel_label(ch))
            .collect();
        let chain_id = composition.monitor_chain().map(str::to_string);
        let playback_channels = composition.playback_channels().map(|ch| ch.to_vec());
        let params_pinned = doc.monitor_params_pinned;
        drop(composition);

        let chain = chain_id.as_deref().and_then(MonitorChain::parse);
        let mut chain_choices = vec![ChainChoice {
            id: None,
            label: "Direct".into(),
        }];
        for chain in MonitorChain::ALL {
            chain_choices.push(ChainChoice {
                id: Some(chain.id().to_string()),
                label: chain.label().to_string(),
            });
        }
        let chain_label = chain
            .map(MonitorChain::label)
            .unwrap_or("Direct")
            .to_string();
        let expected_inputs = chain.map(MonitorChain::num_inputs);

        let ui_json = self.monitor_ui_json();
        let schema_id = ui_json.map(|json| json.as_ptr() as u64).unwrap_or(0);
        let layout = ui_json
            .and_then(param_ui_layout_from_json)
            .unwrap_or_default();
        let mut addresses = collect_param_addresses(&layout.sections);
        addresses.extend(collect_param_addresses(&layout.output_params));
        let live_params: HashMap<String, f32> = addresses
            .into_iter()
            .filter_map(|address| self.monitor_param(&address).map(|value| (address, value)))
            .collect();
        let meters = self.monitor_meters();
        let output_devices = self.output_devices_cache.clone();

        Some(MonitorSnapshot {
            chain_label,
            chain_id,
            chain_choices,
            params_pinned,
            channel_labels,
            playback_channels,
            expected_inputs,
            schema_id,
            params_ui: layout.sections,
            input_meters: layout.input_meters,
            output_meters: layout.output_meters,
            output_params: layout.output_params,
            live_params,
            meters,
            output_device: self.output_device.clone(),
            output_devices,
        })
    }

    fn stop_playback_into_active(&mut self, cx: &mut Context<Self>) {
        if self.playback.transport_state() != TransportState::Playing {
            return;
        }
        self.sync_playback_to_document(cx);
        self.playback.stop();
        if let Some(views) = self.active_views() {
            views.workspace.update(cx, |workspace, cx| {
                workspace.sync_transport(TransportState::Stopped, self.playback.looping(), cx);
            });
        }
    }

    fn commit_monitor_for_id(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Some(views) = self.views.get(&id).cloned() else {
            self.playback.commit_monitor_to_session();
            return;
        };
        views.document.update(cx, |doc, _| {
            self.playback.commit_monitor_for_document(doc);
        });
    }

    fn commit_active_monitor_params(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.session.active() {
            self.commit_monitor_for_id(id, cx);
        } else {
            self.playback.commit_monitor_to_session();
        }
    }

    fn focus_document(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old_id) = self.session.focus(id) {
            self.commit_monitor_for_id(old_id, cx);
            if self.playback.transport_state() == TransportState::Playing {
                if let Some(old) = self.views.get(&old_id).cloned() {
                    old.document.update(cx, |doc, cx| {
                        self.playback.sync_document_from_playback(doc);
                        cx.notify();
                    });
                }
                self.playback.stop();
            }
            if let Some(old) = self.views.get(&old_id) {
                old.workspace.update(cx, |workspace, cx| {
                    workspace.sync_transport(TransportState::Stopped, self.playback.looping(), cx);
                });
            }
        } else {
            self.refresh_explorer(cx);
            self.update_window_title(window, cx);
            return;
        }
        self.apply_active(window, cx);
    }

    fn center_tab_slot(area: &DockArea, panel_id: PanelId) -> Option<(NodeId, usize, usize)> {
        Self::panel_tab_slot(area, DockPlacement::Center, panel_id)
    }

    fn panel_tab_slot(
        area: &DockArea,
        placement: DockPlacement,
        panel_id: PanelId,
    ) -> Option<(NodeId, usize, usize)> {
        let tree = area.layout(placement)?;
        let node = tree.find_panel_node(panel_id)?;
        match tree.find_node(node)?.kind() {
            PaneRef::Tabs { panels, active_ix } => {
                let ix = panels.iter().position(|id| *id == panel_id)?;
                Some((node, ix, active_ix))
            }
            _ => None,
        }
    }

    /// Select `workspace` in the center tab bar, or add it once with a titled
    /// panel handle. Never inserts a second copy of the same panel.
    fn show_workspace_tab(
        area: &mut DockArea,
        workspace: Entity<WorkspacePanel>,
        insert_ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<DockArea>,
    ) {
        let panel_id = PanelId::from(workspace.entity_id());
        if let Some((node, ix, active_ix)) = Self::center_tab_slot(area, panel_id) {
            if ix != active_ix {
                area.move_panel(
                    panel_id,
                    InsertTarget::Tabs {
                        node,
                        ix: Some(ix),
                        activate: true,
                    },
                    window,
                    cx,
                );
            }
            return;
        }
        area.add_panel_view(
            panel_handle(workspace),
            DockPlacement::Center,
            None,
            window,
            cx,
        );
        if let Some(insert_ix) = insert_ix {
            if let Some((node, current_ix, _)) = Self::center_tab_slot(area, panel_id) {
                if current_ix != insert_ix {
                    area.move_panel(
                        panel_id,
                        InsertTarget::Tabs {
                            node,
                            ix: Some(insert_ix),
                            activate: true,
                        },
                        window,
                        cx,
                    );
                }
            }
        }
    }

    fn document_id_for_panel(&self, panel_id: u64) -> Option<DocumentId> {
        self.views.iter().find_map(|(id, views)| {
            (PanelId::from(views.workspace.entity_id()).as_u64() == panel_id).then_some(*id)
        })
    }

    fn center_tab_document_ids(&self, cx: &App) -> Vec<DocumentId> {
        let empty = self.empty_editors_id();
        self.dock_area
            .read(cx)
            .layout(DockPlacement::Center)
            .map(|tree| {
                tree.panels()
                    .filter(|id| id.as_u64() != empty)
                    .filter_map(|panel_id| self.document_id_for_panel(panel_id.as_u64()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn first_transient_id(&self, cx: &App) -> Option<DocumentId> {
        self.session
            .first_transient_in(&self.center_tab_document_ids(cx))
    }

    fn panel_is_pinned(&self, panel_id: u64) -> bool {
        self.document_id_for_panel(panel_id)
            .is_some_and(|id| self.session.tab_pinned(id))
    }

    fn pin_tab(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if self.session.pin_tab(id) {
            self.dock_area.update(cx, |_, cx| cx.notify());
            cx.notify();
        }
    }

    fn pin_center_panel(&mut self, panel_id: u64, cx: &mut Context<Self>) {
        if let Some(id) = self.document_id_for_panel(panel_id) {
            self.pin_tab(id, cx);
        }
    }

    fn ensure_tab(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_tab_at(id, None, window, cx);
    }

    fn ensure_tab_at(
        &mut self,
        id: DocumentId,
        insert_ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.views.get(&id).cloned() else {
            return;
        };
        self.focus_document(id, window, cx);
        let workspace = views.workspace.clone();
        self.dock_area.update(cx, |area, cx| {
            Self::show_workspace_tab(area, workspace, insert_ix, window, cx);
        });
        self.session.ensure_tab(id);
        self.remove_placeholder(window, cx);
    }

    fn activate_or_replace_tab(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.get(id).is_some_and(|doc| doc.tab_open) {
            self.ensure_tab(id, window, cx);
            return;
        }
        if let Some(transient) = self.first_transient_id(cx) {
            self.replace_transient_tab(transient, id, window, cx);
            return;
        }
        self.ensure_tab(id, window, cx);
    }

    fn replace_transient_tab(
        &mut self,
        old_id: DocumentId,
        new_id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let insert_ix = self.views.get(&old_id).and_then(|views| {
            let panel_id = PanelId::from(views.workspace.entity_id());
            Self::center_tab_slot(&self.dock_area.read(cx), panel_id).map(|(_, ix, _)| ix)
        });
        self.close_tab(old_id, window, cx);
        self.ensure_tab_at(new_id, insert_ix, window, cx);
    }

    fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for id in self.session.open_tab_ids() {
            self.close_tab(id, window, cx);
        }
    }

    fn close_saved_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<DocumentId> = self
            .session
            .open_tab_ids()
            .into_iter()
            .filter(|&id| {
                !self
                    .views
                    .get(&id)
                    .is_some_and(|views| views.composition.read().unwrap().is_modified())
            })
            .collect();
        for id in ids {
            self.close_tab(id, window, cx);
        }
    }

    fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_enabled = !self.preview_enabled;
        cx.notify();
    }

    pub(crate) fn preview_enabled(&self) -> bool {
        self.preview_enabled
    }

    pub(crate) fn playback_looping(&self) -> bool {
        self.playback.looping()
    }

    pub(crate) fn script_activate_document(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.session.get(id).is_none() {
            return Err("composition is not open".into());
        }
        self.activate_or_replace_tab(id, window, cx);
        self.apply_preview_if_enabled(cx);
        Ok(())
    }

    fn apply_preview_if_enabled(&mut self, cx: &mut Context<Self>) {
        if !self.preview_enabled {
            return;
        }
        self.preview_from_start(cx);
    }

    fn preview_from_start(&mut self, cx: &mut Context<Self>) {
        let Some(views) = self.active_views() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            doc.set_position_from_playback(0, ChannelScope::all());
            cx.notify();
        });
        views.document.update(cx, |doc, _| {
            self.playback.sync_from_document(doc);
        });
        self.playback.play_from(0);
        views.workspace.update(cx, |workspace, cx| {
            workspace.sync_transport(TransportState::Playing, self.playback.looping(), cx);
        });
        cx.notify();
    }

    fn close_tab(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.session.get(id).is_some_and(|doc| doc.tab_open) {
            return;
        }
        if self.session.tab_open_count() == 1 {
            self.ensure_placeholder(window, cx);
        }
        if let Some(views) = self.views.get(&id) {
            let workspace = views.workspace.clone();
            self.dock_area.update(cx, |area, cx| {
                area.remove_panel(workspace, window, cx);
            });
        }
        self.session.close_tab(id);
        cx.notify();
    }

    fn close_center_panel(&mut self, panel_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if panel_id == self.empty_editors_id() {
            return;
        }
        let Some(id) = self.document_id_for_panel(panel_id) else {
            return;
        };
        self.close_tab(id, window, cx);
    }

    fn close_document(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.active() == Some(id) {
            self.commit_active_monitor_params(cx);
            self.stop_playback_into_active(cx);
        }
        if self.session.get(id).is_some_and(|doc| doc.tab_open) {
            self.close_tab(id, window, cx);
        }
        if let Some(views) = self.views.get(&id) {
            views.document.read(cx).progress.cancel();
        }
        self.session.close_document(id);
        self.views.remove(&id);
        if self.session.is_empty() {
            self.ensure_placeholder(window, cx);
        }
        self.apply_active(window, cx);
        self.refresh_explorer(cx);
        cx.notify();
    }

    fn add_document(
        &mut self,
        composition: Arc<RwLock<Composition>>,
        buffer: Arc<RwLock<Buffer>>,
        source_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DocumentId {
        self.commit_active_monitor_params(cx);
        let app = cx.weak_entity();
        let id = self.session.push(source_path);
        let views = Self::make_views(
            id,
            composition,
            buffer,
            self.waveform_representation,
            app,
            cx,
        );
        let workspace = views.workspace.clone();
        self.views.insert(id, views);
        self.dock_area.update(cx, |area, cx| {
            Self::show_workspace_tab(area, workspace, None, window, cx);
        });
        self.remove_placeholder(window, cx);
        id
    }

    fn handle_explorer(
        &mut self,
        event: ExplorerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ExplorerEvent::Activate(id) => {
                self.activate_or_replace_tab(id, window, cx);
                self.apply_preview_if_enabled(cx);
            }
            ExplorerEvent::OpenTab(id) => {
                self.ensure_tab(id, window, cx);
                self.pin_tab(id, cx);
                self.apply_preview_if_enabled(cx);
            }
            ExplorerEvent::Close(id) => self.request_close_document(id, window, cx),
            ExplorerEvent::Place { id, group, index } => {
                self.session.place_document(id, group, index);
                self.refresh_explorer(cx);
            }
            ExplorerEvent::Reattach(id) => {
                let tree = self.lineage_tree(cx);
                let Some(parent_id) = tree.parent(id) else {
                    return;
                };
                let parent_group = self
                    .session
                    .get(parent_id)
                    .and_then(|doc| doc.group.clone());
                let members: Vec<DocumentId> = self
                    .session
                    .documents()
                    .iter()
                    .filter(|doc| doc.id != id && doc.group == parent_group)
                    .map(|doc| doc.id)
                    .collect();
                let index = members
                    .iter()
                    .position(|&member| member == parent_id)
                    .map(|i| i + 1)
                    .unwrap_or(members.len());
                self.session.place_document(id, parent_group, index);
                self.refresh_explorer(cx);
            }
            ExplorerEvent::Rename { id, name } => {
                if let Some(views) = self.views.get(&id) {
                    views.composition.write().unwrap().set_display_title(name);
                }
                self.refresh_explorer(cx);
                if self.session.active() == Some(id) {
                    self.update_window_title(window, cx);
                }
            }
            ExplorerEvent::RenameGroup { from, to } => {
                self.session.rename_group(&from, to);
                self.refresh_explorer(cx);
            }
            ExplorerEvent::AddGroup { name, after } => {
                self.session.add_group_after(name, after.as_deref());
                self.refresh_explorer(cx);
            }
            ExplorerEvent::DeleteGroup { name } => {
                self.session.delete_group(&name);
                self.refresh_explorer(cx);
            }
            ExplorerEvent::MoveGroup { name, index } => {
                self.session.move_group(&name, index);
                self.refresh_explorer(cx);
            }
        }
    }

    fn push_loaded_composition(
        &mut self,
        id: DocumentId,
        composition: Composition,
        elapsed: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.views.get(&id).cloned() else {
            return;
        };
        {
            *views.composition.write().unwrap() = composition;
        }
        {
            *views.buffer.write().unwrap() = Buffer::empty();
        }
        views.document.read(cx).progress.cancel();
        if self.session.active() == Some(id) {
            self.request_waveform_analysis(id, cx);
        }
        views
            .document
            .update(cx, |doc, _| doc.reset_for_new_buffer());
        views.waveform.update(cx, |view, cx| view.reset_view(cx));
        views.workspace.update(cx, |_, cx| cx.notify());
        if self.session.active() == Some(id) {
            self.playback.bind_composition(views.composition.clone());
            let snapshot = views.buffer.read().unwrap();
            self.playback.reload(&snapshot);
            drop(snapshot);
            views.document.update(cx, |doc, _| {
                self.playback.sync_from_document(doc);
            });
            self.playback
                .load_monitor_for_document(views.document.read(cx));
            self.update_window_title(window, cx);
        }
        self.refresh_explorer(cx);
        self.adopt_shared_media_from_lineage(cx);
        self.pending_loaded_scripts.push((id, elapsed));
        cx.notify();
    }

    fn toggle_detail_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area.update(cx, |area, cx| {
            area.toggle_dock(DockPlacement::Right, window, cx);
        });
        self.sync_view_menus(cx);
        cx.notify();
    }

    fn set_detail_dock(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail_dock_open(cx) != open {
            self.toggle_detail_dock(window, cx);
        }
    }

    /// Keep tool docks at least as wide as their combined tabs.
    /// gpui-component only floors at `PANEL_MIN_SIZE` (100px) during splitter
    /// drags, so we re-clamp on every dock notify (and again on LayoutChanged).
    fn enforce_tool_dock_min_sizes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.enforce_dock_min_size(DockPlacement::Left, self.explorer_dock_min_size, window, cx);
        self.enforce_dock_min_size(DockPlacement::Right, self.detail_dock_min_size, window, cx);
    }

    fn enforce_dock_min_size(
        &mut self,
        placement: DockPlacement,
        min: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(size) = self.dock_area.read(cx).dock_size(placement) else {
            return;
        };
        if size < min {
            self.dock_area.update(cx, |area, cx| {
                area.set_dock_size(placement, min, window, cx);
            });
        }
    }

    fn toggle_explorer_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area.update(cx, |area, cx| {
            area.toggle_dock(DockPlacement::Left, window, cx);
        });
        self.sync_view_menus(cx);
        cx.notify();
    }

    fn set_explorer_dock(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.explorer_dock_open(cx) != open {
            self.toggle_explorer_dock(window, cx);
        }
    }

    fn detail_dock_open(&self, cx: &App) -> bool {
        self.dock_area.read(cx).is_dock_open(DockPlacement::Right)
    }

    fn monitor_tab_visible(&self, cx: &App) -> bool {
        if !self.detail_dock_open(cx) {
            return false;
        }
        let panel_id = PanelId::from(self.monitor.entity_id());
        Self::panel_tab_slot(&self.dock_area.read(cx), DockPlacement::Right, panel_id)
            .is_some_and(|(_, ix, active_ix)| ix == active_ix)
    }

    pub(crate) fn explorer_dock_open(&self, cx: &App) -> bool {
        self.dock_area.read(cx).is_dock_open(DockPlacement::Left)
    }

    fn toggle_script_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.script_dock_open(cx) {
            self.hide_script_dock(window, cx);
        } else {
            self.show_script_dock(window, cx);
        }
    }

    fn script_dock_open(&self, cx: &App) -> bool {
        self.dock_area.read(cx).is_dock_open(DockPlacement::Bottom)
    }

    fn sync_view_menus(&self, cx: &mut Context<Self>) {
        apply_app_menus(&self.app_menu_state(cx), cx);
        if let Some(bar) = self.app_menu_bar.clone() {
            bar.update(cx, |bar, cx| bar.reload(cx));
        }
    }

    fn app_menu_state(&self, cx: &App) -> AppMenuState {
        let (snap_to_marker, marker_types, snap_disabled, envelope_overlay, analyze_selection_only) =
            if let Some(views) = self.active_views() {
                let doc = views.document.read(cx);
                (
                    doc.snap_to_marker,
                    doc.marker_types().into_iter().map(|ty| ty.name).collect(),
                    doc.snap_marker_disabled.clone(),
                    doc.show_envelope_peak,
                    doc.analyze_selection_only,
                )
            } else {
                (
                    false,
                    DEFAULT_MARKER_TYPES
                        .iter()
                        .map(|(name, _)| (*name).to_string())
                        .collect(),
                    HashSet::new(),
                    false,
                    false,
                )
            };
        AppMenuState {
            editor_open: true,
            explorer: self.explorer_dock_open(cx),
            detail: self.detail_dock_open(cx),
            script: self.script_dock_open(cx),
            envelope_overlay,
            waveform_representation: self.waveform_representation,
            analyze_selection_only,
            marker_type: self.active_marker_type.clone(),
            add_at_hover: self.add_marker_at_hover,
            snap_to_marker,
            marker_types,
            snap_disabled,
            workflow_running: self.script.active_workflow_name().is_some(),
            menu_workflows: self.script.menu_workflows(),
        }
    }

    fn show_script_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opened = !self.script_dock_open(cx);
        if opened {
            let script_handle = panel_handle(self.repl.clone());
            let messages_handle = panel_handle(self.messages.clone());
            let size = self.script_dock_size;
            self.dock_area.update(cx, |area, cx| {
                area.set_dock(
                    DockPlacement::Bottom,
                    DockLayout::tabs()
                        .panel_view(script_handle, cx)
                        .panel_view(messages_handle, cx),
                    window,
                    cx,
                );
                area.set_dock_size(DockPlacement::Bottom, size, window, cx);
                area.set_dock_collapsible(DockPlacement::Bottom, false, window, cx);
            });
        }
        self.repl.focus_handle(cx).focus(window, cx);
        if opened {
            self.sync_view_menus(cx);
            cx.notify();
        }
    }

    fn hide_script_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.script_dock_open(cx) {
            return;
        }
        if let Some(size) = self.dock_area.read(cx).dock_size(DockPlacement::Bottom) {
            self.script_dock_size = size;
        }
        self.dock_area.update(cx, |area, cx| {
            area.remove_dock(DockPlacement::Bottom, window, cx);
        });
        self.sync_view_menus(cx);
        cx.notify();
    }

    fn load_init_lua(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.load_init() {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&err, cx);
            });
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
    }

    fn eval_lua(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = crate::script::enter(self, window, cx);
        let output = self.script.eval(code);
        self.repl.update(cx, |repl, cx| {
            repl.append_eval(code, &to_repl_output(&output), cx);
        });
        self.flush_script_logs(cx);
    }

    fn fire_document_scripts(
        &mut self,
        id: DocumentId,
        elapsed: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        self.script.fire_detect_layout(id);
        self.script.fire_loaded(id, elapsed);
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        if let Some(views) = self.views.get(&id).cloned() {
            views.document.update(cx, |_, cx| cx.notify());
            views.waveform.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    fn fire_saved_script(
        &mut self,
        id: DocumentId,
        elapsed: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        self.script.fire_saved(id, elapsed);
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
    }

    fn flush_script_logs(&mut self, cx: &mut Context<Self>) {
        let logs = self.script.take_logs();
        if !logs.is_empty() {
            self.messages.update(cx, |panel, cx| {
                panel.append(to_log_lines(logs), cx);
            });
            cx.notify();
        }
        self.refresh_workflow_bar(cx);
        self.sync_view_menus(cx);
    }

    fn flush_playback_faults(&mut self, cx: &mut Context<Self>) {
        if let Some(faults) = self.playback.take_faults() {
            self.playback_faults.push(faults);
        }
        let messages = self.playback_faults.poll(Instant::now());
        if messages.is_empty() {
            return;
        }
        let lines: Vec<LogLine> = messages
            .into_iter()
            .map(|entry| LogLine {
                level: match entry.level {
                    PlaybackFaultLevel::Warn => LogLevel::Warn,
                    PlaybackFaultLevel::Error => LogLevel::Error,
                },
                topic: "playback".into(),
                text: entry.text,
            })
            .collect();
        self.messages.update(cx, |panel, cx| {
            panel.append(lines, cx);
        });
        cx.notify();
    }

    pub(crate) fn refresh_workflow_bar(&mut self, cx: &mut Context<Self>) {
        let next = self.script.toolbar_snapshot();
        if self.workflow_bar != next {
            self.workflow_bar = next.clone();
            self.workflow_bar_view.update(cx, |bar, cx| {
                bar.set_snapshot(next, cx);
            });
            cx.notify();
        }
    }

    fn messages_tab_visible(&self, cx: &App) -> bool {
        if !self.script_dock_open(cx) {
            return false;
        }
        let area = self.dock_area.read(cx);
        let Some(tree) = area.layout(DockPlacement::Bottom) else {
            return false;
        };
        let panel_id = PanelId::from(self.messages.entity_id());
        let Some(node) = tree.find_panel_node(panel_id) else {
            return false;
        };
        match tree.find_node(node).map(|node| node.kind()) {
            Some(PaneRef::Tabs { panels, active_ix }) => panels.get(active_ix) == Some(&panel_id),
            _ => false,
        }
    }

    fn toggle_messages_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.messages_tab_visible(cx) {
            self.hide_script_dock(window, cx);
            return;
        }
        self.show_messages_tab(window, cx);
    }

    fn clear_message_counts(&mut self, cx: &mut Context<Self>) {
        self.messages.update(cx, |panel, cx| {
            panel.clear_counts(cx);
        });
        cx.notify();
    }

    fn show_messages_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel_id = PanelId::from(self.messages.entity_id());
        let dock_open = self.script_dock_open(cx);
        if !dock_open {
            self.show_script_dock(window, cx);
        }
        self.dock_area.update(cx, |area, cx| {
            if let Some((node, ix, active_ix)) =
                Self::panel_tab_slot(area, DockPlacement::Bottom, panel_id)
            {
                if ix != active_ix {
                    area.move_panel(
                        panel_id,
                        InsertTarget::Tabs {
                            node,
                            ix: Some(ix),
                            activate: true,
                        },
                        window,
                        cx,
                    );
                }
            }
        });
        self.messages.focus_handle(cx).focus(window, cx);
        self.sync_view_menus(cx);
        cx.notify();
    }

    fn choose_channel_layout(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            return;
        };
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.choose_layout(id, Some(name)) {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("channel layout: {err}"), cx);
            });
            return;
        }
        self.after_script_edit(id, window, cx);
    }

    pub(crate) fn monitor_ui_json(&self) -> Option<&'static str> {
        self.playback.monitor_ui_json()
    }

    pub(crate) fn monitor_param(&self, address: &str) -> Option<f32> {
        self.playback.monitor_param(address)
    }

    pub(crate) fn monitor_meters(&self) -> HashMap<String, f32> {
        self.playback.monitor_meters()
    }

    pub(crate) fn set_monitor_param(&self, address: &str, value: f32) {
        self.playback.set_monitor_param(address, value);
    }

    pub(crate) fn output_device(&self) -> Option<&str> {
        self.output_device.as_deref()
    }

    pub(crate) fn set_output_device(
        &mut self,
        spec: Option<&str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let device = resolve_output_device(spec).map_err(|err| err.to_string())?;
        self.playback
            .set_output_device(&device)
            .map_err(|err| err.to_string())?;
        self.output_device = spec.map(|_| output_device_name(&device));
        self.refresh_output_devices_cache();
        self.monitor.update(cx, |_, cx| cx.notify());
        cx.notify();
        Ok(())
    }

    pub(crate) fn select_output_device(
        &mut self,
        spec: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = self.set_output_device(spec, window, cx) {
            self.script
                .log(ScriptLogLevel::Error, "output".into(), err.clone());
            self.flush_script_logs(cx);
            self.messages.update(cx, |panel, cx| {
                panel.append(
                    vec![LogLine {
                        level: LogLevel::Error,
                        topic: "output".into(),
                        text: format!(
                            "Failed to open audio output ({err}). Select another device in Monitor."
                        )
                        .into(),
                    }],
                    cx,
                );
            });
            cx.notify();
        }
    }

    pub(crate) fn toggle_monitor_params_pin(&mut self, cx: &mut Context<Self>) {
        let Some(views) = self.active_views() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            doc.monitor_params_pinned = !doc.monitor_params_pinned;
            if !doc.monitor_params_pinned {
                doc.pinned_monitor_params.clear();
            }
            cx.notify();
        });
        self.monitor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(crate) fn set_monitor_chain(
        &mut self,
        chain: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.active_views() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            doc.composition
                .write()
                .unwrap()
                .set_monitor_chain(chain.map(str::to_string));
            cx.notify();
        });
        views.document.update(cx, |doc, _| {
            self.playback.sync_from_document(doc);
        });
        self.monitor.update(cx, |_, cx| cx.notify());
        self.update_window_title(window, cx);
        cx.notify();
    }

    pub(crate) fn set_playback_channel(
        &mut self,
        index: usize,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.active_views() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            let mut composition = doc.composition.write().unwrap();
            let n = composition.channel_count();
            if index >= n {
                return;
            }
            let mut selected: Vec<bool> = match composition.playback_channels() {
                Some(channels) => (0..n).map(|i| channels.contains(&i)).collect(),
                None => vec![true; n],
            };
            selected[index] = enabled;
            if !selected.iter().any(|on| *on) {
                selected[index] = true;
            }
            let channels: Vec<usize> = selected
                .iter()
                .enumerate()
                .filter_map(|(i, on)| on.then_some(i))
                .collect();
            composition.set_playback_channels(Some(channels));
            drop(composition);
            cx.notify();
        });
        views.document.update(cx, |doc, _| {
            self.playback.sync_from_document(doc);
        });
        self.monitor.update(cx, |_, cx| cx.notify());
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn toggle_monitor_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.monitor_tab_visible(cx) {
            self.toggle_detail_dock(window, cx);
            return;
        }
        self.show_monitor_tab(window, cx);
    }

    fn show_monitor_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel_id = PanelId::from(self.monitor.entity_id());
        let dock_open = self.detail_dock_open(cx);
        let already_active = dock_open
            && Self::panel_tab_slot(&self.dock_area.read(cx), DockPlacement::Right, panel_id)
                .is_some_and(|(_, ix, active_ix)| ix == active_ix);
        if already_active {
            return;
        }
        if !dock_open {
            self.dock_area.update(cx, |area, cx| {
                area.toggle_dock(DockPlacement::Right, window, cx);
            });
        }
        self.dock_area.update(cx, |area, cx| {
            if let Some((node, ix, active_ix)) =
                Self::panel_tab_slot(area, DockPlacement::Right, panel_id)
            {
                if ix != active_ix {
                    area.move_panel(
                        panel_id,
                        InsertTarget::Tabs {
                            node,
                            ix: Some(ix),
                            activate: true,
                        },
                        window,
                        cx,
                    );
                }
            }
        });
        self.refresh_output_devices_cache();
        self.monitor.update(cx, |_, cx| cx.notify());
        self.sync_view_menus(cx);
        cx.notify();
    }

    fn refresh_output_devices_cache(&mut self) {
        self.output_devices_cache = list_output_devices()
            .map(|devices| devices.into_iter().map(|info| info.name).collect())
            .unwrap_or_default();
    }

    pub(crate) fn session(&self) -> &Session {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub(crate) fn session_active(&self) -> Option<DocumentId> {
        self.session.active()
    }

    pub(crate) fn session_document_ids(&self) -> Vec<DocumentId> {
        self.session.documents().iter().map(|doc| doc.id).collect()
    }

    pub(crate) fn script_save_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.save_session(window, cx);
        Ok(())
    }

    pub(crate) fn script_save_session_to(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.write_session(path, AfterSessionWrite::None, window, cx);
        Ok(())
    }

    pub(crate) fn script_save_document(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.session.get(id).is_none() {
            return Err("composition is not open".into());
        }
        if let Some(path) = self.save_path_for_document(id) {
            self.write_project(id, path, AfterWrite::None, false, window, cx);
        } else {
            self.prompt_save_as_for(id, AfterWrite::None, window, cx);
        }
        Ok(())
    }

    pub(crate) fn script_close_document(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_document(id, window, cx);
    }

    pub(crate) fn script_display_name(&self, id: DocumentId, cx: &App) -> Option<String> {
        Some(self.display_title(id, cx).to_string())
    }

    pub(crate) fn script_set_display_name(
        &mut self,
        id: DocumentId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> mlua::Result<()> {
        let Some(views) = self.views.get(&id) else {
            return Err(mlua::Error::runtime("composition is not open"));
        };
        views.composition.write().unwrap().set_display_title(name);
        self.refresh_explorer(cx);
        if self.session.active() == Some(id) {
            self.update_window_title(window, cx);
        }
        Ok(())
    }

    pub(crate) fn script_composition_uuid(&self, id: DocumentId, _cx: &App) -> Option<String> {
        self.views
            .get(&id)
            .map(|views| views.composition.read().unwrap().id().to_string())
    }

    pub(crate) fn script_composition_parent(&self, id: DocumentId, cx: &App) -> Option<DocumentId> {
        self.lineage_tree(cx).parent(id)
    }

    pub(crate) fn script_composition_children(&self, id: DocumentId, cx: &App) -> Vec<DocumentId> {
        let ordered: Vec<DocumentId> = self.session.documents().iter().map(|d| d.id).collect();
        self.lineage_tree(cx).children(id, &ordered)
    }

    pub(crate) fn script_break_out(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Vec<DocumentId>, String> {
        if self.session.active() != Some(id) {
            self.session.focus(id);
            self.apply_active(window, cx);
        }
        let before: HashSet<DocumentId> = self.session.documents().iter().map(|d| d.id).collect();
        self.break_out_selection(window, cx);
        let created: Vec<DocumentId> = self
            .session
            .documents()
            .iter()
            .map(|d| d.id)
            .filter(|doc| !before.contains(doc))
            .collect();
        Ok(created)
    }

    pub(crate) fn script_path(&self, id: DocumentId) -> Option<PathBuf> {
        self.session
            .get(id)
            .and_then(|doc| doc.project_path.clone().or_else(|| doc.source_path.clone()))
    }

    pub(crate) fn script_open(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<DocumentId, String> {
        self.open_path(path.clone(), window, cx);
        self.session
            .find_by_path(&path)
            .or_else(|| self.session.active())
            .ok_or_else(|| format!("failed to open {}", path.display()))
    }

    pub(crate) fn script_alert(
        &self,
        subject: &str,
        body: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subject = subject.to_string();
        let body = body.to_string();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert.title(subject.clone()).description(body.clone())
        });
    }

    /// Expire a file-drop snapshot when no drag is active.
    /// Layout is created by [`Self::ensure_file_drop_layout`], which only runs
    /// for `ExternalPaths` (OS file drags), not dock splitter drags.
    pub(crate) fn sync_file_drop_layout(&mut self, cx: &App) -> Option<Arc<DropLayout>> {
        if !cx.has_active_drag() {
            self.drop_layout = None;
        }
        self.drop_layout.clone()
    }

    /// Snapshot drop-target layout for an OS file drag. `notify` is deferred
    /// because GPUI takes `active_drag` while computing `drag_over` styles.
    pub(crate) fn ensure_file_drop_layout(&mut self, cx: &mut Context<Self>) {
        if self.drop_layout.is_some() {
            return;
        }
        self.drop_layout = Some(Arc::new(self.script.drop_layout()));
        let empty = self.empty_editors.clone();
        let workspaces: Vec<_> = self
            .views
            .values()
            .map(|views| views.workspace.clone())
            .collect();
        cx.defer(move |cx| {
            empty.update(cx, |_, cx| cx.notify());
            for workspace in workspaces {
                workspace.update(cx, |_, cx| cx.notify());
            }
        });
    }

    fn clear_drop_layout(&mut self, cx: &mut Context<Self>) {
        if self.drop_layout.take().is_some() {
            self.notify_drop_targets(cx);
        }
    }

    fn notify_drop_targets(&mut self, cx: &mut Context<Self>) {
        self.empty_editors.update(cx, |_, cx| cx.notify());
        for views in self.views.values() {
            views.workspace.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    pub(crate) fn invoke_drop_workflow(
        &mut self,
        name: &str,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_drop_layout(cx);
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.invoke_workflow(name, paths) {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("workflow `{name}`: {err}"), cx);
            });
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        cx.notify();
    }

    pub(crate) fn invoke_menu_workflow(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.invoke_menu_workflow(name) {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("workflow `{name}`: {err}"), cx);
            });
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        cx.notify();
    }

    pub(crate) fn dispatch_workflow_command(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.dispatch_workflow_command(command) {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("workflow command `{command}`: {err}"), cx);
            });
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        cx.notify();
    }

    pub(crate) fn dispatch_toolbar_path(
        &mut self,
        id: &str,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        match self.script.dispatch_toolbar_path(id, paths) {
            Ok(_) => {}
            Err(err) => {
                self.repl.update(cx, |repl, cx| {
                    repl.append_error(&format!("toolbar path `{id}`: {err}"), cx);
                });
            }
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        cx.notify();
    }

    pub(crate) fn dispatch_toolbar_toggle(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.dispatch_toolbar_toggle(id) {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("toolbar toggle `{id}`: {err}"), cx);
            });
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        cx.notify();
    }

    pub(crate) fn set_toolbar_path_value(
        &mut self,
        id: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.set_toolbar_path_value(id, value) {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("toolbar path `{id}`: {err}"), cx);
            });
        }
        // Sync the AppView snapshot only. Do not refresh WorkflowBar here: the
        // path InputEvent handler already updated local items, and refreshing
        // can nest a WorkflowBar update during text input flush_effects.
        self.workflow_bar = self.script.toolbar_snapshot();
        let logs = self.script.take_logs();
        if !logs.is_empty() {
            self.messages.update(cx, |panel, cx| {
                panel.append(to_log_lines(logs), cx);
            });
            cx.notify();
        }
        self.sync_view_menus(cx);
    }

    pub(crate) fn script_replace_document(
        &mut self,
        id: DocumentId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<DocumentId, String> {
        if is_fasession_path(&path) {
            return Err("cannot replace a composition with a session file".into());
        }
        if self.session.get(id).is_none() {
            self.open_path(path.clone(), window, cx);
            return self
                .session
                .find_by_path(&path)
                .or_else(|| self.session.active())
                .ok_or_else(|| format!("failed to open {}", path.display()));
        }
        if let Some(existing) = self.session.find_by_path(&path) {
            if existing != id {
                self.activate_or_replace_tab(existing, window, cx);
            }
            return Ok(existing);
        }
        let modified = self
            .views
            .get(&id)
            .is_some_and(|views| views.composition.read().unwrap().is_modified());
        if !modified {
            self.finish_replace_document(id, path.clone(), window, cx);
            return Ok(id);
        }
        self.pending_replace = Some((id, path));
        self.prompt_unsaved_replace(id, window, cx);
        Ok(id)
    }

    fn prompt_unsaved_replace(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.display_title(id, cx);
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Unsaved changes")
                .description(format!("Save changes to {name} before replacing?"))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("replace-dont-save")
                                .outline()
                                .label("Don't Save")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            if let Some((id, path)) = this.pending_replace.take() {
                                                this.finish_replace_document(id, path, window, cx);
                                            }
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("replace-cancel")
                                .outline()
                                .label("Cancel")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, _| {
                                            this.pending_replace = None;
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("replace-save")
                                .primary()
                                .label("Save")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            this.save_then_replace(id, window, cx);
                                        });
                                    }
                                }),
                        ),
                )
        });
    }

    fn save_then_replace(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.save_path_for_document(id) {
            self.write_project(id, path, AfterWrite::Replace, false, window, cx);
        } else {
            self.prompt_save_as_for(id, AfterWrite::Replace, window, cx);
        }
    }

    fn finish_replace_document(
        &mut self,
        id: DocumentId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_playback_into_active(cx);
        if let Some(views) = self.views.get(&id) {
            views.document.read(cx).progress.cancel();
        }
        self.session.replace_document_path(id, path.clone());
        self.focus_document(id, window, cx);
        self.apply_active(window, cx);
        self.spawn_document_load(id, path, window, cx);
        self.refresh_explorer(cx);
        self.update_window_title(window, cx);
        cx.notify();
    }

    pub(crate) fn script_with_document<R>(
        &mut self,
        id: DocumentId,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut BufferDocument) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        let views = self
            .views
            .get(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        views.document.update(cx, |doc, _| f(doc))
    }

    pub(crate) fn after_script_edit(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.active() == Some(id) {
            if let Some(views) = self.views.get(&id).cloned() {
                views.document.update(cx, |doc, cx| {
                    self.playback.sync_from_document(doc);
                    cx.notify();
                });
                views.waveform.update(cx, |_, cx| cx.notify());
                self.monitor.update(cx, |_, cx| cx.notify());
            }
        }
        self.refresh_explorer(cx);
        self.request_waveform_analysis(id, cx);
        self.update_window_title(window, cx);
        self.sync_view_menus(cx);
        cx.notify();
    }

    pub(crate) fn invoke_command(
        &mut self,
        command_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match command_id {
            "file.open" => self.prompt_open_file(window, cx),
            "file.save" => self.save_active(window, cx),
            "file.save_as" => self.prompt_save_as(window, cx),
            "file.save_session" => self.save_session(window, cx),
            "file.save_session_as" => {
                self.prompt_save_session_as(AfterSessionWrite::None, window, cx)
            }
            "file.close" => self.request_close_active(window, cx),
            "file.render" => self.open_render_sheet(window, cx),
            "file.quit" => self.request_quit(window, cx),
            "help.about" => self.show_about(window, cx),
            "transport.home" => {
                self.playback.home();
                self.sync_playback_to_document(cx);
            }
            "transport.previous" => {
                self.playback.previous();
                self.sync_playback_to_document(cx);
            }
            "transport.start" => {
                self.playback.start();
                self.sync_playback_to_document(cx);
            }
            "transport.play_pause" => {
                self.playback.toggle_play_pause();
                self.sync_playback_to_document(cx);
            }
            "transport.stop" => {
                self.playback.stop();
                self.sync_playback_to_document(cx);
            }
            "transport.next" => {
                self.playback.next();
                self.sync_playback_to_document(cx);
            }
            "transport.end" => {
                self.playback.end();
                self.sync_playback_to_document(cx);
            }
            "transport.loop" => {
                self.playback.toggle_loop();
                cx.notify();
            }
            "transport.preview" => {
                self.toggle_preview(cx);
            }
            "view.fit_all" => {
                if let Some(views) = self.active_views() {
                    views.waveform.update(cx, |view, cx| view.fit(cx));
                }
            }
            "view.frame" => {
                if let Some(views) = self.active_views() {
                    views.waveform.update(cx, |view, cx| view.frame(cx));
                }
            }
            "view.zoom_in" => {
                if let Some(views) = self.active_views() {
                    views.waveform.update(cx, |view, cx| view.zoom_in(cx));
                }
            }
            "view.zoom_out" => {
                if let Some(views) = self.active_views() {
                    views.waveform.update(cx, |view, cx| view.zoom_out(cx));
                }
            }
            "view.show-explorer" => self.set_explorer_dock(true, window, cx),
            "view.hide-explorer" => self.set_explorer_dock(false, window, cx),
            "view.toggle-explorer" => self.toggle_explorer_dock(window, cx),
            "view.show-detail" => self.set_detail_dock(true, window, cx),
            "view.hide-detail" => self.set_detail_dock(false, window, cx),
            "view.toggle-detail" => self.toggle_detail_dock(window, cx),
            "view.show-script" => self.show_script_dock(window, cx),
            "view.hide-script" => self.hide_script_dock(window, cx),
            "view.toggle-script" => self.toggle_script_dock(window, cx),
            "view.overlay_envelope_peak" => self.toggle_envelope_overlay(window, cx),
            "view.waveform_peaks" => self.set_waveform_representation(
                field_ui_components::WaveformRepresentation::Peaks,
                window,
                cx,
            ),
            "view.waveform_spectrum" => self.set_waveform_representation(
                field_ui_components::WaveformRepresentation::Spectrum,
                window,
                cx,
            ),
            "view.waveform_peaks_spectrum" => self.set_waveform_representation(
                field_ui_components::WaveformRepresentation::PeaksSpectrum,
                window,
                cx,
            ),
            "analyze.selection_only" => self.toggle_analyze_selection_only(cx),
            "analyze.envelope_peak" => self.run_analyze_envelope_peak(window, cx),
            "analyze.transients" => self.run_analyze_transients(window, cx),
            "edit.undo" => self.run_edit(cx, |doc| {
                doc.edit_undo();
            }),
            "edit.redo" => self.run_edit(cx, |doc| {
                doc.edit_redo();
            }),
            "edit.cut" => self.run_edit(cx, |doc| doc.edit_cut()),
            "edit.copy" => self.run_edit(cx, |doc| doc.edit_copy()),
            "edit.paste" => self.run_edit(cx, |doc| doc.edit_paste()),
            "edit.clear" => self.run_edit(cx, |doc| doc.edit_clear()),
            "edit.remove" => self.run_edit(cx, |doc| doc.edit_remove()),
            "edit.duplicate" => self.run_edit(cx, |doc| doc.edit_duplicate()),
            "edit.trim" => self.run_edit(cx, |doc| doc.edit_trim()),
            "edit.break_out" => self.break_out_selection(window, cx),
            "selection.select_all" => self.run_edit(cx, |doc| doc.select_all()),
            "selection.select_none" => self.run_edit(cx, |doc| doc.clear_selection()),
            "selection.invert" => self.run_edit(cx, |doc| doc.invert_selection()),
            "selection.marker_type_blue" => self.set_active_marker_type(MARKER_TYPE_BLUE, cx),
            "selection.marker_type_yellow" => self.set_active_marker_type(MARKER_TYPE_YELLOW, cx),
            "selection.marker_type_purple" => self.set_active_marker_type(MARKER_TYPE_PURPLE, cx),
            "selection.snap_to_marker" => {
                self.update_active_document(cx, |doc| doc.toggle_marker_snap());
                self.sync_view_menus(cx);
            }
            "selection.add_at_hover" => {
                self.add_marker_at_hover = !self.add_marker_at_hover;
                self.sync_view_menus(cx);
                cx.notify();
            }
            "selection.add_marker" => {
                let kind = self.active_marker_type.clone();
                let sample = self.marker_target_sample(cx).unwrap_or(0);
                self.run_edit(cx, |doc| {
                    doc.add_marker_of_type(sample, &kind);
                });
            }
            "selection.delete_marker" => {
                let kind = self.active_marker_type.clone();
                if let Some(sample) = self.marker_target_sample(cx) {
                    self.run_edit(cx, |doc| {
                        doc.remove_marker_at_type(sample, &kind);
                    });
                }
            }
            other => return Err(format!("unknown command `{other}`")),
        }
        Ok(())
    }

    fn run_edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut BufferDocument)) {
        let Some(id) = self.session.active() else {
            return;
        };
        let Some(views) = self.views.get(&id).cloned() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            f(doc);
            self.playback.sync_from_document(doc);
            cx.notify();
        });
        self.refresh_explorer(cx);
        self.request_waveform_analysis(id, cx);
    }

    fn update_active_document(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut BufferDocument),
    ) {
        let Some(views) = self.active_views() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            f(doc);
            cx.notify();
        });
    }

    fn set_active_marker_type(&mut self, marker_type: &str, cx: &mut Context<Self>) {
        if self.active_marker_type == marker_type {
            return;
        }
        self.active_marker_type = marker_type.to_string();
        self.sync_view_menus(cx);
        cx.notify();
    }

    fn marker_target_sample(&self, cx: &App) -> Option<usize> {
        let views = self.active_views()?;
        if self.add_marker_at_hover {
            if let Some(sample) = views.waveform.read(cx).hover_sample() {
                return Some(sample);
            }
        }
        views
            .document
            .read(cx)
            .current_position
            .as_ref()
            .map(|pos| pos.sample)
    }

    fn cancel_inactive_analysis_jobs(&self, except: DocumentId, cx: &App) {
        for (id, views) in &self.views {
            if *id == except {
                continue;
            }
            let progress = views.document.read(cx).progress.clone();
            // Never cancel document loads — progress is shared with analysis, and
            // batch-open under Peaks+Spectrum used to abort earlier "opening"
            // epochs so tabs stayed on empty placeholders ("FieldAssist").
            if progress
                .snapshot()
                .is_some_and(|state| state.label != "opening")
            {
                progress.cancel();
            }
        }
    }

    fn drain_analysis_requests(&mut self, cx: &mut Context<Self>) {
        let mut jobs: Vec<(DocumentId, Vec<AnalysisKind>)> = Vec::new();
        for (id, views) in &self.views {
            let requests = views.document.read(cx).take_analysis_requests();
            if !requests.is_empty() {
                jobs.push((*id, requests));
            }
        }
        for (id, kinds) in jobs {
            self.spawn_analysis_pass(id, kinds, cx);
        }
    }

    /// Queue analysis for whatever the current waveform representation needs.
    ///
    /// Peaks + Spectrum always starts MinMax and Spectral together so one
    /// shared pass feeds both panes (no double progress / peaks-then-spectrum).
    fn request_waveform_analysis(&self, id: DocumentId, cx: &mut Context<Self>) {
        let Some(views) = self.views.get(&id) else {
            return;
        };
        // Empty placeholders (open_path before load finishes) must not start
        // analysis — Spectrum/Peaks+Spectrum would otherwise claim progress and
        // race batch loads via cancel_inactive_analysis_jobs.
        if views.composition.read().unwrap().frames() == 0 {
            return;
        }
        if views
            .document
            .read(cx)
            .progress
            .snapshot()
            .is_some_and(|state| state.label == "opening")
        {
            return;
        }
        let rep = views.document.read(cx).waveform_representation;
        let kinds: &[AnalysisKind] = match rep {
            field_ui_components::WaveformRepresentation::PeaksSpectrum => {
                &[AnalysisKind::MinMax, AnalysisKind::Spectral]
            }
            field_ui_components::WaveformRepresentation::Spectrum => &[AnalysisKind::Spectral],
            field_ui_components::WaveformRepresentation::Peaks => &[AnalysisKind::MinMax],
        };
        self.request_analysis_kinds(id, kinds, cx);
    }

    fn request_analysis(&self, id: DocumentId, kind: AnalysisKind, cx: &mut Context<Self>) {
        self.request_analysis_kinds(id, &[kind], cx);
    }

    fn request_analysis_kinds(
        &self,
        id: DocumentId,
        kinds: &[AnalysisKind],
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.views.get(&id) else {
            return;
        };
        // Queue first so concurrent ensure_* / menu requests coalesce into one
        // shared pass. If a job is already running, kinds stay queued for the
        // next timer drain.
        for &kind in kinds {
            views.document.read(cx).request_analysis(kind);
        }
        let queued = views.document.read(cx).take_analysis_requests();
        if queued.is_empty() {
            return;
        }
        self.spawn_analysis_pass(id, queued, cx);
    }

    fn spawn_analysis_pass(
        &self,
        id: DocumentId,
        mut kinds: Vec<AnalysisKind>,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.views.get(&id) else {
            return;
        };
        let composition = views.composition.clone();
        let rep = views.document.read(cx).waveform_representation;
        // Expand to representation siblings before filtering so an early
        // MinMax-only open under Peaks+Spectrum still starts a shared pass.
        if matches!(
            rep,
            field_ui_components::WaveformRepresentation::PeaksSpectrum
        ) {
            kinds.push(AnalysisKind::MinMax);
            kinds.push(AnalysisKind::Spectral);
        } else if matches!(rep, field_ui_components::WaveformRepresentation::Spectrum) {
            kinds.push(AnalysisKind::Spectral);
        }
        kinds.sort_by_key(|k| k.id());
        kinds.dedup();
        kinds.retain(|kind| match kind {
            AnalysisKind::MinMax => composition.read().unwrap().needs_peak_build(),
            AnalysisKind::EnvelopePeak => composition.read().unwrap().needs_envelope_peak_build(),
            AnalysisKind::Spectral => composition.read().unwrap().needs_spectral_build(),
            AnalysisKind::Transients => true,
        });
        if kinds.is_empty() {
            return;
        }
        if views.document.read(cx).progress.snapshot().is_some() {
            // A single-kind job may already be running (e.g. MinMax from
            // activate). Cancel it so we can start the shared pass now rather
            // than finishing peaks first and spectrum later.
            if kinds.len() > 1 {
                views.document.read(cx).progress.cancel();
            } else {
                for kind in kinds {
                    views.document.read(cx).request_analysis(kind);
                }
                return;
            }
        }
        if kinds
            .iter()
            .any(|k| matches!(k, AnalysisKind::EnvelopePeak | AnalysisKind::Transients))
        {
            // Menu Analyze snapshots the target (honoring Selection Only). Pull
            // rebuilds after edits leave this unset so the job can use dirty
            // neighborhoods (or the full timeline) instead of the selection.
            let ranges = match views.document.read(cx).take_pending_analysis_target() {
                Some(snapshotted) => snapshotted,
                None => None,
            };
            if matches!(ranges.as_ref(), Some(r) if r.is_empty()) {
                return;
            }
            composition.write().unwrap().begin_analysis_target(ranges);
        }
        self.cancel_inactive_analysis_jobs(id, cx);
        let progress = views.document.read(cx).progress.clone();
        let label = AnalysisKind::progress_label_for_kinds(&kinds);
        let epoch = progress.begin(label);
        views.waveform.update(cx, |view, cx| {
            view.bump_paint_epoch(cx);
        });
        let pending = self.pending_analysis.clone();
        let pending_logs = self.pending_analysis_logs.clone();
        std::thread::spawn(move || {
            loop {
                match Composition::build_next_analysis_kinds(
                    &composition,
                    &kinds,
                    Some(&progress),
                    epoch,
                ) {
                    Ok(AnalysisBlockOutcome::Progress) => {
                        pending.lock().unwrap().push(id);
                    }
                    Ok(AnalysisBlockOutcome::Complete) => {
                        let stats = composition.read().unwrap().analysis_pass_stats();
                        // Push the log before notifying the UI so the next
                        // drain_pending_analysis cannot race past an empty queue.
                        pending_logs.lock().unwrap().push(LogLine {
                            level: LogLevel::Info,
                            topic: "analysis".into(),
                            text: stats.info_message(&kinds),
                        });
                        pending.lock().unwrap().push(id);
                        break;
                    }
                    Ok(AnalysisBlockOutcome::Cancelled) => break,
                    Err(err) => {
                        eprintln!(
                            "failed to run {}: {err:#}",
                            AnalysisKind::progress_label_for_kinds(&kinds)
                        );
                        break;
                    }
                }
            }
            progress.finish(epoch);
            pending.lock().unwrap().push(id);
        });
    }

    fn drain_pending_analysis(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let logs = std::mem::take(&mut *self.pending_analysis_logs.lock().unwrap());
        let completed = std::mem::take(&mut *self.pending_analysis.lock().unwrap());
        if logs.is_empty() && completed.is_empty() {
            return;
        }
        if !logs.is_empty() {
            self.messages.update(cx, |panel, cx| {
                panel.append(logs, cx);
            });
        }
        for id in completed {
            let Some(views) = self.views.get(&id) else {
                continue;
            };
            views.document.update(cx, |_, cx| cx.notify());
            views.waveform.update(cx, |view, cx| {
                view.bump_paint_epoch(cx);
            });
        }
        window.refresh();
    }

    fn run_analyze_envelope_peak(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            return;
        };
        if let Some(views) = self.views.get(&id) {
            if !views.document.read(cx).snapshot_analysis_target() {
                return;
            }
            views.document.update(cx, |doc, cx| {
                doc.show_envelope_peak = true;
                cx.notify();
            });
            // Clear prior stream so Analyze re-runs.
            views
                .composition
                .write()
                .unwrap()
                .analysis_streams_mut()
                .clear();
        }
        self.request_analysis(id, AnalysisKind::EnvelopePeak, cx);
        self.sync_view_menus(cx);
        window.refresh();
    }

    fn run_analyze_transients(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            return;
        };
        if let Some(views) = self.views.get(&id) {
            if !views.document.read(cx).snapshot_analysis_target() {
                return;
            }
        }
        self.request_analysis(id, AnalysisKind::Transients, cx);
    }

    fn toggle_analyze_selection_only(&mut self, cx: &mut Context<Self>) {
        let Some(views) = self.active_views() else {
            return;
        };
        views.document.update(cx, |doc, cx| {
            doc.analyze_selection_only = !doc.analyze_selection_only;
            cx.notify();
        });
        self.sync_view_menus(cx);
    }

    fn toggle_envelope_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            return;
        };
        let enabled = if let Some(views) = self.views.get(&id) {
            views.document.update(cx, |doc, cx| {
                doc.show_envelope_peak = !doc.show_envelope_peak;
                let on = doc.show_envelope_peak;
                cx.notify();
                on
            })
        } else {
            false
        };
        if enabled {
            if let Some(views) = self.views.get(&id) {
                let _ = views.document.read(cx).snapshot_analysis_target();
            }
            self.request_analysis(id, AnalysisKind::EnvelopePeak, cx);
        }
        self.sync_view_menus(cx);
        window.refresh();
    }

    fn set_waveform_representation(
        &mut self,
        representation: field_ui_components::WaveformRepresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = self.waveform_representation != representation;
        self.waveform_representation = representation;
        if changed {
            let ids: Vec<_> = self.views.keys().copied().collect();
            for id in ids {
                let Some(views) = self.views.get(&id).cloned() else {
                    continue;
                };
                views.document.update(cx, |doc, cx| {
                    doc.waveform_representation = representation;
                    cx.notify();
                });
                views.waveform.update(cx, |view, cx| {
                    view.bump_paint_epoch(cx);
                });
                if matches!(
                    representation,
                    field_ui_components::WaveformRepresentation::Spectrum
                        | field_ui_components::WaveformRepresentation::PeaksSpectrum
                ) {
                    let mut kinds = Vec::new();
                    if matches!(
                        representation,
                        field_ui_components::WaveformRepresentation::PeaksSpectrum
                    ) {
                        kinds.push(AnalysisKind::MinMax);
                    }
                    kinds.push(AnalysisKind::Spectral);
                    self.request_analysis_kinds(id, &kinds, cx);
                }
            }
        }
        self.sync_view_menus(cx);
        window.refresh();
    }

    fn show_load_error(&self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message = message.to_string();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Failed to open file")
                .description(message.clone())
        });
    }

    fn show_media_warning(&self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message = message.to_string();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Source media changed")
                .description(message.clone())
        });
    }

    fn show_save_error(&self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message = message.to_string();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert.title("Failed to save").description(message.clone())
        });
    }

    fn suggested_save_directory(&self, id: DocumentId, cx: &App) -> PathBuf {
        if let Some(doc) = self.session.get(id) {
            if let Some(parent) = doc
                .project_path
                .as_ref()
                .or(doc.source_path.as_ref())
                .and_then(|path| path.parent())
            {
                return parent.to_path_buf();
            }
        }
        if let Some(views) = self.views.get(&id) {
            if let Some(parent) = views
                .composition
                .read()
                .unwrap()
                .pool()
                .first()
                .and_then(|media| media.path.parent())
            {
                return parent.to_path_buf();
            }
        }
        let _ = cx;
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            self.show_save_error("No composition is open.", window, cx);
            return;
        };
        if let Some(path) = self.save_path_for_document(id) {
            self.write_project(id, path, AfterWrite::None, false, window, cx);
        } else {
            self.prompt_save_as(window, cx);
        }
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            self.show_save_error("No composition is open.", window, cx);
            return;
        };
        self.prompt_save_as_for(id, AfterWrite::None, window, cx);
    }

    fn prompt_save_as_for(
        &mut self,
        id: DocumentId,
        after: AfterWrite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.get(id).is_none() {
            self.show_save_error("Composition is not open.", window, cx);
            return;
        }
        let directory = self.suggested_save_directory(id, cx);
        let suggested = self
            .views
            .get(&id)
            .map(|views| views.composition.read().unwrap().suggested_facomp_name())
            .unwrap_or_else(|| "untitled.facomp".into());
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested));
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                _ => {
                    if after == AfterWrite::ContinuePending {
                        let _ = cx.update(|_, cx| {
                            view.update(cx, |this, _| {
                                this.quit_save_queue.clear();
                                this.pending_continue = None;
                            });
                        });
                    }
                    if after == AfterWrite::Replace {
                        let _ = cx.update(|_, cx| {
                            view.update(cx, |this, _| {
                                this.pending_replace = None;
                            });
                        });
                    }
                    return;
                }
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.write_project(id, path, after, true, window, cx);
                });
            });
        })
        .detach();
    }

    fn write_project(
        &mut self,
        id: DocumentId,
        path: PathBuf,
        after: AfterWrite,
        mint_new_id: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.views.get(&id) else {
            self.show_save_error("Composition is not open.", window, cx);
            if after == AfterWrite::ContinuePending {
                self.quit_save_queue.clear();
                self.pending_continue = None;
            }
            if after == AfterWrite::Replace {
                self.pending_replace = None;
            }
            return;
        };
        let result = {
            let started = Instant::now();
            let mut composition = views.composition.write().unwrap();
            let path_changed = self
                .session
                .get(id)
                .and_then(|doc| doc.project_path.as_ref())
                .is_some_and(|existing| existing != &path);
            if mint_new_id && path_changed {
                composition.mint_new_id();
            }
            let result = composition.save_to_path(&path);
            (result, started.elapsed().as_secs_f64())
        };
        match result {
            (Ok(()), elapsed) => {
                self.session.set_project_path(id, path);
                self.fire_saved_script(id, elapsed, window, cx);
                match after {
                    AfterWrite::None => {
                        self.refresh_explorer(cx);
                        self.update_window_title(window, cx);
                        cx.notify();
                    }
                    AfterWrite::Close => self.close_document(id, window, cx),
                    AfterWrite::ContinuePending => {
                        self.quit_save_queue.retain(|queued| *queued != id);
                        self.refresh_explorer(cx);
                        self.update_window_title(window, cx);
                        self.pump_quit_saves(window, cx);
                    }
                    AfterWrite::Replace => {
                        self.refresh_explorer(cx);
                        self.update_window_title(window, cx);
                        if let Some((replace_id, path)) = self.pending_replace.take() {
                            self.finish_replace_document(replace_id, path, window, cx);
                        }
                    }
                }
            }
            (Err(err), _) => {
                if after == AfterWrite::ContinuePending {
                    self.quit_save_queue.clear();
                    self.pending_continue = None;
                }
                if after == AfterWrite::Replace {
                    self.pending_replace = None;
                }
                self.show_save_error(&format!("{err:#}"), window, cx);
            }
        }
    }

    fn request_close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            return;
        };
        self.request_close_document(id, window, cx);
    }

    fn request_close_document(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.get(id).is_none() {
            return;
        }
        let modified = self
            .views
            .get(&id)
            .is_some_and(|views| views.composition.read().unwrap().is_modified());
        if !modified {
            self.close_document(id, window, cx);
            return;
        }
        self.prompt_unsaved_close(id, window, cx);
    }

    fn prompt_unsaved_close(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.display_title(id, cx);
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Unsaved changes")
                .description(format!("Save changes to {name} before closing?"))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("close-dont-save")
                                .outline()
                                .label("Don't Save")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            this.close_document(id, window, cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("close-cancel")
                                .outline()
                                .label("Cancel")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(Button::new("close-save").primary().label("Save").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                view.update(cx, |this, cx| {
                                    this.save_then_close(id, window, cx);
                                });
                            }
                        })),
                )
        });
    }

    fn save_then_close(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.save_path_for_document(id) {
            self.write_project(id, path, AfterWrite::Close, false, window, cx);
        } else {
            self.prompt_save_as_for(id, AfterWrite::Close, window, cx);
        }
    }

    fn modified_compositions(&self, cx: &App) -> Vec<(DocumentId, SharedString)> {
        self.session
            .documents()
            .iter()
            .filter_map(|doc| {
                let modified = self
                    .views
                    .get(&doc.id)
                    .is_some_and(|views| views.composition.read().unwrap().is_modified());
                modified.then(|| (doc.id, self.display_title(doc.id, cx)))
            })
            .collect()
    }

    fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_continue = Some(PendingContinue::Quit);
        self.resolve_unsaved_then_continue(window, cx);
    }

    fn request_open_session(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_continue = Some(PendingContinue::LoadSession(path));
        self.resolve_unsaved_then_continue(window, cx);
    }

    fn resolve_unsaved_then_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let modified = self.modified_compositions(cx);
        if !modified.is_empty() {
            self.prompt_unsaved_then_continue(modified, window, cx);
            return;
        }
        self.after_compositions_clean(window, cx);
    }

    fn pending_continue_is_quit(&self) -> bool {
        matches!(self.pending_continue, Some(PendingContinue::Quit))
    }

    fn prompt_unsaved_then_continue(
        &mut self,
        items: Vec<(DocumentId, SharedString)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let quitting = self.pending_continue_is_quit();
        let description = if quitting {
            "Save changes to these compositions before quitting?"
        } else {
            "Save changes to these compositions before opening this session?"
        };
        let list = cx.new(|cx| QuitUnsavedList::new(items, cx));
        let view = cx.entity();
        list.update(cx, |list, _| {
            let view = view.clone();
            list.set_handler(Rc::new(move |action, ids, window, cx| {
                window.close_dialog(cx);
                let _ = view.update(cx, |this, cx| match action {
                    QuitUnsavedAction::Discard => this.after_compositions_clean(window, cx),
                    QuitUnsavedAction::SaveAll | QuitUnsavedAction::SaveSelected => {
                        this.start_quit_saves(ids, window, cx);
                    }
                });
            }));
        });
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Unsaved changes")
                .description(description)
                .width(px(520.))
                .child(list.clone())
                .footer(div())
        });
    }

    fn after_compositions_clean(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.suspend_active_workflow(window, cx) {
            self.pending_continue = None;
            return;
        }
        if self.session.should_prompt_save() {
            self.prompt_save_session_then_continue(window, cx);
            return;
        }
        self.finish_pending_continue(window, cx);
    }

    fn prompt_save_session_then_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let quitting = self.pending_continue_is_quit();
        let description = if quitting {
            "Save the current session before quitting?"
        } else {
            "Save the current session before opening another?"
        };
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Unsaved session")
                .description(description)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("session-dont-save")
                                .outline()
                                .label("Don't Save")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            this.finish_pending_continue(window, cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("session-cancel")
                                .outline()
                                .label("Cancel")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, _| {
                                            this.pending_continue = None;
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("session-save")
                                .primary()
                                .label("Save")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            this.save_session_then_continue(window, cx);
                                        });
                                    }
                                }),
                        ),
                )
        });
    }

    fn save_session_then_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.session.path().map(Path::to_path_buf) {
            self.write_session(path, AfterSessionWrite::Continue, window, cx);
        } else {
            self.prompt_save_session_as(AfterSessionWrite::Continue, window, cx);
        }
    }

    fn finish_pending_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pending_continue.take() {
            Some(PendingContinue::Quit) => cx.quit(),
            Some(PendingContinue::LoadSession(path)) => {
                if let Err(err) = self.replace_session_from_path(&path, window, cx) {
                    self.show_load_error(&err, window, cx);
                }
            }
            None => {}
        }
    }

    fn start_quit_saves(
        &mut self,
        ids: Vec<DocumentId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quit_save_queue = ids;
        self.pump_quit_saves(window, cx);
    }

    fn pump_quit_saves(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        loop {
            let Some(id) = self.quit_save_queue.first().copied() else {
                self.after_compositions_clean(window, cx);
                return;
            };
            if self.session.get(id).is_none() {
                self.quit_save_queue.remove(0);
                continue;
            }
            if let Some(path) = self.save_path_for_document(id) {
                self.write_project(id, path, AfterWrite::ContinuePending, false, window, cx);
                return;
            }
            self.prompt_save_as_for(id, AfterWrite::ContinuePending, window, cx);
            return;
        }
    }

    fn show_about(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        open_about_window(cx);
    }

    fn open_render_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active() else {
            window.open_alert_dialog(cx, |alert, _, _| {
                alert
                    .title("Nothing to render")
                    .description("Open a composition before rendering.")
            });
            return;
        };
        let Some(views) = self.views.get(&id).cloned() else {
            return;
        };
        let directory = self.suggested_save_directory(id, cx);
        let prefs = self.render_prefs_for(id, cx);
        let composition = views.composition.clone();
        self.render_sheet.update(cx, |sheet, cx| {
            sheet.configure_with_prefs(
                &composition.read().unwrap(),
                directory,
                Some(&prefs),
                window,
                cx,
            );
        });
        self.render_sheet_open = true;
        cx.notify();
    }

    fn render_prefs_for(
        &self,
        id: DocumentId,
        cx: &App,
    ) -> crate::components::render_sheet::RenderPrefs {
        use crate::components::render_sheet::RenderPrefs;
        use crate::render::PcmFormat;

        let mut prefs = RenderPrefs::default();
        prefs.encoder = self.inherited_property(id, "render.encoder", cx);
        prefs.sample_rate = self
            .inherited_property(id, "render.sample_rate", cx)
            .and_then(|value| value.parse().ok());
        prefs.sample_format = self
            .inherited_property(id, "render.sample_format", cx)
            .and_then(|value| match value.as_str() {
                "s16" | "S16" => Some(PcmFormat::S16),
                "s24" | "S24" => Some(PcmFormat::S24),
                "s32" | "S32" => Some(PcmFormat::S32),
                "f32" | "F32" => Some(PcmFormat::F32),
                _ => None,
            });
        if let Some(mask) = self.inherited_property(id, "render.channels", cx) {
            prefs.channels_selected =
                Some(mask.split(',').map(|part| part.trim() == "1").collect());
        }
        prefs
    }

    /// Resolve a document/session string property walking open parents.
    pub(crate) fn inherited_property(&self, id: DocumentId, key: &str, cx: &App) -> Option<String> {
        let tree = self.lineage_tree(cx);
        for ancestor in tree.ancestors(id) {
            if let Some(value) = self
                .session
                .get(ancestor)
                .and_then(|doc| doc.properties.get(key).cloned())
            {
                return Some(value);
            }
        }
        self.session.properties().get(key).cloned()
    }

    fn close_render_sheet(&mut self, cx: &mut Context<Self>) {
        if !self.render_sheet_open {
            return;
        }
        self.render_sheet_open = false;
        cx.notify();
    }

    fn render_sheet_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let can_render = self.render_sheet.read(cx).can_render(cx);
        let sheet = self.render_sheet.clone();
        div()
            .id("render-sheet-layer")
            .absolute()
            .inset_0()
            .occlude()
            .child(
                div()
                    .id("render-sheet-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(hsla(0., 0., 0., 0.25))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_render_sheet(cx);
                    })),
            )
            .child(
                v_flex()
                    .id("render-sheet-panel")
                    .absolute()
                    .top_0()
                    .left(rems(5.))
                    .right(rems(5.))
                    .bg(theme.background)
                    .border_l_1()
                    .border_r_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .shadow_xl()
                    .occlude()
                    .child(div().px_4().py_2().font_semibold().child("Render"))
                    .child(div().px_4().py_1().w_full().child(sheet.clone()))
                    .child(
                        h_flex()
                            .w_full()
                            .px_4()
                            .py_3()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("render-cancel")
                                    .outline()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_render_sheet(cx);
                                    })),
                            )
                            .child(
                                Button::new("render-go")
                                    .primary()
                                    .label("Render")
                                    .disabled(!can_render)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.start_render(&sheet, window, cx);
                                    })),
                            ),
                    ),
            )
    }

    fn start_render(
        &mut self,
        sheet: &Entity<RenderSheet>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.session.active() else {
            return;
        };
        let Some(job) = sheet.read(cx).job(cx) else {
            return;
        };
        let Some(views) = self.views.get(&id).cloned() else {
            return;
        };
        self.close_render_sheet(cx);
        let epoch = views.document.read(cx).progress.begin("rendering");
        let progress = views.document.read(cx).progress.clone();
        views.waveform.update(cx, |_, cx| cx.notify());
        cx.notify();
        let pending = self.pending_render.clone();
        let composition = views.composition.clone();
        std::thread::spawn(move || {
            let result = {
                let guard = composition.read().unwrap();
                crate::render::render_to_path(&guard, &job, Some(&progress), epoch)
                    .map_err(|err| format!("{err:#}"))
            };
            pending.lock().unwrap().push((id, epoch, result));
        });
    }

    fn drain_pending_render(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let completed = std::mem::take(&mut *self.pending_render.lock().unwrap());
        for (id, epoch, result) in completed {
            let Some(views) = self.views.get(&id) else {
                continue;
            };
            if !views.document.read(cx).progress.is_epoch(epoch) {
                continue;
            }
            views.document.read(cx).progress.finish(epoch);
            match result {
                Ok(()) => {
                    views.waveform.update(cx, |_, cx| cx.notify());
                    cx.notify();
                }
                Err(err) => {
                    let message = err.clone();
                    window.open_alert_dialog(cx, move |alert, _, _| {
                        alert.title("Render failed").description(message.clone())
                    });
                    cx.notify();
                }
            }
        }
    }

    fn drain_pending_opens(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = std::mem::take(&mut *self.pending_opens.lock().unwrap());
        for path in paths {
            #[cfg(target_os = "macos")]
            crate::macos_open::log_open(&format!("AppView::open_path {}", path.display()));
            self.open_path(path, window, cx);
        }
    }

    fn drain_pending_loaded_scripts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = std::mem::take(&mut self.pending_loaded_scripts);
        for (id, elapsed) in ids {
            if self.views.contains_key(&id) {
                self.fire_document_scripts(id, elapsed, window, cx);
            }
        }
    }

    fn drain_pending_load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let completed = std::mem::take(&mut *self.pending_load.lock().unwrap());
        for (id, epoch, elapsed, result) in completed {
            let valid = self
                .views
                .get(&id)
                .is_some_and(|views| views.document.read(cx).progress.is_epoch(epoch));
            if !valid {
                continue;
            }
            match result {
                Ok((composition, warnings)) => {
                    self.push_loaded_composition(id, composition, elapsed, window, cx);
                    if !warnings.is_empty() {
                        self.show_media_warning(&warnings.join("\n"), window, cx);
                    }
                }
                Err(err) => {
                    if let Some(views) = self.views.get(&id) {
                        views.document.read(cx).progress.cancel();
                    }
                    self.show_load_error(&err, window, cx);
                    self.close_document(id, window, cx);
                    cx.notify();
                }
            }
        }
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if is_fasession_path(&path) {
            self.request_open_session(path, window, cx);
            return;
        }
        if let Some(id) = self.session.find_by_path(&path) {
            self.activate_or_replace_tab(id, window, cx);
            return;
        }
        self.stop_playback_into_active(cx);
        let composition = Arc::new(RwLock::new(Composition::new(44100, 2)));
        let buffer = Arc::new(RwLock::new(Buffer::empty()));
        let id = self.add_document(composition, buffer, Some(path.clone()), window, cx);
        if is_facomp_path(&path) {
            self.session.set_project_path(id, path.clone());
        }
        self.apply_active(window, cx);
        self.spawn_document_load(id, path, window, cx);
    }

    fn spawn_document_load(
        &mut self,
        id: DocumentId,
        path: PathBuf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(views) = self.views.get(&id) else {
            return;
        };
        let epoch = views.document.read(cx).progress.begin("opening");
        views.waveform.update(cx, |_, cx| cx.notify());
        cx.notify();
        let pending = self.pending_load.clone();
        let progress = views.document.read(cx).progress.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            // Header/project probe only. Overview bins are built later for the
            // active document so session restore cannot stall a small file
            // behind N parallel full-file decodes.
            let result = Composition::load_from_path_with_progress(&path, Some(&progress), epoch)
                .map_err(|err| format!("{err:#}"));
            let elapsed = started.elapsed().as_secs_f64();
            pending.lock().unwrap().push((id, epoch, elapsed, result));
        });
    }

    fn save_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.session.path().map(Path::to_path_buf) {
            self.write_session(path, AfterSessionWrite::None, window, cx);
        } else {
            self.prompt_save_session_as(AfterSessionWrite::None, window, cx);
        }
    }

    fn prompt_save_session_as(
        &mut self,
        after: AfterSessionWrite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = self.suggested_session_directory();
        let suggested = self
            .session
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.session.suggested_fasession_name());
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested));
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                _ => {
                    if after == AfterSessionWrite::Continue {
                        let _ = cx.update(|_, cx| {
                            view.update(cx, |this, _| {
                                this.pending_continue = None;
                            });
                        });
                    }
                    return;
                }
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.write_session(path, after, window, cx);
                });
            });
        })
        .detach();
    }

    fn suggested_session_directory(&self) -> PathBuf {
        if let Some(parent) = self.session.path().and_then(|path| path.parent()) {
            return parent.to_path_buf();
        }
        if let Some(parent) = self.session.documents().iter().find_map(|doc| {
            doc.file_path()
                .and_then(|path| path.parent())
                .map(Path::to_path_buf)
        }) {
            return parent;
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    fn capture_session_ui(&self, window: &Window, cx: &App) -> Option<SessionUi> {
        if !self.session.capture_ui() {
            return None;
        }
        let bounds = window.bounds();
        Some(SessionUi {
            window: Some(SessionWindowUi {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
            }),
            docks: Some(SessionDocksUi {
                explorer: self.explorer_dock_open(cx),
                detail: self.detail_dock_open(cx),
                script: self.script_dock_open(cx),
            }),
        })
    }

    fn write_session(
        &mut self,
        path: PathBuf,
        after: AfterSessionWrite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = self.ensure_untitled_documents_saved(window, cx) {
            if after == AfterSessionWrite::Continue {
                self.pending_continue = None;
            }
            self.show_save_error(&err, window, cx);
            return;
        }
        if !self.suspend_active_workflow(window, cx) {
            if after == AfterSessionWrite::Continue {
                self.pending_continue = None;
            }
            return;
        }
        let titles: HashMap<DocumentId, String> = self
            .session
            .documents()
            .iter()
            .map(|doc| (doc.id, self.display_title(doc.id, cx).to_string()))
            .collect();
        let ui = self.capture_session_ui(window, cx);
        let result = self.session.save_to_path(
            &path,
            |id| titles.get(&id).cloned().unwrap_or_else(|| id.to_string()),
            ui,
        );
        match result {
            Ok(()) => {
                self.fire_session_saved_script(window, cx);
                match after {
                    AfterSessionWrite::None => {
                        cx.notify();
                    }
                    AfterSessionWrite::Continue => self.finish_pending_continue(window, cx),
                }
            }
            Err(err) => {
                if after == AfterSessionWrite::Continue {
                    self.pending_continue = None;
                }
                self.show_save_error(&format!("{err:#}"), window, cx);
            }
        }
    }

    fn replace_session_from_path(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.cancel_outgoing_workflow(window, cx);
        let json = std::fs::read_to_string(path).map_err(|err| format!("{err:#}"))?;
        let loaded = crate::model::session::Session::from_json(&json, Some(path))
            .map_err(|err| format!("{err:#}"))?;
        self.teardown_all_documents(window, cx);
        let ui = loaded.ui;
        let first = loaded.session.documents().first().map(|doc| doc.id);
        let docs: Vec<DocumentId> = loaded
            .session
            .documents()
            .iter()
            .map(|doc| doc.id)
            .collect();
        self.session = loaded.session;
        self.restoring_session = true;
        for id in docs {
            self.attach_session_document(id, window, cx);
        }
        self.restoring_session = false;
        if let Some(id) = first {
            if self.session.get(id).is_some() {
                self.ensure_tab(id, window, cx);
            }
        }
        // ensure_tab → focus_document is a no-op when the first doc is already
        // session.active, so bind playback explicitly after restore.
        self.apply_active(window, cx);
        self.apply_session_ui(ui.as_ref(), window, cx);
        self.session.mark_clean();
        self.refresh_explorer(cx);
        self.explorer.update(cx, |explorer, cx| {
            explorer.sync_selection_to_active(cx);
        });
        self.update_window_title(window, cx);
        self.sync_view_menus(cx);
        self.fire_session_loaded_script(window, cx);
        self.resume_bound_workflow(window, cx);
        cx.notify();
        Ok(())
    }

    fn teardown_all_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<DocumentId> = self.session.documents().iter().map(|doc| doc.id).collect();
        for id in ids {
            if self.session.active() == Some(id) {
                self.commit_active_monitor_params(cx);
                self.stop_playback_into_active(cx);
            }
            if self.session.get(id).is_some_and(|doc| doc.tab_open) {
                if let Some(views) = self.views.get(&id) {
                    let workspace = views.workspace.clone();
                    self.dock_area.update(cx, |area, cx| {
                        area.remove_panel(workspace, window, cx);
                    });
                }
            }
            if let Some(views) = self.views.get(&id) {
                views.document.read(cx).progress.cancel();
            }
            self.views.remove(&id);
        }
        self.ensure_placeholder(window, cx);
    }

    fn attach_session_document(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.session.get(id).cloned() else {
            return;
        };
        let composition = Arc::new(RwLock::new(Composition::new(44100, 2)));
        let buffer = Arc::new(RwLock::new(Buffer::empty()));
        let app = cx.weak_entity();
        let views = Self::make_views(
            id,
            composition,
            buffer,
            self.waveform_representation,
            app,
            cx,
        );
        let workspace = views.workspace.clone();
        self.views.insert(id, views);
        if doc.tab_open {
            self.dock_area.update(cx, |area, cx| {
                Self::show_workspace_tab(area, workspace, None, window, cx);
            });
            self.remove_placeholder(window, cx);
        }
        if let Some(path) = doc.file_path().map(Path::to_path_buf) {
            self.spawn_document_load(id, path, window, cx);
        }
    }

    fn apply_session_ui(
        &mut self,
        ui: Option<&SessionUi>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session.capture_ui() {
            return;
        }
        let Some(ui) = ui else {
            return;
        };
        if let Some(bounds) = &ui.window {
            if bounds.width > 0.0 && bounds.height > 0.0 {
                window.resize(size(px(bounds.width), px(bounds.height)));
            }
        }
        if let Some(docks) = &ui.docks {
            self.set_explorer_dock(docks.explorer, window, cx);
            self.set_detail_dock(docks.detail, window, cx);
            if docks.script {
                self.show_script_dock(window, cx);
            } else {
                self.hide_script_dock(window, cx);
            }
        }
    }

    fn fire_session_loaded_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = crate::script::enter(self, window, cx);
        self.script.fire_session_loaded();
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
    }

    fn fire_session_saved_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = crate::script::enter(self, window, cx);
        self.script.fire_session_saved();
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
    }

    fn suspend_active_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let _guard = crate::script::enter(self, window, cx);
        let result = self.script.suspend_workflow();
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        match result {
            Ok(true) => true,
            Ok(false) => {
                self.script_alert(
                    "Cannot continue",
                    "The running workflow blocked this action.",
                    window,
                    cx,
                );
                false
            }
            Err(err) => {
                self.repl.update(cx, |repl, cx| {
                    repl.append_error(&format!("workflow suspend: {err}"), cx);
                });
                false
            }
        }
    }

    fn cancel_outgoing_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = crate::script::enter(self, window, cx);
        if let Err(err) = self.script.cancel_workflow() {
            self.repl.update(cx, |repl, cx| {
                repl.append_error(&format!("workflow cancel: {err}"), cx);
            });
        }
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
    }

    fn resume_bound_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = crate::script::enter(self, window, cx);
        let result = self.script.resume_workflow();
        let prints = self.script.take_prints();
        if !prints.is_empty() {
            let output = EvalOutput {
                prints,
                result: None,
                error: None,
            };
            self.repl.update(cx, |repl, cx| {
                repl.append_output(&to_repl_output(&output), cx);
            });
        }
        self.flush_script_logs(cx);
        match result {
            Ok(ResumeWorkflow::None | ResumeWorkflow::Resumed) => {}
            Ok(ResumeWorkflow::Unknown { name }) => {
                self.prompt_unknown_workflow(&name, window, cx);
            }
            Err(err) => {
                self.repl.update(cx, |repl, cx| {
                    repl.append_error(&format!("workflow resume: {err}"), cx);
                });
            }
        }
    }

    fn prompt_unknown_workflow(&self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let description = format!(
            "This session is bound to `{name}`, which is not a declared stateful workflow."
        );
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Unknown workflow")
                .description(description.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("workflow-keep")
                                .outline()
                                .label("Keep")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            this.refresh_workflow_bar(cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("workflow-clear")
                                .primary()
                                .label("Clear")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |this, cx| {
                                            this.script.clear_session_workflow();
                                            this.refresh_workflow_bar(cx);
                                        });
                                    }
                                }),
                        ),
                )
        });
    }

    fn prompt_open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    for path in paths {
                        this.open_path(path, window, cx);
                    }
                });
            });
        })
        .detach();
    }
}

impl AppView {
    fn app_key_context(&self, window: &mut Window, cx: &mut App) -> KeyContext {
        let mut context = KeyContext::parse("App").expect("App key context");
        let typing = window.focused_input(cx).is_some();
        let over_waveform = self
            .active_views()
            .is_some_and(|views| views.waveform.read(cx).pointer_over());
        if waveform_hover_key_context_active(over_waveform, typing) {
            context.add("WaveformHover");
        }
        context
    }
}

/// Whether App should include `WaveformHover` so Space (and a/f/m/d) work
/// without waveform focus — requires pointer_over to survive keypresses.
fn waveform_hover_key_context_active(pointer_over: bool, typing: bool) -> bool {
    pointer_over && !typing
}

#[cfg(test)]
mod waveform_hover_key_context_tests {
    use super::waveform_hover_key_context_active;

    #[test]
    fn space_play_pause_needs_persistent_waveform_hover() {
        // Hover path: first Space must not drop WaveformHover (issue #15).
        assert!(
            waveform_hover_key_context_active(true, false),
            "hovered waveform enables WaveformHover for Space"
        );
        assert!(
            !waveform_hover_key_context_active(false, false),
            "if hover clears after a keypress, Space stops matching"
        );
        assert!(
            !waveform_hover_key_context_active(true, true),
            "focused text input must not steal Space via WaveformHover"
        );
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let views = self.active_views();
        let file_status = views
            .as_ref()
            .and_then(|views| file_status_from_composition(&views.composition.read().unwrap()));
        let layout_picker = file_status.as_ref().map(|_| {
            let current = views.as_ref().and_then(|views| {
                views
                    .composition
                    .read()
                    .unwrap()
                    .channel_layout()
                    .map(str::to_string)
            });
            let choices = self.script.layout_choices();
            let app = cx.weak_entity();
            LayoutPicker {
                current,
                choices,
                on_choose: Rc::new(move |name, window, cx| {
                    let name = name.to_string();
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |this, cx| {
                            this.choose_channel_layout(&name, window, cx);
                        });
                    }
                }),
            }
        });
        let on_monitor = file_status.as_ref().map(|_| {
            let app = cx.weak_entity();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                if let Some(app) = app.upgrade() {
                    app.update(cx, |this, cx| {
                        this.toggle_monitor_tab(window, cx);
                    });
                }
            }) as Rc<dyn Fn(&mut Window, &mut App)>
        });
        let (error_count, warn_count) = {
            let panel = self.messages.read(cx);
            (panel.error_count(), panel.warn_count())
        };
        let on_messages = {
            let app = cx.weak_entity();
            Rc::new(move |shift: bool, window: &mut Window, cx: &mut App| {
                if let Some(app) = app.upgrade() {
                    app.update(cx, |this, cx| {
                        if shift {
                            this.clear_message_counts(cx);
                        } else {
                            this.toggle_messages_tab(window, cx);
                        }
                    });
                }
            }) as Rc<dyn Fn(bool, &mut Window, &mut App)>
        };
        let on_preview = {
            let app = cx.weak_entity();
            Rc::new(move |_window: &mut Window, cx: &mut App| {
                if let Some(app) = app.upgrade() {
                    app.update(cx, |this, cx| {
                        this.toggle_preview(cx);
                    });
                }
            }) as Rc<dyn Fn(&mut Window, &mut App)>
        };
        let progress_message = views.as_ref().and_then(|views| {
            views
                .document
                .read(cx)
                .progress
                .snapshot()
                .map(|state| state.message())
        });
        let explorer_open = self.explorer_dock_open(cx);
        let explorer_icon = if explorer_open {
            IconName::PanelLeft
        } else {
            IconName::PanelLeftOpen
        };
        let explorer_tooltip = if explorer_open {
            "Hide Explorer"
        } else {
            "Show Explorer"
        };
        let console_open = self.script_dock_open(cx);
        let console_icon = if console_open {
            IconName::PanelBottom
        } else {
            IconName::PanelBottomOpen
        };
        let console_tooltip = if console_open {
            "Hide Script"
        } else {
            "Show Script"
        };
        let detail_open = self.detail_dock_open(cx);
        let detail_icon = if detail_open {
            IconName::PanelRight
        } else {
            IconName::PanelRightOpen
        };
        let detail_tooltip = if detail_open {
            "Hide Detail"
        } else {
            "Show Detail"
        };

        let content_fg = content_foreground(cx);
        div()
            .id("app-view")
            .key_context(self.app_key_context(window, cx))
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .on_action(cx.listener(|this, _: &ToggleZeroCrossing, _, cx| {
                if let Some(views) = this.active_views() {
                    views.document.update(cx, |doc, _| {
                        doc.toggle_zero_crossing_snap();
                    });
                    cx.notify();
                }
            }))
            .child(
                v_flex()
                    .size_full()
                    .bg(theme.background)
                    .text_color(content_fg)
                    .when(
                        // Custom TitleBar reserves traffic-light height even when
                        // AppKit hides the system chrome in fullscreen — omit it.
                        !window.is_fullscreen() && !window.is_simple_fullscreen(),
                        |this| {
                            this.child(
                                TitleBar::new().child(
                                    h_flex()
                                        .id("app-title-bar-leading")
                                        .h_full()
                                        .items_center()
                                        .gap_2()
                                        .when(!cfg!(target_os = "macos"), |this| {
                                            this.child(
                                                img("icons/app-mark.svg").size(px(16.)).flex_none(),
                                            )
                                        })
                                        .when_some(self.app_menu_bar.clone(), |this, menu_bar| {
                                            this.child(menu_bar)
                                        }),
                                ),
                            )
                        },
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .child(
                                v_flex()
                                    .size_full()
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .flex_none()
                                            .px_3()
                                            .py_2()
                                            .gap_2()
                                            .items_center()
                                            .border_b_1()
                                            .border_color(theme.border)
                                            .bg(theme.title_bar)
                                            .child(
                                                Button::new("toggle-explorer-dock")
                                                    .ghost()
                                                    .small()
                                                    .text_color(theme.muted_foreground)
                                                    .icon(explorer_icon)
                                                    .tooltip(explorer_tooltip)
                                                    .selected(explorer_open)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.toggle_explorer_dock(window, cx);
                                                        },
                                                    )),
                                            )
                                            .child(self.header_meta.clone())
                                            .child(
                                                Button::new("toggle-script-dock")
                                                    .ghost()
                                                    .small()
                                                    .text_color(theme.muted_foreground)
                                                    .icon(console_icon)
                                                    .tooltip(console_tooltip)
                                                    .selected(console_open)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.toggle_script_dock(window, cx);
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("toggle-detail-dock")
                                                    .ghost()
                                                    .small()
                                                    .text_color(theme.muted_foreground)
                                                    .icon(detail_icon)
                                                    .tooltip(detail_tooltip)
                                                    .selected(detail_open)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.toggle_detail_dock(window, cx);
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .w_full()
                                            .child(self.dock_area.clone()),
                                    )
                                    .when(self.workflow_bar.is_some(), |this| {
                                        this.child(self.workflow_bar_view.clone())
                                    })
                                    .child(
                                        SessionStatusBar::new(file_status)
                                            .with_progress_message(progress_message)
                                            .with_preview(Some(on_preview))
                                            .with_preview_selected(self.preview_enabled)
                                            .with_message_alerts(
                                                error_count,
                                                warn_count,
                                                on_messages,
                                            )
                                            .with_monitor(on_monitor)
                                            .with_monitor_selected(self.monitor_tab_visible(cx))
                                            .with_monitor_faulted(!self.playback.output_active())
                                            .with_layout(layout_picker),
                                    ),
                            )
                            .when(self.render_sheet_open, |this| {
                                this.child(self.render_sheet_overlay(cx))
                            }),
                    ),
            )
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
    }
}

impl Focusable for AppView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AppView {
    fn sync_playback_to_document(&mut self, cx: &mut Context<Self>) {
        if let Some(views) = self.active_views() {
            views.document.update(cx, |doc, cx| {
                self.playback.sync_document_from_playback(doc);
                cx.notify();
            });
        }
        let transport = self.playback.transport_state();
        self.header_meta.update(cx, |meta, cx| {
            meta.set_transport(transport, cx);
        });
    }
}

pub(crate) fn dispatch_command(command_id: &str, cx: &mut App) -> Result<(), String> {
    if command_id == "help.about" {
        open_about_window(cx);
        return Ok(());
    }

    if let Some((view, window)) = living_editor_window(cx) {
        let command_id = command_id.to_string();
        // Menu and key handlers run inside an update. Defer so path prompts and
        // nested view updates are not attempted on the same tick.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |this, cx| this.invoke_command(&command_id, window, cx))
            });
        });
        return Ok(());
    }

    match command_id {
        "file.quit" => {
            cx.quit();
            Ok(())
        }
        "file.open" => {
            prompt_open_at_app_level(cx);
            Ok(())
        }
        _ => Err("no active window".into()),
    }
}

fn clear_editor_globals(cx: &mut App) {
    if cx.has_global::<OpenTarget>() {
        let _ = cx.remove_global::<OpenTarget>();
    }
    if cx.has_global::<EditorWindow>() {
        let _ = cx.remove_global::<EditorWindow>();
    }
}

fn window_is_open(cx: &App, handle: AnyWindowHandle) -> bool {
    cx.windows().iter().any(|window| *window == handle)
}

fn living_editor_window(cx: &mut App) -> Option<(Entity<AppView>, AnyWindowHandle)> {
    let view = cx.try_global::<OpenTarget>().map(|target| target.0.clone());
    let handle = cx.try_global::<EditorWindow>().map(|editor| editor.0);
    match (view, handle) {
        (Some(view), Some(handle)) => Some((view, handle)),
        (None, None) => None,
        // Partial pair: leave globals alone. Teardown is `on_app_window_closed`.
        _ => None,
    }
}

fn on_app_window_closed(cx: &mut App, id: WindowId) {
    if let Some(EditorWindow(handle)) = cx.try_global::<EditorWindow>().copied() {
        if handle.window_id() == id {
            clear_editor_globals(cx);
            refresh_menus_for_editor_presence(cx);
        }
    }
    if let Some(AboutWindow(handle)) = cx.try_global::<AboutWindow>().copied() {
        if handle.window_id() == id {
            let _ = cx.remove_global::<AboutWindow>();
        }
    }
}

fn open_about_window(cx: &mut App) {
    if let Some(AboutWindow(handle)) = cx.try_global::<AboutWindow>().copied() {
        if window_is_open(cx, handle) {
            // Activate if we can; a nested-update Err must not open a second About.
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
            return;
        }
        let _ = cx.remove_global::<AboutWindow>();
    }

    // Native titlebar: movable, with system close/miniaturize controls. Do not use
    // TitleBar::window_options (transparent + app-owned drag) without drawing TitleBar.
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from(format!("About {}", crate::APP_NAME))),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(200.), px(200.)),
            size: size(px(480.), px(300.)),
        })),
        app_owns_titlebar_drag: false,
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Server),
        ..Default::default()
    };

    match cx.open_window(options, |window, cx| {
        let view = cx.new(AboutView::new);
        window.focus(&view.focus_handle(cx), cx);
        cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
    }) {
        Ok(handle) => {
            cx.set_global(AboutWindow(handle.into()));
        }
        Err(err) => {
            eprintln!("failed to open About window: {err}");
        }
    }
}

fn prompt_open_at_app_level(cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Open".into()),
    });
    cx.spawn(async move |cx| {
        let paths = match receiver.await {
            Ok(Ok(Some(paths))) => paths,
            _ => return,
        };
        cx.update(|cx| {
            open_main_window(cx, paths);
        });
    })
    .detach();
}

fn take_pending_open_paths(cx: &App) -> Vec<PathBuf> {
    cx.try_global::<AppLaunchState>()
        .map(|launch| std::mem::take(&mut *launch.pending_opens.lock().unwrap()))
        .unwrap_or_default()
}

fn push_pending_open_paths(cx: &App, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    if let Some(launch) = cx.try_global::<AppLaunchState>() {
        launch.pending_opens.lock().unwrap().extend(paths);
    }
}

fn flush_pending_opens_into_editor(cx: &mut App) {
    if living_editor_window(cx).is_none() {
        let ready = cx.try_global::<AppLaunchState>().is_some_and(|launch| {
            launch
                .launch_ui_ready
                .load(std::sync::atomic::Ordering::SeqCst)
        });
        if !ready {
            #[cfg(target_os = "macos")]
            crate::macos_open::log_open(
                "flush: skip (no editor yet, launch not ready — leave pending)",
            );
            return;
        }
        #[cfg(target_os = "macos")]
        crate::macos_open::log_open("flush: no editor but launch ready → open_main_window");
        open_main_window(cx, vec![]);
        return;
    }

    let paths = take_pending_open_paths(cx);
    if paths.is_empty() {
        return;
    }
    #[cfg(target_os = "macos")]
    crate::macos_open::log_open(&format!(
        "flush_pending_opens_into_editor {} path(s)",
        paths.len()
    ));
    let Some((view, handle)) = living_editor_window(cx) else {
        push_pending_open_paths(cx, paths);
        return;
    };
    let paths_for_retry = paths.clone();
    match handle.update(cx, |_, window, cx| {
        view.update(cx, |this, cx| {
            for path in paths {
                #[cfg(target_os = "macos")]
                crate::macos_open::log_open(&format!("flush→open_path {}", path.display()));
                this.open_path(path, window, cx);
            }
        });
    }) {
        Ok(()) => {}
        Err(err) => {
            #[cfg(target_os = "macos")]
            crate::macos_open::log_open(&format!("flush: handle.update failed ({err}); re-queue"));
            push_pending_open_paths(cx, paths_for_retry);
        }
    }
}

fn mark_launch_ui_ready(cx: &mut App) {
    if let Some(launch) = cx.try_global::<AppLaunchState>() {
        launch
            .launch_ui_ready
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn open_main_window(cx: &mut App, paths: Vec<PathBuf>) {
    open_main_window_seeded(cx, paths, MainWindowSeed::Empty);
}

fn open_main_window_seeded(cx: &mut App, paths: Vec<PathBuf>, seed: MainWindowSeed) {
    // Keep Finder / Dock paths in the shared pending queue. Taking them into a
    // local vec and only opening via `handle.update` after `open_window` was
    // dropping cold-launch opens when that update did not stick; AppView's
    // drain (and an explicit flush below) are the reliable delivery path.
    push_pending_open_paths(cx, paths);

    if let Some((_, handle)) = living_editor_window(cx) {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
            flush_pending_opens_into_editor(cx);
        });
        return;
    }

    let Some((output_spec, pending_opens)) = cx.try_global::<AppLaunchState>().map(|launch| {
        (
            launch.output_device_spec.clone(),
            launch.pending_opens.clone(),
        )
    }) else {
        // Launch callback has not stored AppLaunchState yet (e.g. early reopen).
        return;
    };
    let (device, output_device) = match resolve_output_device(output_spec.as_deref()) {
        Ok(device) => {
            let name = output_spec.as_ref().map(|_| output_device_name(&device));
            (Some(device), name)
        }
        Err(err) => {
            eprintln!(
                "FieldAssist: failed to resolve output audio device ({err}); \
                 continuing with playback disabled"
            );
            (None, None)
        }
    };

    let (composition, source_path, load_elapsed, session_path) = match seed {
        MainWindowSeed::Empty => (Composition::new(44100, 2), None, None, None),
        MainWindowSeed::Preloaded {
            composition,
            load_elapsed,
        } => {
            let source_path = composition.pool().first().map(|media| media.path.clone());
            (composition, source_path, load_elapsed, None)
        }
        MainWindowSeed::Session(path) => (Composition::new(44100, 2), None, None, Some(path)),
    };

    let title = AppView::composition_title(&composition);
    let shared_composition = Arc::new(RwLock::new(composition));
    let shared_buffer = Arc::new(RwLock::new(Buffer::empty()));
    let playback = match &device {
        Some(device) => PlaybackSession::open(device, shared_composition.clone()),
        None => PlaybackSession::disabled(shared_composition.clone()),
    };
    let playback = match playback {
        Ok(playback) => playback,
        Err(err) => {
            eprintln!("FieldAssist: failed to create playback session: {err}");
            return;
        }
    };

    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.clone()),
            ..TitleBar::title_bar_options()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(80.), px(80.)),
            size: size(px(1280.), px(760.)),
        })),
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Client),
        ..TitleBar::window_options()
    };

    let handle = match cx.open_window(options, move |window, cx| {
        let view = cx.new(|cx| {
            AppView::new(
                shared_composition.clone(),
                shared_buffer.clone(),
                source_path.clone(),
                load_elapsed,
                playback,
                output_device,
                pending_opens.clone(),
                session_path.clone(),
                window,
                cx,
            )
        });
        cx.set_global(OpenTarget(view.clone()));
        let closer = view.clone();
        let pinner = view.clone();
        let pinned = view.clone();
        let close_all = view.clone();
        let close_saved = view.clone();
        cx.set_global(CenterTabBarHandler {
            close: Rc::new(move |panel_id, window, cx| {
                closer.update(cx, |this, cx| {
                    this.close_center_panel(panel_id, window, cx);
                });
            }),
            pin: Rc::new(move |panel_id, _, cx| {
                pinner.update(cx, |this, cx| {
                    this.pin_center_panel(panel_id, cx);
                });
            }),
            is_pinned: Rc::new(move |panel_id, cx| pinned.read(cx).panel_is_pinned(panel_id)),
            close_all: Rc::new(move |window, cx| {
                close_all.update(cx, |this, cx| {
                    this.close_all_tabs(window, cx);
                });
            }),
            close_saved: Rc::new(move |window, cx| {
                close_saved.update(cx, |this, cx| {
                    this.close_saved_tabs(window, cx);
                });
            }),
        });
        // Pull any straggler macos_open queue into the shared pending Arc, then
        // open inside this callback while `window` + `AppView` are live.
        #[cfg(target_os = "macos")]
        {
            pending_opens
                .lock()
                .unwrap()
                .extend(crate::macos_open::take_queued_paths());
        }
        let pending_count = pending_opens.lock().unwrap().len();
        #[cfg(target_os = "macos")]
        crate::macos_open::log_open(&format!(
            "open_window build: draining {pending_count} pending path(s)"
        ));
        view.update(cx, |this, cx| {
            this.drain_pending_opens(window, cx);
        });
        window.focus(&view.focus_handle(cx), cx);
        cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
    }) {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("failed to open window: {err}");
            return;
        }
    };

    cx.set_global(EditorWindow(handle.into()));
    refresh_menus_for_editor_presence(cx);

    // Stragglers that arrive after the window is up (Dock drop while running
    // uses open_handler → flush).
    cx.defer(|cx| {
        flush_pending_opens_into_editor(cx);
    });
}

fn update_open_view(
    cx: &mut App,
    f: impl FnOnce(&mut AppView, &mut Window, &mut Context<AppView>) + 'static,
) {
    let Some((view, window)) = living_editor_window(cx) else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            view.update(cx, |this, cx| f(this, window, cx));
        });
    });
}

fn quit(_: &Quit, cx: &mut App) {
    let _ = crate::commands::dispatch("file.quit", cx);
}

fn settings(_: &Settings, _cx: &mut App) {}

fn open(_: &Open, cx: &mut App) {
    let _ = crate::commands::dispatch("file.open", cx);
}

fn save(_: &Save, cx: &mut App) {
    let _ = crate::commands::dispatch("file.save", cx);
}

fn save_as(_: &SaveAs, cx: &mut App) {
    let _ = crate::commands::dispatch("file.save_as", cx);
}

fn save_session(_: &SaveSession, cx: &mut App) {
    let _ = crate::commands::dispatch("file.save_session", cx);
}

fn save_session_as(_: &SaveSessionAs, cx: &mut App) {
    let _ = crate::commands::dispatch("file.save_session_as", cx);
}

fn close(_: &Close, cx: &mut App) {
    let _ = crate::commands::dispatch("file.close", cx);
}

fn render_cmd(_: &RenderFile, cx: &mut App) {
    let _ = crate::commands::dispatch("file.render", cx);
}

fn about(_: &About, cx: &mut App) {
    let _ = crate::commands::dispatch("help.about", cx);
}

fn transport_home(_: &TransportHome, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.home", cx);
}

fn transport_previous(_: &TransportPrevious, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.previous", cx);
}

fn transport_start(_: &TransportStart, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.start", cx);
}

fn transport_play_pause(_: &TransportPlayPause, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.play_pause", cx);
}

fn transport_stop(_: &TransportStop, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.stop", cx);
}

fn transport_next(_: &TransportNext, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.next", cx);
}

fn transport_end(_: &TransportEnd, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.end", cx);
}

fn transport_loop(_: &TransportLoop, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.loop", cx);
}

fn transport_preview(_: &TransportPreview, cx: &mut App) {
    let _ = crate::commands::dispatch("transport.preview", cx);
}

fn view_fit_all(_: &ViewFitAll, cx: &mut App) {
    let _ = crate::commands::dispatch("view.fit_all", cx);
}

fn view_frame(_: &ViewFrame, cx: &mut App) {
    let _ = crate::commands::dispatch("view.frame", cx);
}

fn view_zoom_in(_: &ViewZoomIn, cx: &mut App) {
    let _ = crate::commands::dispatch("view.zoom_in", cx);
}

fn view_zoom_out(_: &ViewZoomOut, cx: &mut App) {
    let _ = crate::commands::dispatch("view.zoom_out", cx);
}

fn view_explorer(_: &ViewExplorer, cx: &mut App) {
    let _ = crate::commands::dispatch("view.toggle-explorer", cx);
}

fn view_show_explorer(_: &ViewShowExplorer, cx: &mut App) {
    let _ = crate::commands::dispatch("view.show-explorer", cx);
}

fn view_hide_explorer(_: &ViewHideExplorer, cx: &mut App) {
    let _ = crate::commands::dispatch("view.hide-explorer", cx);
}

fn view_detail(_: &ViewDetail, cx: &mut App) {
    let _ = crate::commands::dispatch("view.toggle-detail", cx);
}

fn view_show_detail(_: &ViewShowDetail, cx: &mut App) {
    let _ = crate::commands::dispatch("view.show-detail", cx);
}

fn view_hide_detail(_: &ViewHideDetail, cx: &mut App) {
    let _ = crate::commands::dispatch("view.hide-detail", cx);
}

fn view_script(_: &ViewScript, cx: &mut App) {
    let _ = crate::commands::dispatch("view.toggle-script", cx);
}

fn view_show_script(_: &ViewShowScript, cx: &mut App) {
    let _ = crate::commands::dispatch("view.show-script", cx);
}

fn view_hide_script(_: &ViewHideScript, cx: &mut App) {
    let _ = crate::commands::dispatch("view.hide-script", cx);
}

fn view_overlay_envelope_peak(_: &ViewOverlayEnvelopePeak, cx: &mut App) {
    let _ = crate::commands::dispatch("view.overlay_envelope_peak", cx);
}

fn view_waveform_peaks(_: &ViewWaveformPeaks, cx: &mut App) {
    let _ = crate::commands::dispatch("view.waveform_peaks", cx);
}

fn view_waveform_spectrum(_: &ViewWaveformSpectrum, cx: &mut App) {
    let _ = crate::commands::dispatch("view.waveform_spectrum", cx);
}

fn view_waveform_peaks_spectrum(_: &ViewWaveformPeaksSpectrum, cx: &mut App) {
    let _ = crate::commands::dispatch("view.waveform_peaks_spectrum", cx);
}

fn analyze_envelope_peak(_: &AnalyzeEnvelopePeak, cx: &mut App) {
    let _ = crate::commands::dispatch("analyze.envelope_peak", cx);
}

fn analyze_selection_only(_: &AnalyzeSelectionOnly, cx: &mut App) {
    let _ = crate::commands::dispatch("analyze.selection_only", cx);
}

fn analyze_transients(_: &AnalyzeTransients, cx: &mut App) {
    let _ = crate::commands::dispatch("analyze.transients", cx);
}

fn edit_undo(_: &EditUndo, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.undo", cx);
}

fn edit_redo(_: &EditRedo, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.redo", cx);
}

fn edit_cut(_: &EditCut, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.cut", cx);
}

fn edit_copy(_: &EditCopy, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.copy", cx);
}

fn edit_paste(_: &EditPaste, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.paste", cx);
}

fn edit_clear(_: &EditClear, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.clear", cx);
}

fn edit_remove(_: &EditRemove, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.remove", cx);
}

fn edit_duplicate(_: &EditDuplicate, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.duplicate", cx);
}

fn edit_trim(_: &EditTrim, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.trim", cx);
}

fn edit_break_out(_: &EditBreakOut, cx: &mut App) {
    let _ = crate::commands::dispatch("edit.break_out", cx);
}

fn select_all(_: &SelectAll, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.select_all", cx);
}

fn select_none(_: &SelectNone, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.select_none", cx);
}

fn invert_selection(_: &InvertSelection, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.invert", cx);
}

fn marker_type_blue(_: &MarkerTypeBlue, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.marker_type_blue", cx);
}

fn marker_type_yellow(_: &MarkerTypeYellow, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.marker_type_yellow", cx);
}

fn marker_type_purple(_: &MarkerTypePurple, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.marker_type_purple", cx);
}

fn snap_to_marker(_: &SnapToMarker, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.snap_to_marker", cx);
}

fn set_active_marker_type_action(action: &SetActiveMarkerType, cx: &mut App) {
    let name = action.name.clone();
    update_open_view(cx, move |this, _, cx| {
        this.set_active_marker_type(&name, cx);
    });
}

fn toggle_snap_marker_type_action(action: &ToggleSnapMarkerType, cx: &mut App) {
    let name = action.name.clone();
    update_open_view(cx, move |this, _, cx| {
        this.update_active_document(cx, |doc| doc.toggle_snap_marker_type(&name));
        this.sync_view_menus(cx);
    });
}

fn add_marker_at_hover(_: &AddMarkerAtHover, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.add_at_hover", cx);
}

fn add_marker(_: &AddMarker, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.add_marker", cx);
}

fn delete_marker(_: &DeleteMarker, cx: &mut App) {
    let _ = crate::commands::dispatch("selection.delete_marker", cx);
}

fn cancel_workflow_action(_: &CancelWorkflow, cx: &mut App) {
    update_open_view(cx, |this, window, cx| {
        this.cancel_outgoing_workflow(window, cx);
    });
}

fn start_workflow_action(action: &StartWorkflow, cx: &mut App) {
    let name = action.name.clone();
    update_open_view(cx, move |this, window, cx| {
        this.invoke_menu_workflow(&name, window, cx);
    });
}

struct AppMenuState {
    /// False when macOS is running with no editor window (menus stay visible).
    editor_open: bool,
    explorer: bool,
    detail: bool,
    script: bool,
    envelope_overlay: bool,
    waveform_representation: field_ui_components::WaveformRepresentation,
    analyze_selection_only: bool,
    marker_type: String,
    add_at_hover: bool,
    snap_to_marker: bool,
    marker_types: Vec<String>,
    snap_disabled: HashSet<String>,
    workflow_running: bool,
    menu_workflows: Vec<(String, String)>,
}

/// View → Peaks / Spectrum / Peaks + Spectrum: exclusive radio marks (app-global).
///
/// Windows/Linux paint a Lucide `dot` in the PopupMenu gutter via
/// [`AppMenuBar`]. On macOS, `.checked(true)` still drives the state column;
/// [`macos_menu::apply_after_set_menus`] replaces the system checkmark
/// with a filled-circle so labels stay aligned with other View items.
fn waveform_representation_item(
    label: &'static str,
    action: impl gpui_kit::Action,
    selected: bool,
) -> MenuItem {
    MenuItem::action(label, action).checked(selected)
}

fn marker_type_menu_item(name: &str, active: &str) -> MenuItem {
    let checked = active == name;
    match name {
        MARKER_TYPE_BLUE => MenuItem::action("Blue", MarkerTypeBlue).checked(checked),
        MARKER_TYPE_YELLOW => MenuItem::action("Yellow", MarkerTypeYellow).checked(checked),
        MARKER_TYPE_PURPLE => MenuItem::action("Purple", MarkerTypePurple).checked(checked),
        other => MenuItem::action(
            other.to_string(),
            SetActiveMarkerType {
                name: other.to_string(),
            },
        )
        .checked(checked),
    }
}

fn snap_marker_type_menu_item(name: &str, disabled: &HashSet<String>) -> MenuItem {
    MenuItem::action(
        name.to_string(),
        ToggleSnapMarkerType {
            name: name.to_string(),
        },
    )
    .checked(!disabled.contains(name))
}

fn no_editor_menu_state() -> AppMenuState {
    AppMenuState {
        editor_open: false,
        explorer: false,
        detail: false,
        script: false,
        envelope_overlay: false,
        waveform_representation: field_ui_components::WaveformRepresentation::Peaks,
        analyze_selection_only: false,
        marker_type: default_marker_type().to_string(),
        add_at_hover: true,
        snap_to_marker: false,
        marker_types: DEFAULT_MARKER_TYPES
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect(),
        snap_disabled: HashSet::new(),
        workflow_running: false,
        menu_workflows: Vec::new(),
    }
}

/// Disable when there is no editor window. App menu + File → Open stay enabled.
fn needs_editor(item: MenuItem, editor_open: bool) -> MenuItem {
    item.disabled(!editor_open)
}

fn app_menus(state: &AppMenuState) -> Vec<Menu> {
    let open = state.editor_open;
    let create_type_items: Vec<MenuItem> = state
        .marker_types
        .iter()
        .map(|name| needs_editor(marker_type_menu_item(name, &state.marker_type), open))
        .collect();
    let snap_type_items: Vec<MenuItem> = state
        .marker_types
        .iter()
        .map(|name| needs_editor(snap_marker_type_menu_item(name, &state.snap_disabled), open))
        .collect();

    let mut file_items = vec![
        MenuItem::action("Open...", Open),
        MenuItem::separator(),
        needs_editor(MenuItem::action("Save", Save), open),
        needs_editor(MenuItem::action("Save As...", SaveAs), open),
        needs_editor(MenuItem::action("Close", Close), open),
        MenuItem::separator(),
        needs_editor(MenuItem::action("Render...", RenderFile), open),
        MenuItem::separator(),
        needs_editor(MenuItem::action("Save Session", SaveSession), open),
        needs_editor(MenuItem::action("Save Session As...", SaveSessionAs), open),
    ];
    if !cfg!(target_os = "macos") {
        file_items.push(MenuItem::separator());
        file_items.push(MenuItem::action("Quit", Quit));
    }

    let mut edit_items = vec![
        needs_editor(MenuItem::action("Undo", EditUndo), open),
        needs_editor(MenuItem::action("Redo", EditRedo), open),
        MenuItem::separator(),
        needs_editor(MenuItem::action("Cut", EditCut), open),
        needs_editor(MenuItem::action("Copy", EditCopy), open),
        needs_editor(MenuItem::action("Paste", EditPaste), open),
        MenuItem::separator(),
        needs_editor(MenuItem::action("Clear", EditClear), open),
        needs_editor(MenuItem::action("Remove", EditRemove), open),
        needs_editor(MenuItem::action("Duplicate", EditDuplicate), open),
        needs_editor(MenuItem::action("Trim to Selection", EditTrim), open),
        needs_editor(
            MenuItem::action("Break Out to Composition", EditBreakOut),
            open,
        ),
    ];
    if !cfg!(target_os = "macos") {
        edit_items.push(MenuItem::separator());
        edit_items.push(MenuItem::action("Settings...", Settings).disabled(true));
    }

    let mut menus = Vec::new();
    if cfg!(target_os = "macos") {
        // App menu stays fully available with no editor (About / Quit).
        menus.push(Menu::new(crate::APP_NAME).items([
            MenuItem::action("About...", About),
            MenuItem::separator(),
            MenuItem::action("Settings...", Settings).disabled(true),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ]));
    }
    menus.push(Menu::new("File").items(file_items));
    menus.push(Menu::new("Edit").items(edit_items));
    menus.push(Menu::new("Selection").items([
        needs_editor(MenuItem::action("Select All", SelectAll), open),
        needs_editor(MenuItem::action("Select None", SelectNone), open),
        needs_editor(MenuItem::action("Invert", InvertSelection), open),
        MenuItem::separator(),
        needs_editor(
            MenuItem::action("Snap To Marker", SnapToMarker).checked(state.snap_to_marker),
            open,
        ),
        needs_editor(
            MenuItem::submenu(Menu::new("Snap Marker Type").items(snap_type_items)),
            open,
        ),
        MenuItem::separator(),
        needs_editor(
            MenuItem::submenu(Menu::new("Create Marker Type").items(create_type_items)),
            open,
        ),
        needs_editor(
            MenuItem::action("Add at Hover", AddMarkerAtHover).checked(state.add_at_hover),
            open,
        ),
        needs_editor(MenuItem::action("Add Marker", AddMarker), open),
        needs_editor(MenuItem::action("Delete Marker", DeleteMarker), open),
    ]));
    menus.push(
        Menu::new("View").items([
            needs_editor(
                MenuItem::action("Show Explorer", ViewExplorer).checked(state.explorer),
                open,
            ),
            needs_editor(
                MenuItem::action("Show Detail", ViewDetail).checked(state.detail),
                open,
            ),
            needs_editor(
                MenuItem::action("Show Script", ViewScript).checked(state.script),
                open,
            ),
            MenuItem::separator(),
            needs_editor(
                waveform_representation_item(
                    "Peaks",
                    ViewWaveformPeaks,
                    matches!(
                        state.waveform_representation,
                        field_ui_components::WaveformRepresentation::Peaks
                    ),
                ),
                open,
            ),
            needs_editor(
                waveform_representation_item(
                    "Spectrum",
                    ViewWaveformSpectrum,
                    matches!(
                        state.waveform_representation,
                        field_ui_components::WaveformRepresentation::Spectrum
                    ),
                ),
                open,
            ),
            needs_editor(
                waveform_representation_item(
                    "Peaks + Spectrum",
                    ViewWaveformPeaksSpectrum,
                    matches!(
                        state.waveform_representation,
                        field_ui_components::WaveformRepresentation::PeaksSpectrum
                    ),
                ),
                open,
            ),
            MenuItem::separator(),
            needs_editor(
                MenuItem::submenu(
                    Menu::new("Overlay").items([MenuItem::action(
                        "Envelope Peak",
                        ViewOverlayEnvelopePeak,
                    )
                    .checked(state.envelope_overlay)]),
                ),
                open,
            ),
            MenuItem::separator(),
            needs_editor(MenuItem::action("Zoom In", ViewZoomIn), open),
            needs_editor(MenuItem::action("Zoom Out", ViewZoomOut), open),
            needs_editor(MenuItem::action("Reset View", ViewFitAll), open),
        ]),
    );
    menus.push(
        Menu::new("Analyze").items([
            needs_editor(
                MenuItem::action("Selection Only", AnalyzeSelectionOnly)
                    .checked(state.analyze_selection_only),
                open,
            ),
            MenuItem::separator(),
            needs_editor(
                MenuItem::submenu(
                    Menu::new("Envelope").items([MenuItem::action("Peak", AnalyzeEnvelopePeak)]),
                ),
                open,
            ),
            needs_editor(
                MenuItem::submenu(
                    Menu::new("Mark").items([MenuItem::action("Transients", AnalyzeTransients)]),
                ),
                open,
            ),
        ]),
    );
    menus.push(workflow_menu(state));
    if !cfg!(target_os = "macos") {
        menus.push(Menu::new("Help").items([MenuItem::action("About...", About)]));
    }
    menus
}

fn workflow_menu(state: &AppMenuState) -> Menu {
    let open = state.editor_open;
    let mut items = vec![
        MenuItem::action("Cancel", CancelWorkflow).disabled(!state.workflow_running || !open),
        MenuItem::separator(),
    ];
    for (name, display_name) in &state.menu_workflows {
        items.push(needs_editor(
            MenuItem::action(display_name.clone(), StartWorkflow { name: name.clone() }),
            open,
        ));
    }
    Menu::new("Workflow").items(items)
}

fn apply_muted_chrome(cx: &mut App) {
    let muted = Theme::global(cx).muted_foreground;
    let content = Theme::global(cx).foreground;
    cx.set_global(ContentForeground(content));
    let theme = Theme::global_mut(cx);
    theme.tab_foreground = muted;
    theme.tab_active_foreground = muted;
    // Submenu titles use MenuItemElement's foreground; match the muted chrome
    // used by custom-rendered menu items.
    theme.foreground = muted;
    Theme::sync_base(cx);
}

fn apply_app_menus(state: &AppMenuState, cx: &mut App) {
    cx.set_menus(app_menus(state));
    #[cfg(target_os = "macos")]
    crate::macos_menu::apply_after_set_menus();
    let owned = app_menus(state)
        .into_iter()
        .map(|menu| menu.owned())
        .collect();
    GlobalState::global_mut(cx).set_app_menus(owned);
}

fn refresh_menus_for_editor_presence(cx: &mut App) {
    if let Some((view, _)) = living_editor_window(cx) {
        let state = view.read(cx).app_menu_state(cx);
        apply_app_menus(&state, cx);
        if let Some(bar) = view.read(cx).app_menu_bar.clone() {
            bar.update(cx, |bar, cx| bar.reload(cx));
        }
    } else {
        apply_app_menus(&no_editor_menu_state(), cx);
    }
}

fn install_app_menu(cx: &mut App) {
    cx.on_action(open);
    cx.on_action(save);
    cx.on_action(save_as);
    cx.on_action(save_session);
    cx.on_action(save_session_as);
    cx.on_action(close);
    cx.on_action(render_cmd);
    cx.on_action(quit);
    cx.on_action(settings);
    cx.on_action(about);
    cx.on_action(transport_home);
    cx.on_action(transport_previous);
    cx.on_action(transport_start);
    cx.on_action(transport_play_pause);
    cx.on_action(transport_stop);
    cx.on_action(transport_next);
    cx.on_action(transport_end);
    cx.on_action(transport_loop);
    cx.on_action(transport_preview);
    cx.on_action(view_fit_all);
    cx.on_action(view_frame);
    cx.on_action(view_zoom_in);
    cx.on_action(view_zoom_out);
    cx.on_action(view_explorer);
    cx.on_action(view_show_explorer);
    cx.on_action(view_hide_explorer);
    cx.on_action(view_detail);
    cx.on_action(view_show_detail);
    cx.on_action(view_hide_detail);
    cx.on_action(view_script);
    cx.on_action(view_show_script);
    cx.on_action(view_hide_script);
    cx.on_action(view_overlay_envelope_peak);
    cx.on_action(view_waveform_peaks);
    cx.on_action(view_waveform_spectrum);
    cx.on_action(view_waveform_peaks_spectrum);
    cx.on_action(analyze_selection_only);
    cx.on_action(analyze_envelope_peak);
    cx.on_action(analyze_transients);
    cx.on_action(edit_undo);
    cx.on_action(edit_redo);
    cx.on_action(edit_cut);
    cx.on_action(edit_copy);
    cx.on_action(edit_paste);
    cx.on_action(edit_clear);
    cx.on_action(edit_remove);
    cx.on_action(edit_duplicate);
    cx.on_action(edit_trim);
    cx.on_action(edit_break_out);
    cx.on_action(select_all);
    cx.on_action(select_none);
    cx.on_action(invert_selection);
    cx.on_action(marker_type_blue);
    cx.on_action(marker_type_yellow);
    cx.on_action(marker_type_purple);
    cx.on_action(snap_to_marker);
    cx.on_action(set_active_marker_type_action);
    cx.on_action(toggle_snap_marker_type_action);
    cx.on_action(add_marker_at_hover);
    cx.on_action(add_marker);
    cx.on_action(delete_marker);
    cx.on_action(cancel_workflow_action);
    cx.on_action(start_workflow_action);
    install_keybindings(cx);
    apply_app_menus(&no_editor_menu_state(), cx);
    cx.activate(true);
}

pub(crate) fn path_from_open_url(url: &str) -> Option<PathBuf> {
    // Finder / Launch Services may send file URLs with a host (`localhost`) or
    // percent-encoded path segments. Strip query/fragment before decoding.
    let url = url.split_once('#').map(|(head, _)| head).unwrap_or(url);
    let url = url.split_once('?').map(|(head, _)| head).unwrap_or(url);
    let decoded = if let Some(rest) = url.strip_prefix("file://") {
        let path = if rest.starts_with('/') {
            rest
        } else {
            // file://localhost/Users/... or file://hostname/path
            let slash = rest.find('/')?;
            &rest[slash..]
        };
        percent_decode(path)?
    } else if url.starts_with('/') {
        url.to_owned()
    } else {
        return None;
    };
    let path = PathBuf::from(decoded);
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(path)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod open_url_tests {
    use super::path_from_open_url;
    use std::path::PathBuf;

    #[test]
    fn parses_plain_file_url() {
        assert_eq!(
            path_from_open_url("file:///Users/me/take.wav"),
            Some(PathBuf::from("/Users/me/take.wav"))
        );
    }

    #[test]
    fn parses_localhost_file_url_and_encoding() {
        assert_eq!(
            path_from_open_url("file://localhost/Users/me/my%20take.wav"),
            Some(PathBuf::from("/Users/me/my take.wav"))
        );
    }

    #[test]
    fn strips_query_and_fragment() {
        assert_eq!(
            path_from_open_url("file:///tmp/a.wav?x=1#frag"),
            Some(PathBuf::from("/tmp/a.wav"))
        );
    }

    #[test]
    fn accepts_bare_unix_path() {
        assert_eq!(
            path_from_open_url("/tmp/a.wav"),
            Some(PathBuf::from("/tmp/a.wav"))
        );
    }
}

pub fn run(
    initial: Option<Composition>,
    load_elapsed: Option<f64>,
    _device: Device,
    output_spec: Option<String>,
    session_path: Option<PathBuf>,
) {
    let pending_opens = Arc::new(Mutex::new(Vec::<PathBuf>::new()));

    let seed = match (initial, session_path) {
        (_, Some(path)) => MainWindowSeed::Session(path),
        (Some(composition), None) => MainWindowSeed::Preloaded {
            composition,
            load_elapsed,
        },
        (None, None) => MainWindowSeed::Empty,
    };

    let app = gpui_kit::application().with_assets(AppAssets);
    // Observe willFinishLaunching → NSAppleEventManager odoc handler. Must be
    // before `run` so the observer is live when AppKit posts the notification.
    #[cfg(target_os = "macos")]
    crate::macos_open::install();
    // Backup: GPUI application:openURLs: (used if our AE handler is not yet set).
    app.on_open_urls({
        let pending_opens = pending_opens.clone();
        move |urls| {
            let mut pending = pending_opens.lock().unwrap();
            for url in urls {
                if let Some(path) = path_from_open_url(&url) {
                    pending.push(path);
                }
            }
        }
    });
    // Dock / Finder reopen. Ignore until the deferred first-window open has
    // finished — otherwise cold launch via Open Document creates an empty
    // editor and drops the preload / pending-open seed.
    app.on_reopen(|cx| {
        let ready = cx.try_global::<AppLaunchState>().is_some_and(|launch| {
            launch
                .launch_ui_ready
                .load(std::sync::atomic::Ordering::SeqCst)
        });
        if !ready {
            return;
        }
        if let Some((_, handle)) = living_editor_window(cx) {
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
            return;
        }
        open_main_window(cx, vec![]);
    });
    app.run(move |cx| {
        // Keep did_finish_launching light: activating or opening a window here
        // can re-enter applicationShouldHandleReopen while App is still
        // borrowed (RefCell panic → abort across the ObjC boundary). That shows
        // up when launching by double-clicking a document.
        gpui_kit::init(cx);

        #[cfg(target_os = "macos")]
        {
            // Cold-launch odoc may have been handled before App existed.
            pending_opens
                .lock()
                .unwrap()
                .extend(crate::macos_open::take_queued_paths());
            pending_opens
                .lock()
                .unwrap()
                .extend(crate::macos_open::paths_from_current_open_documents_event());
            // Warm opens: queue paths, then flush on the main-thread GPUI
            // executor. AsyncApp is !Send, so it lives in a thread-local; the
            // AE handler only touches the Send `pending_opens` Arc.
            thread_local! {
                static ASYNC_APP: std::cell::RefCell<Option<gpui_kit::AsyncApp>> =
                    const { std::cell::RefCell::new(None) };
            }
            ASYNC_APP.with(|slot| {
                *slot.borrow_mut() = Some(cx.to_async());
            });
            let pending_opens = pending_opens.clone();
            crate::macos_open::set_open_handler(move |paths| {
                crate::macos_open::log_open(&format!(
                    "open_handler: queueing {} path(s)",
                    paths.len()
                ));
                pending_opens.lock().unwrap().extend(paths);
                ASYNC_APP.with(|slot| {
                    let Some(async_cx) = slot.borrow().clone() else {
                        crate::macos_open::log_open("open_handler: no AsyncApp yet");
                        return;
                    };
                    async_cx
                        .spawn(async move |cx| {
                            cx.update(|cx| {
                                let ready =
                                    cx.try_global::<AppLaunchState>().is_some_and(|launch| {
                                        launch
                                            .launch_ui_ready
                                            .load(std::sync::atomic::Ordering::SeqCst)
                                    });
                                if living_editor_window(cx).is_some() {
                                    flush_pending_opens_into_editor(cx);
                                } else if ready {
                                    open_main_window(cx, vec![]);
                                }
                            });
                        })
                        .detach();
                });
            });
        }

        cx.set_global(AppLaunchState {
            pending_opens: pending_opens.clone(),
            output_device_spec: output_spec.clone(),
            launch_ui_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        cx.on_window_closed(on_app_window_closed).detach();

        cx.spawn(async move |cx| {
            // Yield so application:openURLs: can also append to pending_opens.
            cx.background_executor()
                .timer(Duration::from_millis(0))
                .await;
            #[cfg(target_os = "macos")]
            {
                let _ = cx.update(|cx| {
                    if let Some(launch) = cx.try_global::<AppLaunchState>() {
                        launch
                            .pending_opens
                            .lock()
                            .unwrap()
                            .extend(crate::macos_open::take_queued_paths());
                    }
                });
            }
            cx.update(|cx| {
                Theme::change(ThemeMode::Dark, None, cx);
                apply_muted_chrome(cx);
                install_app_menu(cx);
                open_main_window_seeded(cx, vec![], seed);
                mark_launch_ui_ready(cx);
            });
            // Straggler openURLs / AE deliveries just after the first window.
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            cx.update(|cx| {
                flush_pending_opens_into_editor(cx);
            });
        })
        .detach();
    });
}
