// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    resizable_panel,
    tooltip::Tooltip,
    v_flex, v_resizable, ActiveTheme as _, Colorize as _, Icon, IconName, IconNamed,
    Selectable as _, Sizable as _, StyledExt as _,
};
use gpui_kit::{
    actions, div, prelude::FluentBuilder as _, px, App, AppContext as _, Bounds, ClickEvent,
    Context, DragMoveEvent, Entity, EventEmitter, ExternalDragPayload, FileDragPaths, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, ParentElement as _,
    Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::model::DocumentId;

actions!(
    explorer,
    [
        ConfirmSelected,
        SelectPrev,
        SelectNext,
        RenameSelected,
        CancelRename
    ]
);

const CONTEXT: &str = "Compositions";
const SESSION_LABEL: &str = "session";
const DISCLOSURE_SLOT: f32 = 14.;
const INFO_PATH_CHARS: usize = 40;
const INFO_MEDIA_PATH_CHARS: usize = 28;
const INFO_PANE_DEFAULT: f32 = 160.;
const INFO_PANE_MIN: f32 = 96.;
const INFO_PANE_MAX: f32 = 480.;

/// One media entry shown in the explorer Info tool.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InfoMediaRow {
    pub path: String,
    pub sample_rate: u32,
    pub channel_count: usize,
    pub duration_secs: f64,
    pub size_bytes: u64,
}

type InfoProvider = Rc<dyn Fn(DocumentId, &App) -> Vec<InfoMediaRow>>;

struct LinkIcon;

impl IconNamed for LinkIcon {
    fn path(self) -> SharedString {
        "icons/link-2.svg".into()
    }
}

struct AudioLinesIcon;

impl IconNamed for AudioLinesIcon {
    fn path(self) -> SharedString {
        "icons/audio-lines.svg".into()
    }
}

struct SquareTextIcon;

impl IconNamed for SquareTextIcon {
    fn path(self) -> SharedString {
        "icons/square-text.svg".into()
    }
}

/// Shorten `text` to at most `max_chars`, keeping the start and end with `…`
/// in the middle.
fn middle_ellipsis(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".into();
    }
    let keep = max_chars - 1;
    let head = keep / 2;
    let tail = keep - head;
    let mut out = String::with_capacity(max_chars);
    out.extend(chars.iter().take(head));
    out.push('…');
    out.extend(chars.iter().skip(chars.len() - tail));
    out
}

fn format_duration_secs(secs: f64) -> String {
    if !secs.is_finite() || secs < 0. {
        return "—".into();
    }
    if secs < 10. {
        format!("{secs:.2}s")
    } else if secs < 100. {
        format!("{secs:.1}s")
    } else {
        format!("{secs:.0}s")
    }
}

fn format_size_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.;
    const MB: f64 = KB * 1024.;
    const GB: f64 = MB * 1024.;
    let n = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if n < MB {
        format!("{:.1} KB", n / KB)
    } else if n < GB {
        format!("{:.1} MB", n / MB)
    } else {
        format!("{:.2} GB", n / GB)
    }
}

#[derive(Clone, Debug)]
pub enum ExplorerEvent {
    Activate(DocumentId),
    OpenTab(DocumentId),
    Close(DocumentId),
    /// Place `id` in `group` at 0-based `index` among that group's members
    /// (after removing the dragged row from the list).
    Place {
        id: DocumentId,
        group: Option<String>,
        index: usize,
    },
    /// Move `id` back to sit immediately under its open parent.
    Reattach(DocumentId),
    Rename {
        id: DocumentId,
        name: String,
    },
    RenameGroup {
        from: String,
        to: String,
    },
    AddGroup {
        name: String,
        after: Option<String>,
    },
    DeleteGroup {
        name: String,
    },
    /// Move named group to 0-based `index` among named groups.
    MoveGroup {
        name: String,
        index: usize,
    },
}

#[derive(Clone, PartialEq)]
struct ExplorerItem {
    id: DocumentId,
    name: SharedString,
    modified: bool,
    has_edits: bool,
    group: Option<String>,
    path: Option<PathBuf>,
    depth: usize,
    parent: Option<DocumentId>,
    /// Parent is open but this row is not in the contiguous block under it.
    detached: bool,
    /// Has contiguous attached descendants that can be disclosed.
    has_children: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SectionKey {
    Session,
    Named(String),
}

impl SectionKey {
    fn label(&self) -> SharedString {
        match self {
            Self::Session => SESSION_LABEL.into(),
            Self::Named(name) => name.clone().into(),
        }
    }

    fn element_id(&self) -> SharedString {
        match self {
            Self::Session => "composition-group-session".into(),
            Self::Named(name) => format!("composition-group-{name}").into(),
        }
    }

    fn group_name(&self) -> Option<String> {
        match self {
            Self::Session => None,
            Self::Named(name) => Some(name.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RenameTarget {
    Document(DocumentId),
    Group { name: String },
    NewGroup { after: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExplorerTool {
    Info,
}

/// Drag payload for explorer compositions. Path is offered to path fields and
/// as a native file drag when the pointer leaves the window.
#[derive(Clone, Debug)]
pub(crate) struct CompositionDrag {
    pub(crate) id: DocumentId,
    pub(crate) name: SharedString,
    pub(crate) path: Option<PathBuf>,
}

impl Render for CompositionDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .text_xs()
            .bg(cx.theme().secondary)
            .text_color(cx.theme().foreground)
            .rounded(cx.theme().radius)
            .shadow_md()
            .child(self.name.clone())
    }
}

/// Drag payload for reordering named group headers.
#[derive(Clone, Debug)]
struct GroupDrag {
    name: String,
}

impl Render for GroupDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .text_xs()
            .bg(cx.theme().secondary)
            .text_color(cx.theme().muted_foreground)
            .rounded(cx.theme().radius)
            .shadow_md()
            .child(self.name.clone())
    }
}

struct ExplorerSection<'a> {
    key: SectionKey,
    items: Vec<&'a ExplorerItem>,
}

fn group_sections<'a>(
    items: &'a [ExplorerItem],
    session_groups: &[String],
) -> Vec<ExplorerSection<'a>> {
    let mut sections = Vec::new();
    let session: Vec<&ExplorerItem> = items.iter().filter(|item| item.group.is_none()).collect();
    if !session.is_empty() {
        sections.push(ExplorerSection {
            key: SectionKey::Session,
            items: session,
        });
    }
    let mut named: HashMap<String, Vec<&ExplorerItem>> = HashMap::new();
    for item in items {
        let Some(name) = item.group.as_ref() else {
            continue;
        };
        named.entry(name.clone()).or_default().push(item);
    }
    for name in session_groups {
        sections.push(ExplorerSection {
            key: SectionKey::Named(name.clone()),
            items: named.remove(name).unwrap_or_default(),
        });
    }
    sections
}

fn item_is_descendant(
    items: &HashMap<DocumentId, &ExplorerItem>,
    id: DocumentId,
    ancestor: DocumentId,
) -> bool {
    let mut current = id;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Some(item) = items.get(&current) else {
            break;
        };
        match item.parent {
            Some(parent) if parent == ancestor => return true,
            Some(parent) => current = parent,
            None => break,
        }
    }
    false
}

/// Ids hidden because an ancestor row with attached children is collapsed.
fn hidden_under_collapsed(
    items: &[&ExplorerItem],
    collapsed_parents: &HashSet<DocumentId>,
) -> HashSet<DocumentId> {
    let by_id: HashMap<DocumentId, &ExplorerItem> =
        items.iter().map(|item| (item.id, *item)).collect();
    let ordered: Vec<DocumentId> = items.iter().map(|item| item.id).collect();
    let mut hidden = HashSet::new();
    for &parent in collapsed_parents {
        let Some(i) = ordered.iter().position(|&id| id == parent) else {
            continue;
        };
        if !items[i].has_children {
            continue;
        }
        let mut j = i + 1;
        while j < ordered.len() && item_is_descendant(&by_id, ordered[j], parent) {
            hidden.insert(ordered[j]);
            j += 1;
        }
    }
    hidden
}

fn visible_section_items<'a>(
    items: &'a [&'a ExplorerItem],
    collapsed_parents: &HashSet<DocumentId>,
) -> Vec<&'a ExplorerItem> {
    let hidden = hidden_under_collapsed(items, collapsed_parents);
    items
        .iter()
        .copied()
        .filter(|item| !hidden.contains(&item.id))
        .collect()
}

/// True when the pointer is in the upper half of `bounds` (insert before).
fn slot_before_from_y(y: Pixels, bounds: Bounds<Pixels>) -> bool {
    y < bounds.center().y
}

/// Convert a visual "before index" in the current section (including the
/// dragged row) into the `index_in_group` passed to `place_document`.
fn index_in_group_after_remove(drop_before: usize, dragged_index: Option<usize>) -> usize {
    match dragged_index {
        Some(from) if drop_before > from => drop_before - 1,
        _ => drop_before,
    }
}

type EventHandler = Rc<dyn Fn(ExplorerEvent, &mut Window, &mut App)>;

pub struct ExplorerPanel {
    items: Vec<ExplorerItem>,
    session_groups: Vec<String>,
    active: Option<DocumentId>,
    selected: Option<DocumentId>,
    hovered_close: Option<DocumentId>,
    /// Insertion gap while dragging a composition: before index `0..=section.len`.
    drop_slot: Option<(SectionKey, usize)>,
    /// Insertion gap while dragging a named group header: before index among
    /// named groups (`0..=session_groups.len`).
    group_drop_slot: Option<usize>,
    /// Armed only by CompositionDrag / GroupDrag `on_drag_move`. Cleared each
    /// paint so dock-resize (and other) GPUI drags cannot revive a stale slot.
    reorder_drop_active: bool,
    collapsed: HashSet<SectionKey>,
    /// Parents whose attached children are hidden.
    collapsed_parents: HashSet<DocumentId>,
    renaming: Option<RenameTarget>,
    rename_input: Option<Entity<InputState>>,
    active_tool: Option<ExplorerTool>,
    on_event: Option<EventHandler>,
    info_provider: Option<InfoProvider>,
    focus_handle: FocusHandle,
}

impl ExplorerPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("enter", ConfirmSelected, Some(CONTEXT)),
            KeyBinding::new("shift-enter", RenameSelected, Some(CONTEXT)),
            KeyBinding::new("escape", CancelRename, Some(CONTEXT)),
            KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
            KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        ]);
        Self {
            items: Vec::new(),
            session_groups: Vec::new(),
            active: None,
            selected: None,
            hovered_close: None,
            drop_slot: None,
            group_drop_slot: None,
            reorder_drop_active: false,
            collapsed: HashSet::new(),
            collapsed_parents: HashSet::new(),
            renaming: None,
            rename_input: None,
            active_tool: None,
            on_event: None,
            info_provider: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_handler(&mut self, handler: EventHandler) {
        self.on_event = Some(handler);
    }

    pub fn set_info_provider(&mut self, provider: InfoProvider) {
        self.info_provider = Some(provider);
    }

    fn handler(&self) -> Option<EventHandler> {
        self.on_event.clone()
    }

    pub fn set_documents(
        &mut self,
        docs: &[(
            DocumentId,
            SharedString,
            bool,
            bool,
            Option<String>,
            Option<PathBuf>,
            usize,
            Option<DocumentId>,
            bool,
            bool,
        )],
        session_groups: &[String],
        active: Option<DocumentId>,
        cx: &mut Context<Self>,
    ) {
        let items: Vec<ExplorerItem> = docs
            .iter()
            .map(
                |(
                    id,
                    name,
                    modified,
                    has_edits,
                    group,
                    path,
                    depth,
                    parent,
                    detached,
                    has_children,
                )| {
                    ExplorerItem {
                        id: *id,
                        name: name.clone(),
                        modified: *modified,
                        has_edits: *has_edits,
                        group: group.clone(),
                        path: path.clone(),
                        depth: *depth,
                        parent: *parent,
                        detached: *detached,
                        has_children: *has_children,
                    }
                },
            )
            .collect();
        if items == self.items
            && active == self.active
            && session_groups == self.session_groups.as_slice()
        {
            return;
        }
        let keep = self
            .selected
            .filter(|id| items.iter().any(|item| item.id == *id));
        self.items = items;
        self.session_groups = session_groups.to_vec();
        self.active = active;
        self.selected = keep.or(active);
        let live: HashSet<_> = group_sections(&self.items, &self.session_groups)
            .into_iter()
            .map(|section| section.key)
            .collect();
        self.collapsed.retain(|key| live.contains(key));
        let live_ids: HashSet<_> = self.items.iter().map(|item| item.id).collect();
        self.collapsed_parents.retain(|id| live_ids.contains(id));
        if let Some(RenameTarget::Document(id)) = self.renaming {
            if !live_ids.contains(&id) {
                self.renaming = None;
                self.rename_input = None;
            }
        }
        cx.notify();
    }

    /// Align keyboard/ghost selection with the active document.
    pub fn sync_selection_to_active(&mut self, cx: &mut Context<Self>) {
        if self.selected == self.active {
            return;
        }
        self.selected = self.active;
        cx.notify();
    }

    fn toggle_parent_collapse(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if !self.collapsed_parents.remove(&id) {
            self.collapsed_parents.insert(id);
        }
        cx.notify();
    }

    fn selected_or_active(&self) -> Option<DocumentId> {
        self.selected.or(self.active)
    }

    fn visible_ids(&self) -> Vec<DocumentId> {
        let mut out = Vec::new();
        for section in group_sections(&self.items, &self.session_groups) {
            if self.collapsed.contains(&section.key) {
                continue;
            }
            let hidden = hidden_under_collapsed(&section.items, &self.collapsed_parents);
            for item in section.items {
                if !hidden.contains(&item.id) {
                    out.push(item.id);
                }
            }
        }
        out
    }

    fn select_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            return;
        }
        let visible = self.visible_ids();
        if visible.is_empty() {
            return;
        }
        let current = self.selected.or(self.active);
        let len = visible.len() as isize;
        let next = match current.and_then(|id| visible.iter().position(|item| *item == id)) {
            Some(ix) => (ix as isize + delta).rem_euclid(len) as usize,
            None if delta < 0 => visible.len() - 1,
            None => 0,
        };
        self.selected = Some(visible[next]);
        cx.notify();
    }

    fn toggle_section(&mut self, key: SectionKey, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        cx.notify();
    }

    fn set_drop_slot(&mut self, slot: Option<(SectionKey, usize)>, cx: &mut Context<Self>) {
        self.reorder_drop_active = true;
        if self.drop_slot != slot {
            self.drop_slot = slot;
            cx.notify();
        }
    }

    fn set_group_drop_slot(&mut self, slot: Option<usize>, cx: &mut Context<Self>) {
        self.reorder_drop_active = true;
        if self.group_drop_slot != slot {
            self.group_drop_slot = slot;
            cx.notify();
        }
    }

    /// Drop slots must not outlive a cancelled reorder. Dock splitter
    /// resize (and other chrome) also uses GPUI `on_drag`, so
    /// `has_active_drag` alone would revive a stale insertion marker.
    fn take_reorder_drop_slots(
        &mut self,
        cx: &App,
    ) -> (Option<(SectionKey, usize)>, Option<usize>) {
        let (drop_slot, group_drop_slot, _) = visible_reorder_drop_slots(
            cx.has_active_drag(),
            &mut self.drop_slot,
            &mut self.group_drop_slot,
            &mut self.reorder_drop_active,
        );
        (drop_slot, group_drop_slot)
    }

    fn begin_rename(&mut self, target: RenameTarget, window: &mut Window, cx: &mut Context<Self>) {
        let seed = match &target {
            RenameTarget::Document(id) => self
                .items
                .iter()
                .find(|item| item.id == *id)
                .map(|item| item.name.to_string())
                .unwrap_or_default(),
            RenameTarget::Group { name } => name.clone(),
            RenameTarget::NewGroup { .. } => String::new(),
        };
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| {
            state.set_value(seed, window, cx);
            state.select_all(window, cx);
        });
        cx.subscribe_in(
            &input,
            window,
            |this, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => this.commit_rename(window, cx),
                InputEvent::Blur => this.commit_rename(window, cx),
                _ => {}
            },
        )
        .detach();
        self.renaming = Some(target);
        self.rename_input = Some(input.clone());
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.is_none() && self.rename_input.is_none() {
            return;
        }
        self.renaming = None;
        self.rename_input = None;
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.renaming.take() else {
            self.rename_input = None;
            return;
        };
        let name = self
            .rename_input
            .take()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
            .trim()
            .to_string();
        cx.notify();
        if name.is_empty() {
            return;
        }
        let event = match target {
            RenameTarget::Document(id) => ExplorerEvent::Rename { id, name },
            RenameTarget::Group { name: from } => ExplorerEvent::RenameGroup { from, to: name },
            RenameTarget::NewGroup { after } => ExplorerEvent::AddGroup { name, after },
        };
        let explorer = cx.entity();
        window.defer(cx, move |window, cx| {
            dispatch(&explorer, event, window, cx);
        });
    }

    fn toggle_info_tool(&mut self, cx: &mut Context<Self>) {
        if self.active_tool == Some(ExplorerTool::Info) {
            self.active_tool = None;
        } else {
            self.active_tool = Some(ExplorerTool::Info);
        }
        cx.notify();
    }

    fn close_tool(&mut self, cx: &mut Context<Self>) {
        if self.active_tool.take().is_some() {
            cx.notify();
        }
    }
}

/// Call the host without holding an `ExplorerPanel` lease. `AppView`
/// refreshes this panel, so invoking the handler from `explorer.update`
/// panics.
fn dispatch(
    explorer: &Entity<ExplorerPanel>,
    event: ExplorerEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(handler) = explorer.read(cx).handler() {
        handler(event, window, cx);
    }
}

fn accepts_composition_drag(data: &dyn std::any::Any) -> bool {
    data.downcast_ref::<CompositionDrag>().is_some()
}

fn accepts_group_drag(data: &dyn std::any::Any) -> bool {
    data.downcast_ref::<GroupDrag>().is_some()
}

fn drop_on_section(
    explorer: &Entity<ExplorerPanel>,
    target: &SectionKey,
    drag: &CompositionDrag,
    section_len: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let drop_before_visual = explorer
        .read(cx)
        .drop_slot
        .as_ref()
        .filter(|(key, _)| key == target)
        .map(|(_, before)| *before)
        .unwrap_or(section_len);
    let (full_ids, visible_ids) = {
        let panel = explorer.read(cx);
        let section_items: Vec<&ExplorerItem> = panel
            .items
            .iter()
            .filter(|item| section_key_for_group(item.group.as_deref()) == *target)
            .collect();
        let visible = visible_section_items(&section_items, &panel.collapsed_parents);
        let full_ids: Vec<DocumentId> = section_items.iter().map(|item| item.id).collect();
        let visible_ids: Vec<DocumentId> = visible.iter().map(|item| item.id).collect();
        (full_ids, visible_ids)
    };
    let drop_before_full = if drop_before_visual >= visible_ids.len() {
        full_ids.len()
    } else {
        let target_id = visible_ids[drop_before_visual];
        full_ids
            .iter()
            .position(|id| *id == target_id)
            .unwrap_or(full_ids.len())
    };
    let dragged_in_section = full_ids.iter().position(|id| *id == drag.id);
    let index = index_in_group_after_remove(drop_before_full, dragged_in_section);
    explorer.update(cx, |this, cx| {
        this.drop_slot = None;
        cx.notify();
    });
    dispatch(
        explorer,
        ExplorerEvent::Place {
            id: drag.id,
            group: target.group_name(),
            index,
        },
        window,
        cx,
    );
}

fn drop_group(
    explorer: &Entity<ExplorerPanel>,
    drag: &GroupDrag,
    named_count: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let drop_before = explorer.read(cx).group_drop_slot.unwrap_or(named_count);
    let from = explorer
        .read(cx)
        .session_groups
        .iter()
        .position(|name| name == &drag.name);
    let index = index_in_group_after_remove(drop_before, from);
    explorer.update(cx, |this, cx| {
        this.group_drop_slot = None;
        cx.notify();
    });
    if from == Some(index) {
        return;
    }
    dispatch(
        explorer,
        ExplorerEvent::MoveGroup {
            name: drag.name.clone(),
            index,
        },
        window,
        cx,
    );
}

fn section_key_for_group(group: Option<&str>) -> SectionKey {
    match group {
        Some(name) => SectionKey::Named(name.to_string()),
        None => SectionKey::Session,
    }
}

/// Decide which insertion markers to paint for this frame.
///
/// `reorder_drop_active` is armed only by CompositionDrag / GroupDrag
/// `on_drag_move` and consumed here, so other GPUI drags (dock resize)
/// cannot show a leftover marker.
fn visible_reorder_drop_slots(
    has_active_drag: bool,
    drop_slot: &mut Option<(SectionKey, usize)>,
    group_drop_slot: &mut Option<usize>,
    reorder_drop_active: &mut bool,
) -> (Option<(SectionKey, usize)>, Option<usize>, bool) {
    if !has_active_drag {
        *drop_slot = None;
        *group_drop_slot = None;
        *reorder_drop_active = false;
        return (None, None, false);
    }
    let live = *reorder_drop_active;
    *reorder_drop_active = false;
    if live {
        (drop_slot.clone(), *group_drop_slot, true)
    } else {
        (None, None, false)
    }
}

/// Overlay line that does not consume layout height (avoids list shift while
/// dragging, which would otherwise flicker the drop slot around midpoints).
fn insertion_marker_overlay(color: gpui_kit::Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(px(6.))
        .right(px(6.))
        .top(px(-1.))
        .h(px(2.))
        .rounded_full()
        .bg(color)
}

fn ghost_hover_bg(cx: &App) -> gpui_kit::Hsla {
    let theme = cx.theme();
    if theme.mode.is_dark() {
        theme.secondary.lighten(0.1).opacity(0.8)
    } else {
        theme.secondary.darken(0.1).opacity(0.8)
    }
}

impl EventEmitter<PanelEvent> for ExplorerPanel {}

impl Focusable for ExplorerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ExplorerPanel {
    fn panel_name(&self) -> &'static str {
        "ExplorerPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ExplorerPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::dock_titles::EXPLORER_TAB_COMPOSITIONS
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(crate::dock_titles::EXPLORER_TAB_COMPOSITIONS.into())
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ExplorerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let explorer = cx.entity();
        let hovered_close = self.hovered_close;
        let selected = self.selected;
        let active = self.active;
        let collapsed = self.collapsed.clone();
        let collapsed_parents = self.collapsed_parents.clone();
        let renaming = self.renaming.clone();
        let rename_input = self.rename_input.clone();
        let active_tool = self.active_tool;
        let highlight_bg = ghost_hover_bg(cx);
        let muted = cx.theme().muted_foreground;
        let cyan = cx.theme().cyan;
        let radius = cx.theme().radius;
        let border = cx.theme().border;
        let (drop_slot, group_drop_slot) = self.take_reorder_drop_slots(cx);
        let sections = group_sections(&self.items, &self.session_groups);
        let named_count = self.session_groups.len();

        let info_focus = self.selected_or_active().and_then(|id| {
            self.items
                .iter()
                .find(|item| item.id == id)
                .map(|item| (item.id, item.path.clone()))
        });
        let info_media = match (
            active_tool,
            info_focus.as_ref(),
            self.info_provider.as_ref(),
        ) {
            (Some(ExplorerTool::Info), Some((id, _)), Some(provider)) => provider(*id, cx),
            _ => Vec::new(),
        };

        v_flex()
            .id("compositions-panel")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action({
                let explorer = explorer.clone();
                move |_: &ConfirmSelected, window, cx| {
                    if explorer.read(cx).renaming.is_some() {
                        return;
                    }
                    let Some(id) = explorer.read(cx).selected_or_active() else {
                        return;
                    };
                    explorer.update(cx, |this, cx| {
                        this.selected = Some(id);
                        cx.notify();
                    });
                    dispatch(&explorer, ExplorerEvent::Activate(id), window, cx);
                }
            })
            .on_action({
                let explorer = explorer.clone();
                move |_: &RenameSelected, window, cx| {
                    let Some(id) = explorer.read(cx).selected_or_active() else {
                        return;
                    };
                    explorer.update(cx, |this, cx| {
                        this.begin_rename(RenameTarget::Document(id), window, cx);
                    });
                }
            })
            .on_action(cx.listener(|this, _: &CancelRename, _, cx| {
                this.cancel_rename(cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                this.select_delta(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.select_delta(1, cx);
            }))
            .child({
                let list = div()
                    .id("compositions-list")
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children({
                        let mut children = Vec::new();
                        let mut named_index = 0usize;
                        for section in sections {
                            let is_named = matches!(section.key, SectionKey::Named(_));
                            let this_named_index = if is_named {
                                let i = named_index;
                                named_index += 1;
                                Some(i)
                            } else {
                                None
                            };

                            let open = !collapsed.contains(&section.key);
                            let explorer = explorer.clone();
                            let header_key = section.key.clone();
                            let drop_key = section.key.clone();
                            let visible_items: Vec<ExplorerItem> =
                                visible_section_items(&section.items, &collapsed_parents)
                                    .into_iter()
                                    .cloned()
                                    .collect();
                            let section_len = visible_items.len();
                            let show_slot = drop_slot
                                .as_ref()
                                .filter(|(key, _)| key == &section.key)
                                .map(|(_, before)| *before);
                            let show_group_marker = this_named_index
                                .and_then(|i| group_drop_slot.filter(|&slot| slot == i));
                            let renaming_this_group = matches!(
                                &renaming,
                                Some(RenameTarget::Group { name })
                                    if section.key.group_name().as_deref() == Some(name.as_str())
                            );
                            let group_name = section.key.group_name();

                            children.push(
                                v_flex()
                                    .id(section.key.element_id())
                                    .w_full()
                                    .flex_none()
                                    .can_drop(|data, _, _| accepts_composition_drag(data))
                                    .on_drag_move(cx.listener({
                                        let key = drop_key.clone();
                                        let len = section_len;
                                        move |this, event: &DragMoveEvent<CompositionDrag>, _, cx| {
                                            if !event.bounds.contains(&event.event.position) {
                                                return;
                                            }
                                            this.set_drop_slot(Some((key.clone(), len)), cx);
                                        }
                                    }))
                                    .on_drop({
                                        let explorer = explorer.clone();
                                        let target = drop_key.clone();
                                        move |drag: &CompositionDrag, window, cx| {
                                            drop_on_section(
                                                &explorer,
                                                &target,
                                                drag,
                                                section_len,
                                                window,
                                                cx,
                                            );
                                        }
                                    })
                                    .child({
                                        let explorer = explorer.clone();
                                        let label = if renaming_this_group {
                                            if let Some(input) = rename_input.clone() {
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .child(Input::new(&input).xsmall().w_full())
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation();
                                                    })
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(section.key.label())
                                                    .into_any_element()
                                            }
                                        } else {
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(section.key.label())
                                                .into_any_element()
                                        };
                                        h_flex()
                                            .id(SharedString::from(format!(
                                                "{}-header",
                                                section.key.element_id()
                                            )))
                                            .relative()
                                            .w_full()
                                            .flex_none()
                                            .items_center()
                                            .gap_1()
                                            .px_1p5()
                                            .py_0p5()
                                            .cursor_pointer()
                                            .when(show_group_marker.is_some(), |this| {
                                                this.child(insertion_marker_overlay(cyan))
                                            })
                                            .can_drop(|data, _, _| {
                                                accepts_composition_drag(data)
                                                    || accepts_group_drag(data)
                                            })
                                            .on_drag_move(cx.listener({
                                                let key = drop_key.clone();
                                                move |this,
                                                      event: &DragMoveEvent<CompositionDrag>,
                                                      _,
                                                      cx| {
                                                    if !event
                                                        .bounds
                                                        .contains(&event.event.position)
                                                    {
                                                        return;
                                                    }
                                                    this.set_drop_slot(
                                                        Some((key.clone(), 0)),
                                                        cx,
                                                    );
                                                }
                                            }))
                                            .on_drop({
                                                let explorer = explorer.clone();
                                                let target = drop_key.clone();
                                                move |drag: &CompositionDrag, window, cx| {
                                                    drop_on_section(
                                                        &explorer,
                                                        &target,
                                                        drag,
                                                        section_len,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            })
                                            .when_some(this_named_index, |this, named_i| {
                                                this.on_drag(
                                                    GroupDrag {
                                                        name: group_name
                                                            .clone()
                                                            .unwrap_or_default(),
                                                    },
                                                    |drag, _, _, cx| {
                                                        cx.stop_propagation();
                                                        cx.new(|_| drag.clone())
                                                    },
                                                )
                                                .on_drag_move(cx.listener(
                                                    move |this,
                                                          event: &DragMoveEvent<GroupDrag>,
                                                          _,
                                                          cx| {
                                                        if !event
                                                            .bounds
                                                            .contains(&event.event.position)
                                                        {
                                                            return;
                                                        }
                                                        let before = slot_before_from_y(
                                                            event.event.position.y,
                                                            event.bounds,
                                                        );
                                                        let slot = if before {
                                                            named_i
                                                        } else {
                                                            named_i + 1
                                                        };
                                                        this.set_group_drop_slot(Some(slot), cx);
                                                    },
                                                ))
                                                .on_drop({
                                                    let explorer = explorer.clone();
                                                    move |drag: &GroupDrag, window, cx| {
                                                        drop_group(
                                                            &explorer,
                                                            drag,
                                                            named_count,
                                                            window,
                                                            cx,
                                                        );
                                                    }
                                                })
                                            })
                                            .when(!renaming_this_group, |this| {
                                                this.on_click({
                                                    let explorer = explorer.clone();
                                                    let header_key = header_key.clone();
                                                    move |_, _, cx| {
                                                        explorer.update(cx, |this, cx| {
                                                            this.toggle_section(
                                                                header_key.clone(),
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                })
                                            })
                                            .context_menu({
                                                let explorer = explorer.clone();
                                                let group_name = group_name.clone();
                                                move |menu, _, _| {
                                                    let add_group = PopupMenuItem::new(
                                                        "Add Group...",
                                                    )
                                                    .on_click({
                                                        let explorer = explorer.clone();
                                                        let after = group_name.clone();
                                                        move |_, window, cx| {
                                                            explorer.update(cx, |this, cx| {
                                                                this.begin_rename(
                                                                    RenameTarget::NewGroup {
                                                                        after: after.clone(),
                                                                    },
                                                                    window,
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    });
                                                    if let Some(name) = group_name.clone() {
                                                        menu.item(
                                                            PopupMenuItem::new("Rename...")
                                                                .on_click({
                                                                    let explorer = explorer.clone();
                                                                    let name = name.clone();
                                                                    move |_, window, cx| {
                                                                        explorer.update(
                                                                            cx,
                                                                            |this, cx| {
                                                                                this.begin_rename(
                                                                                    RenameTarget::Group {
                                                                                        name: name
                                                                                            .clone(),
                                                                                    },
                                                                                    window,
                                                                                    cx,
                                                                                );
                                                                            },
                                                                        );
                                                                    }
                                                                }),
                                                        )
                                                        .item(add_group)
                                                        .item(
                                                            PopupMenuItem::new("Delete Group")
                                                                .on_click({
                                                                    let explorer = explorer.clone();
                                                                    move |_, window, cx| {
                                                                        dispatch(
                                                                            &explorer,
                                                                            ExplorerEvent::DeleteGroup {
                                                                                name: name.clone(),
                                                                            },
                                                                            window,
                                                                            cx,
                                                                        );
                                                                    }
                                                                }),
                                                        )
                                                    } else {
                                                        menu.item(add_group)
                                                    }
                                                }
                                            })
                                            .child(
                                                Icon::new(if open {
                                                    IconName::ChevronDown
                                                } else {
                                                    IconName::ChevronRight
                                                })
                                                .xsmall()
                                                .text_color(muted),
                                            )
                                            .child(label)
                                    })
                                    .children(open.then(|| {
                                        let explorer = explorer.clone();
                                        let drop_key = drop_key.clone();
                                        let collapsed_parents = collapsed_parents.clone();
                                        let renaming = renaming.clone();
                                        let rename_input = rename_input.clone();
                                        v_flex().w_full().children({
                                            let mut rows =
                                                Vec::with_capacity(visible_items.len() + 1);
                                            for (index, item) in
                                                visible_items.into_iter().enumerate()
                                            {
                                                let id = item.id;
                                                let name = item.name.clone();
                                                let path = item.path.clone();
                                                let is_modified = item.modified;
                                                let has_edits = item.has_edits;
                                                let depth = item.depth;
                                                let parent_id = item.parent;
                                                let detached = item.detached;
                                                let has_children = item.has_children;
                                                let parent_open =
                                                    has_children && !collapsed_parents.contains(&id);
                                                let highlighted = selected == Some(id);
                                                let is_active = active == Some(id);
                                                let show_close = hovered_close == Some(id);
                                                let renaming_this = matches!(
                                                    &renaming,
                                                    Some(RenameTarget::Document(doc)) if *doc == id
                                                );
                                                let explorer = explorer.clone();
                                                let drop_target = drop_key.clone();
                                                let marker_top = show_slot == Some(index);
                                                let indent = px(8.0 + depth as f32 * 12.0);
                                                rows.push(
                                                    h_flex()
                                                        .id(SharedString::from(format!(
                                                            "composition-{id}"
                                                        )))
                                                        .relative()
                                                        .w_full()
                                                        .flex_none()
                                                        .items_center()
                                                        .pl(indent)
                                                        .pr_1p5()
                                                        .py_0p5()
                                                        .gap_0p5()
                                                        .rounded(radius)
                                                        .text_xs()
                                                        .cursor_pointer()
                                                        .when(highlighted, |this| {
                                                            this.bg(highlight_bg)
                                                        })
                                                        .when(!highlighted, |this| {
                                                            this.hover(|this| this.bg(highlight_bg))
                                                        })
                                                        .when(marker_top, |this| {
                                                            this.child(insertion_marker_overlay(
                                                                cyan,
                                                            ))
                                                        })
                                                        .when(!renaming_this, |this| {
                                                            this.on_drag(
                                                                CompositionDrag {
                                                                    id,
                                                                    name: name.clone(),
                                                                    path: path.clone(),
                                                                },
                                                                |drag, _, _, cx| {
                                                                    cx.stop_propagation();
                                                                    cx.new(|_| drag.clone())
                                                                },
                                                            )
                                                            .external_drag_payload(
                                                                |drag: &CompositionDrag, _, _| {
                                                                    drag.path.as_ref().map(|path| {
                                                                        ExternalDragPayload::Files(
                                                                            FileDragPaths::new([(
                                                                                path.clone(),
                                                                                false,
                                                                            )]),
                                                                        )
                                                                    })
                                                                },
                                                            )
                                                        })
                                                        .can_drop(|data, _, _| {
                                                            accepts_composition_drag(data)
                                                        })
                                                        .on_drag_move(cx.listener({
                                                            let key = drop_target.clone();
                                                            move |this,
                                                                  event: &DragMoveEvent<
                                                                CompositionDrag,
                                                            >,
                                                                  _,
                                                                  cx| {
                                                                if !event
                                                                    .bounds
                                                                    .contains(&event.event.position)
                                                                {
                                                                    return;
                                                                }
                                                                let before = slot_before_from_y(
                                                                    event.event.position.y,
                                                                    event.bounds,
                                                                );
                                                                let slot = if before {
                                                                    index
                                                                } else {
                                                                    index + 1
                                                                };
                                                                this.set_drop_slot(
                                                                    Some((key.clone(), slot)),
                                                                    cx,
                                                                );
                                                            }
                                                        }))
                                                        .on_drop({
                                                            let explorer = explorer.clone();
                                                            let target = drop_target;
                                                            move |drag: &CompositionDrag,
                                                                  window,
                                                                  cx| {
                                                                drop_on_section(
                                                                    &explorer,
                                                                    &target,
                                                                    drag,
                                                                    section_len,
                                                                    window,
                                                                    cx,
                                                                );
                                                            }
                                                        })
                                                        .on_click({
                                                            let explorer = explorer.clone();
                                                            move |event: &ClickEvent, window, cx| {
                                                                if explorer.read(cx).renaming.is_some()
                                                                {
                                                                    return;
                                                                }
                                                                explorer.update(cx, |this, cx| {
                                                                    this.selected = Some(id);
                                                                    cx.notify();
                                                                });
                                                                let event = if event.click_count()
                                                                    >= 2
                                                                {
                                                                    ExplorerEvent::OpenTab(id)
                                                                } else {
                                                                    ExplorerEvent::Activate(id)
                                                                };
                                                                dispatch(
                                                                    &explorer, event, window, cx,
                                                                );
                                                            }
                                                        })
                                                        .context_menu({
                                                            let explorer = explorer.clone();
                                                            move |menu, _, _| {
                                                                let mut menu = menu;
                                                                if let Some(parent_id) = parent_id {
                                                                    menu = menu.item(
                                                                        PopupMenuItem::new(
                                                                            "Reveal Parent",
                                                                        )
                                                                        .on_click({
                                                                            let explorer =
                                                                                explorer.clone();
                                                                            move |_, window, cx| {
                                                                                dispatch(
                                                                                    &explorer,
                                                                                    ExplorerEvent::Activate(
                                                                                        parent_id,
                                                                                    ),
                                                                                    window,
                                                                                    cx,
                                                                                );
                                                                            }
                                                                        }),
                                                                    );
                                                                }
                                                                menu.item(
                                                                    PopupMenuItem::new("Close")
                                                                        .on_click({
                                                                            let explorer =
                                                                                explorer.clone();
                                                                            move |_, window, cx| {
                                                                                dispatch(
                                                                                    &explorer,
                                                                                    ExplorerEvent::Close(
                                                                                        id,
                                                                                    ),
                                                                                    window,
                                                                                    cx,
                                                                                );
                                                                            }
                                                                        }),
                                                                )
                                                            }
                                                        })
                                                        .child({
                                                            // Left gutter: link when detached
                                                            // (aligned with group disclosure
                                                            // triangles); otherwise the parent
                                                            // chevron when this row has children.
                                                            let explorer = explorer.clone();
                                                            div()
                                                                .id(SharedString::from(if detached {
                                                                    format!("comp-link-{id}")
                                                                } else {
                                                                    format!("comp-disclose-{id}")
                                                                }))
                                                                .w(px(DISCLOSURE_SLOT))
                                                                .h(px(DISCLOSURE_SLOT))
                                                                .flex()
                                                                .flex_none()
                                                                .items_center()
                                                                .justify_center()
                                                                .when(detached, |this| {
                                                                    this.cursor_pointer()
                                                                        .on_mouse_down(
                                                                            MouseButton::Left,
                                                                            |_, _, cx| {
                                                                                cx.stop_propagation();
                                                                            },
                                                                        )
                                                                        .on_click({
                                                                            let explorer =
                                                                                explorer.clone();
                                                                            move |event: &ClickEvent,
                                                                                  window,
                                                                                  cx| {
                                                                                cx.stop_propagation();
                                                                                if event
                                                                                    .modifiers()
                                                                                    .shift
                                                                                {
                                                                                    dispatch(
                                                                                        &explorer,
                                                                                        ExplorerEvent::Reattach(
                                                                                            id,
                                                                                        ),
                                                                                        window,
                                                                                        cx,
                                                                                    );
                                                                                } else if let Some(
                                                                                    parent_id,
                                                                                ) =
                                                                                    parent_id
                                                                                {
                                                                                    explorer
                                                                                        .update(
                                                                                            cx,
                                                                                            |this,
                                                                                             cx| {
                                                                                                this.selected = Some(
                                                                                                    parent_id,
                                                                                                );
                                                                                                cx.notify();
                                                                                            },
                                                                                        );
                                                                                    dispatch(
                                                                                        &explorer,
                                                                                        ExplorerEvent::Activate(
                                                                                            parent_id,
                                                                                        ),
                                                                                        window,
                                                                                        cx,
                                                                                    );
                                                                                }
                                                                            }
                                                                        })
                                                                        .child(
                                                                            Icon::new(LinkIcon)
                                                                                .xsmall()
                                                                                .text_color(muted),
                                                                        )
                                                                })
                                                                .when(
                                                                    !detached && has_children,
                                                                    |this| {
                                                                        this.cursor_pointer()
                                                                            .on_mouse_down(
                                                                                MouseButton::Left,
                                                                                |_, _, cx| {
                                                                                    cx.stop_propagation();
                                                                                },
                                                                            )
                                                                            .on_click({
                                                                                let explorer =
                                                                                    explorer
                                                                                        .clone();
                                                                                move |_, _, cx| {
                                                                                    cx.stop_propagation();
                                                                                    explorer
                                                                                        .update(
                                                                                            cx,
                                                                                            |this,
                                                                                             cx| {
                                                                                                this.toggle_parent_collapse(
                                                                                                    id, cx,
                                                                                                );
                                                                                            },
                                                                                        );
                                                                                }
                                                                            })
                                                                            .child(
                                                                                Icon::new(
                                                                                    if parent_open {
                                                                                        IconName::ChevronDown
                                                                                    } else {
                                                                                        IconName::ChevronRight
                                                                                    },
                                                                                )
                                                                                .xsmall()
                                                                                .text_color(muted),
                                                                            )
                                                                    },
                                                                )
                                                        })
                                                        .when(detached && has_children, |this| {
                                                            // Rare: detached parent with attached
                                                            // descendants — keep collapse control
                                                            // beside the gutter link.
                                                            let explorer = explorer.clone();
                                                            this.child(
                                                                div()
                                                                    .id(SharedString::from(
                                                                        format!(
                                                                            "comp-disclose-{id}"
                                                                        ),
                                                                    ))
                                                                    .w(px(DISCLOSURE_SLOT))
                                                                    .h(px(DISCLOSURE_SLOT))
                                                                    .flex()
                                                                    .flex_none()
                                                                    .items_center()
                                                                    .justify_center()
                                                                    .cursor_pointer()
                                                                    .on_mouse_down(
                                                                        MouseButton::Left,
                                                                        |_, _, cx| {
                                                                            cx.stop_propagation();
                                                                        },
                                                                    )
                                                                    .on_click(move |_, _, cx| {
                                                                        cx.stop_propagation();
                                                                        explorer.update(
                                                                            cx,
                                                                            |this, cx| {
                                                                                this.toggle_parent_collapse(
                                                                                    id, cx,
                                                                                );
                                                                            },
                                                                        );
                                                                    })
                                                                    .child(
                                                                        Icon::new(
                                                                            if parent_open {
                                                                                IconName::ChevronDown
                                                                            } else {
                                                                                IconName::ChevronRight
                                                                            },
                                                                        )
                                                                        .xsmall()
                                                                        .text_color(muted),
                                                                    ),
                                                            )
                                                        })
                                                        .child(
                                                            div()
                                                                .id(SharedString::from(format!(
                                                                    "comp-kind-{id}"
                                                                )))
                                                                .w(px(DISCLOSURE_SLOT))
                                                                .h(px(DISCLOSURE_SLOT))
                                                                .flex()
                                                                .flex_none()
                                                                .items_center()
                                                                .justify_center()
                                                                .child(
                                                                    if has_edits {
                                                                        Icon::new(SquareTextIcon)
                                                                            .xsmall()
                                                                            .text_color(muted)
                                                                            .into_any_element()
                                                                    } else {
                                                                        Icon::new(AudioLinesIcon)
                                                                            .xsmall()
                                                                            .text_color(muted)
                                                                            .into_any_element()
                                                                    },
                                                                ),
                                                        )
                                                        .child({
                                                            if renaming_this {
                                                                if let Some(input) =
                                                                    rename_input.clone()
                                                                {
                                                                    div()
                                                                        .flex_1()
                                                                        .min_w_0()
                                                                        .child(
                                                                            Input::new(&input)
                                                                                .xsmall()
                                                                                .w_full(),
                                                                        )
                                                                        .on_mouse_down(
                                                                            MouseButton::Left,
                                                                            |_, _, cx| {
                                                                                cx.stop_propagation();
                                                                            },
                                                                        )
                                                                        .into_any_element()
                                                                } else {
                                                                    div()
                                                                        .flex_1()
                                                                        .min_w_0()
                                                                        .overflow_hidden()
                                                                        .whitespace_nowrap()
                                                                        .text_xs()
                                                                        .when(is_active, |this| {
                                                                            this.text_color(cyan)
                                                                        })
                                                                        .child(name)
                                                                        .into_any_element()
                                                                }
                                                            } else {
                                                                div()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .overflow_hidden()
                                                                    .whitespace_nowrap()
                                                                    .text_xs()
                                                                    .when(is_active, |this| {
                                                                        this.text_color(cyan)
                                                                    })
                                                                    .child(name)
                                                                    .into_any_element()
                                                            }
                                                        })
                                                        .child({
                                                            let explorer = explorer.clone();
                                                            div()
                                                                .id(SharedString::from(format!(
                                                                    "comp-eol-{id}"
                                                                )))
                                                                .w(px(18.))
                                                                .h(px(18.))
                                                                .flex()
                                                                .flex_none()
                                                                .items_center()
                                                                .justify_center()
                                                                .on_hover({
                                                                    let explorer = explorer.clone();
                                                                    move |hovered: &bool, _, cx| {
                                                                        explorer.update(
                                                                            cx,
                                                                            |this, cx| {
                                                                                this.hovered_close =
                                                                                    if *hovered {
                                                                                        Some(id)
                                                                                    } else {
                                                                                        None
                                                                                    };
                                                                                cx.notify();
                                                                            },
                                                                        );
                                                                    }
                                                                })
                                                                .on_mouse_down(
                                                                    MouseButton::Left,
                                                                    |_, _, cx| {
                                                                        cx.stop_propagation();
                                                                    },
                                                                )
                                                                .on_click({
                                                                    let explorer = explorer.clone();
                                                                    move |_, window, cx| {
                                                                        cx.stop_propagation();
                                                                        dispatch(
                                                                            &explorer,
                                                                            ExplorerEvent::Close(id),
                                                                            window,
                                                                            cx,
                                                                        );
                                                                    }
                                                                })
                                                                .when(show_close, |this| {
                                                                    this.child(
                                                                        Button::new(
                                                                            SharedString::from(
                                                                                format!(
                                                                                    "close-comp-{id}"
                                                                                ),
                                                                            ),
                                                                        )
                                                                        .ghost()
                                                                        .xsmall()
                                                                        .icon(IconName::Close)
                                                                        .tab_stop(false)
                                                                        .on_click({
                                                                            let explorer =
                                                                                explorer.clone();
                                                                            move |_, window, cx| {
                                                                                cx.stop_propagation();
                                                                                dispatch(
                                                                                    &explorer,
                                                                                    ExplorerEvent::Close(
                                                                                        id,
                                                                                    ),
                                                                                    window,
                                                                                    cx,
                                                                                );
                                                                            }
                                                                        }),
                                                                    )
                                                                })
                                                                .when(
                                                                    !show_close && is_modified,
                                                                    |this| {
                                                                        this.child(
                                                                            div()
                                                                                .size(px(6.))
                                                                                .rounded_full()
                                                                                .bg(muted),
                                                                        )
                                                                    },
                                                                )
                                                        })
                                                        .into_any_element(),
                                                );
                                            }
                                            let end_marker = show_slot == Some(section_len);
                                            let drop_target = drop_key.clone();
                                            let explorer = explorer.clone();
                                            rows.push(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "{}-end-gap",
                                                        drop_key.element_id()
                                                    )))
                                                    .relative()
                                                    .w_full()
                                                    .flex_none()
                                                    .h(px(12.))
                                                    .can_drop(|data, _, _| {
                                                        accepts_composition_drag(data)
                                                    })
                                                    .on_drag_move(cx.listener({
                                                        let key = drop_target.clone();
                                                        let len = section_len;
                                                        move |this,
                                                              event: &DragMoveEvent<
                                                            CompositionDrag,
                                                        >,
                                                              _,
                                                              cx| {
                                                            if !event
                                                                .bounds
                                                                .contains(&event.event.position)
                                                            {
                                                                return;
                                                            }
                                                            this.set_drop_slot(
                                                                Some((key.clone(), len)),
                                                                cx,
                                                            );
                                                        }
                                                    }))
                                                    .on_drop({
                                                        let target = drop_target;
                                                        move |drag: &CompositionDrag, window, cx| {
                                                            drop_on_section(
                                                                &explorer,
                                                                &target,
                                                                drag,
                                                                section_len,
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    })
                                                    .when(end_marker, |this| {
                                                        this.child(insertion_marker_overlay(cyan))
                                                    })
                                                    .into_any_element(),
                                            );
                                            rows
                                        })
                                    }))
                                    .into_any_element(),
                            );

                            // New-group inline input after the section that owns `after`.
                            let show_new_after = match &renaming {
                                Some(RenameTarget::NewGroup { after: None }) => {
                                    // Append at end — rendered after the loop.
                                    false
                                }
                                Some(RenameTarget::NewGroup {
                                    after: Some(after_name),
                                }) => group_name.as_deref() == Some(after_name.as_str()),
                                _ => false,
                            };
                            if show_new_after {
                                if let Some(input) = rename_input.clone() {
                                    children.push(new_group_input_row(input, muted, radius));
                                }
                            }
                        }

                        if matches!(
                            &renaming,
                            Some(RenameTarget::NewGroup { after: None })
                        ) {
                            if let Some(input) = rename_input.clone() {
                                children.push(new_group_input_row(input, muted, radius));
                            }
                        }

                        // End gap for group reorder after the last named section.
                        if named_count > 0 {
                            let explorer = explorer.clone();
                            let end_marker = group_drop_slot == Some(named_count);
                            children.push(
                                div()
                                    .id("composition-groups-end-gap")
                                    .relative()
                                    .w_full()
                                    .flex_none()
                                    .h(px(12.))
                                    .can_drop(|data, _, _| accepts_group_drag(data))
                                    .on_drag_move(cx.listener(move |this,
                                                                   event: &DragMoveEvent<GroupDrag>,
                                                                   _,
                                                                   cx| {
                                        if !event.bounds.contains(&event.event.position) {
                                            return;
                                        }
                                        this.set_group_drop_slot(Some(named_count), cx);
                                    }))
                                    .on_drop(move |drag: &GroupDrag, window, cx| {
                                        drop_group(&explorer, drag, named_count, window, cx);
                                    })
                                    .when(end_marker, |this| {
                                        this.child(insertion_marker_overlay(cyan))
                                    })
                                    .into_any_element(),
                            );
                        }

                        children
                    });
                if active_tool == Some(ExplorerTool::Info) {
                    let info_item = info_focus.as_ref().and_then(|(id, _)| {
                        self.items.iter().find(|item| item.id == *id)
                    });
                    let parent_name = info_item.and_then(|item| {
                        let parent_id = item.parent?;
                        self.items
                            .iter()
                            .find(|p| p.id == parent_id)
                            .map(|p| p.name.clone())
                    });
                    let composition_path = match info_focus.as_ref() {
                        Some((_, Some(path))) => {
                            let full = path.display().to_string();
                            let preview = middle_ellipsis(&full, INFO_PATH_CHARS);
                            div()
                                .id("explorer-info-path")
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(muted)
                                .tooltip(move |window, cx| {
                                    Tooltip::new(full.clone()).build(window, cx)
                                })
                                .child(preview)
                                .into_any_element()
                        }
                        Some((_, None)) => div()
                            .text_xs()
                            .text_color(muted)
                            .child("(unsaved)")
                            .into_any_element(),
                        None => div()
                            .text_xs()
                            .text_color(muted)
                            .child("(none)")
                            .into_any_element(),
                    };
                    let mut composition_rows = vec![h_flex()
                        .w_full()
                        .items_start()
                        .gap_1()
                        .child(
                            div()
                                .w(px(44.))
                                .flex_none()
                                .text_xs()
                                .text_color(muted)
                                .child("path"),
                        )
                        .child(composition_path)
                        .into_any_element()];
                    if let Some(name) = parent_name {
                        composition_rows.push(
                            h_flex()
                                .w_full()
                                .items_start()
                                .gap_1()
                                .child(
                                    div()
                                        .w(px(44.))
                                        .flex_none()
                                        .text_xs()
                                        .text_color(muted)
                                        .child("parent"),
                                )
                                .child(
                                    div()
                                        .id("explorer-info-parent")
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(name),
                                )
                                .into_any_element(),
                        );
                    }
                    let info = v_flex()
                        .id("explorer-info-pane")
                        .size_full()
                        .min_h_0()
                        .border_t_1()
                        .border_color(border)
                        .px_1p5()
                        .py_1()
                        .gap_1()
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .font_semibold()
                                        .child("Info"),
                                )
                                .child(
                                    Button::new("explorer-info-close")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .tab_stop(false)
                                        .on_click({
                                            let explorer = explorer.clone();
                                            move |_, _, cx| {
                                                explorer.update(cx, |this, cx| {
                                                    this.close_tool(cx);
                                                });
                                            }
                                        }),
                                ),
                        )
                        .child(
                            v_flex()
                                .w_full()
                                .flex_none()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .child("Composition"),
                                )
                                .children(composition_rows),
                        )
                        .child(
                            v_flex()
                                .id("explorer-info-media")
                                .w_full()
                                .flex_1()
                                .min_h_0()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .child("Media"),
                                )
                                .child(info_media_table(&info_media, muted, border)),
                        );
                    div()
                        .flex_1()
                        .w_full()
                        .min_h_0()
                        .child(
                            v_resizable("explorer-tool-split")
                                .child(
                                    resizable_panel()
                                        .size_range(px(80.)..Pixels::MAX)
                                        .child(list),
                                )
                                .child(
                                    resizable_panel()
                                        .size(px(INFO_PANE_DEFAULT))
                                        .size_range(px(INFO_PANE_MIN)..px(INFO_PANE_MAX))
                                        .flex_none()
                                        .child(info),
                                ),
                        )
                        .into_any_element()
                } else {
                    list.into_any_element()
                }
            })
            .child(
                h_flex()
                    .id("explorer-toolbar")
                    .w_full()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .px_1()
                    .py_0p5()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        Button::new("explorer-tool-info")
                            .ghost()
                            .xsmall()
                            .text_color(muted)
                            .icon(IconName::Info)
                            .selected(active_tool == Some(ExplorerTool::Info))
                            .tab_stop(false)
                            .tooltip("Info")
                            .on_click({
                                let explorer = explorer.clone();
                                move |_, _, cx| {
                                    explorer.update(cx, |this, cx| {
                                        this.toggle_info_tool(cx);
                                    });
                                }
                            }),
                    ),
            )
    }
}

fn info_media_table(
    rows: &[InfoMediaRow],
    muted: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    let header = info_media_row("#", "Path", "Rate", "Ch", "Len", "Size", muted, true, None);
    let body: Vec<gpui_kit::AnyElement> = if rows.is_empty() {
        vec![div()
            .text_xs()
            .text_color(muted)
            .py_0p5()
            .child("(no media)")
            .into_any_element()]
    } else {
        rows.iter()
            .enumerate()
            .map(|(ix, row)| {
                let full = row.path.clone();
                let path = middle_ellipsis(&full, INFO_MEDIA_PATH_CHARS);
                info_media_row(
                    &format!("{}", ix + 1),
                    &path,
                    &row.sample_rate.to_string(),
                    &row.channel_count.to_string(),
                    &format_duration_secs(row.duration_secs),
                    &format_size_bytes(row.size_bytes),
                    muted,
                    false,
                    Some(full),
                )
            })
            .collect()
    };

    v_flex()
        .id("explorer-info-media-table")
        .w_full()
        .flex_1()
        .min_h_0()
        .min_w(px(280.))
        .child(
            div()
                .w_full()
                .flex_none()
                .border_b_1()
                .border_color(border)
                .child(header),
        )
        .child(
            div()
                .id("explorer-info-media-rows")
                .w_full()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .overflow_x_scroll()
                .children(body),
        )
        .into_any_element()
}

fn info_media_row(
    index: &str,
    path: &str,
    rate: &str,
    channels: &str,
    length: &str,
    size: &str,
    muted: gpui_kit::Hsla,
    header: bool,
    path_tooltip: Option<String>,
) -> gpui_kit::AnyElement {
    let path_el = div()
        .id(SharedString::from(format!(
            "explorer-info-media-path-{}",
            if header { "hdr" } else { index }
        )))
        .flex_1()
        .min_w(px(80.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_xs()
        .when(header, |this| this.font_semibold().text_color(muted))
        .when_some(path_tooltip, |this, full| {
            this.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
        })
        .child(path.to_string());

    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .py_0p5()
        .child(
            div()
                .w(px(18.))
                .flex_none()
                .text_xs()
                .when(header, |this| this.font_semibold())
                .text_color(muted)
                .child(index.to_string()),
        )
        .child(path_el)
        .child(
            div()
                .w(px(48.))
                .flex_none()
                .text_xs()
                .when(header, |this| this.font_semibold())
                .text_color(muted)
                .child(rate.to_string()),
        )
        .child(
            div()
                .w(px(24.))
                .flex_none()
                .text_xs()
                .when(header, |this| this.font_semibold())
                .text_color(muted)
                .child(channels.to_string()),
        )
        .child(
            div()
                .w(px(44.))
                .flex_none()
                .text_xs()
                .when(header, |this| this.font_semibold())
                .text_color(muted)
                .child(length.to_string()),
        )
        .child(
            div()
                .w(px(52.))
                .flex_none()
                .text_xs()
                .when(header, |this| this.font_semibold())
                .text_color(muted)
                .child(size.to_string()),
        )
        .into_any_element()
}

fn new_group_input_row(
    input: Entity<InputState>,
    muted: gpui_kit::Hsla,
    radius: Pixels,
) -> gpui_kit::AnyElement {
    h_flex()
        .id("composition-new-group")
        .w_full()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded(radius)
        .child(Icon::new(IconName::ChevronRight).xsmall().text_color(muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&input).xsmall().w_full())
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u128, name: &str, group: Option<&str>) -> ExplorerItem {
        ExplorerItem {
            id: DocumentId::from_u128(id),
            name: name.into(),
            modified: false,
            has_edits: false,
            group: group.map(str::to_string),
            path: None,
            depth: 0,
            parent: None,
            detached: false,
            has_children: false,
        }
    }

    fn section_names(items: &[ExplorerItem], session_groups: &[&str]) -> Vec<(String, Vec<u128>)> {
        let groups: Vec<String> = session_groups.iter().map(|s| (*s).to_string()).collect();
        group_sections(items, &groups)
            .into_iter()
            .map(|section| {
                let label = section.key.label().to_string();
                let ids = section
                    .items
                    .into_iter()
                    .map(|item| item.id.0.as_u128())
                    .collect();
                (label, ids)
            })
            .collect()
    }

    #[test]
    fn ungrouped_only_uses_session_section() {
        let items = [item(1, "a.wav", None), item(2, "b.wav", None)];
        assert_eq!(
            section_names(&items, &[]),
            vec![("session".into(), vec![1, 2])]
        );
    }

    #[test]
    fn session_section_omitted_when_all_grouped() {
        let items = [
            item(1, "a.wav", Some("todo")),
            item(2, "b.wav", Some("todo")),
        ];
        assert_eq!(
            section_names(&items, &["todo"]),
            vec![("todo".into(), vec![1, 2])]
        );
    }

    #[test]
    fn named_groups_follow_session_groups_order() {
        let items = [
            item(1, "keep.wav", None),
            item(2, "a.wav", Some("todo")),
            item(3, "b.wav", Some("done")),
            item(4, "c.wav", Some("todo")),
            item(5, "other.wav", None),
        ];
        // Order comes from session_groups, not first-seen document scan.
        assert_eq!(
            section_names(&items, &["done", "todo"]),
            vec![
                ("session".into(), vec![1, 5]),
                ("done".into(), vec![3]),
                ("todo".into(), vec![2, 4]),
            ]
        );
    }

    #[test]
    fn empty_named_group_appears_in_sections() {
        let items = [item(1, "a.wav", None)];
        assert_eq!(
            section_names(&items, &["empty", "also"]),
            vec![
                ("session".into(), vec![1]),
                ("empty".into(), vec![]),
                ("also".into(), vec![]),
            ]
        );
    }

    #[test]
    fn empty_named_group_alone_still_listed() {
        let items: [ExplorerItem; 0] = [];
        assert_eq!(
            section_names(&items, &["solo"]),
            vec![("solo".into(), vec![])]
        );
    }

    #[test]
    fn named_session_group_is_distinct_from_ungrouped() {
        let items = [
            item(1, "plain.wav", None),
            item(2, "named.wav", Some("session")),
        ];
        let groups = vec!["session".to_string()];
        let keys: Vec<_> = group_sections(&items, &groups)
            .into_iter()
            .map(|section| section.key)
            .collect();
        assert_eq!(
            keys,
            vec![SectionKey::Session, SectionKey::Named("session".into()),]
        );
    }

    #[test]
    fn section_key_group_name_maps_session_to_none() {
        assert_eq!(SectionKey::Session.group_name(), None);
        assert_eq!(
            SectionKey::Named("todo".into()).group_name().as_deref(),
            Some("todo")
        );
    }

    #[test]
    fn slot_before_uses_row_midpoint() {
        let bounds = Bounds {
            origin: gpui_kit::point(px(0.), px(10.)),
            size: gpui_kit::size(px(100.), px(20.)),
        };
        assert!(slot_before_from_y(px(15.), bounds));
        assert!(!slot_before_from_y(px(25.), bounds));
    }

    #[test]
    fn idle_clears_stale_reorder_drop_slots() {
        let mut drop_slot = Some((SectionKey::Session, 1));
        let mut group_drop_slot = Some(0usize);
        let mut active = true;
        let (shown, group, live) =
            visible_reorder_drop_slots(false, &mut drop_slot, &mut group_drop_slot, &mut active);
        assert!(shown.is_none());
        assert!(group.is_none());
        assert!(!live);
        assert!(drop_slot.is_none());
        assert!(group_drop_slot.is_none());
        assert!(!active);
    }

    #[test]
    fn foreign_active_drag_hides_stale_reorder_marker() {
        // Dock splitter resize also sets has_active_drag; without an armed
        // reorder move the insertion marker must stay hidden.
        let mut drop_slot = Some((SectionKey::Session, 2));
        let mut group_drop_slot = Some(1usize);
        let mut active = false;
        let (shown, group, live) =
            visible_reorder_drop_slots(true, &mut drop_slot, &mut group_drop_slot, &mut active);
        assert!(shown.is_none());
        assert!(group.is_none());
        assert!(!live);
        // Stored slot kept for a possible later composition drop read, but
        // not painted while a non-reorder drag is active.
        assert_eq!(drop_slot, Some((SectionKey::Session, 2)));
        assert_eq!(group_drop_slot, Some(1));
    }

    #[test]
    fn armed_reorder_move_shows_drop_slots_once() {
        let mut drop_slot = Some((SectionKey::Named("todo".into()), 0));
        let mut group_drop_slot = None;
        let mut active = true;
        let (shown, group, live) =
            visible_reorder_drop_slots(true, &mut drop_slot, &mut group_drop_slot, &mut active);
        assert_eq!(shown, Some((SectionKey::Named("todo".into()), 0)));
        assert!(group.is_none());
        assert!(live);
        assert!(!active);
        // Second paint without another on_drag_move hides the marker.
        let (shown, _, live) =
            visible_reorder_drop_slots(true, &mut drop_slot, &mut group_drop_slot, &mut active);
        assert!(shown.is_none());
        assert!(!live);
    }

    #[test]
    fn index_after_remove_shifts_when_dropping_below_source() {
        assert_eq!(index_in_group_after_remove(0, Some(2)), 0);
        assert_eq!(index_in_group_after_remove(2, Some(2)), 2);
        assert_eq!(index_in_group_after_remove(3, Some(1)), 2);
        assert_eq!(index_in_group_after_remove(1, None), 1);
        // End gap (drop_before == len) for a 3-item section.
        assert_eq!(index_in_group_after_remove(3, Some(0)), 2);
        assert_eq!(index_in_group_after_remove(3, Some(2)), 2);
    }

    #[test]
    fn middle_ellipsis_keeps_short_strings() {
        assert_eq!(middle_ellipsis("short", 10), "short");
    }

    #[test]
    fn middle_ellipsis_shows_head_and_tail() {
        let path = "/Users/me/projects/FieldAssist/takes/long-name.facomp";
        let shown = middle_ellipsis(path, 24);
        assert!(shown.contains('…'));
        assert!(shown.len() <= 24 + 3); // ellipsis is one char but may be multi-byte
        assert!(shown.starts_with("/Users"));
        assert!(shown.ends_with(".facomp"));
    }

    #[test]
    fn format_duration_and_size() {
        assert_eq!(format_duration_secs(1.234), "1.23s");
        assert_eq!(format_duration_secs(12.34), "12.3s");
        assert_eq!(format_duration_secs(120.0), "120s");
        assert_eq!(format_size_bytes(500), "500 B");
        assert_eq!(format_size_bytes(1536), "1.5 KB");
        assert_eq!(format_size_bytes(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn collapse_hides_attached_descendants_only() {
        let parent = ExplorerItem {
            id: DocumentId::from_u128(1),
            name: "parent".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 0,
            parent: None,
            detached: false,
            has_children: true,
        };
        let child = ExplorerItem {
            id: DocumentId::from_u128(2),
            name: "child".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 1,
            parent: Some(DocumentId::from_u128(1)),
            detached: false,
            has_children: false,
        };
        let other = ExplorerItem {
            id: DocumentId::from_u128(3),
            name: "other".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 0,
            parent: None,
            detached: false,
            has_children: false,
        };
        let detached = ExplorerItem {
            id: DocumentId::from_u128(4),
            name: "detached".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 1,
            parent: Some(DocumentId::from_u128(1)),
            detached: true,
            has_children: false,
        };
        let items = [&parent, &child, &other, &detached];
        let collapsed = HashSet::from([DocumentId::from_u128(1)]);
        let hidden = hidden_under_collapsed(&items, &collapsed);
        assert!(hidden.contains(&DocumentId::from_u128(2)));
        assert!(!hidden.contains(&DocumentId::from_u128(3)));
        assert!(!hidden.contains(&DocumentId::from_u128(4)));
    }

    #[test]
    fn collapse_nested_break_out_hides_grandchild_with_child() {
        let root = ExplorerItem {
            id: DocumentId::from_u128(1),
            name: "root".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 0,
            parent: None,
            detached: false,
            has_children: true,
        };
        let child = ExplorerItem {
            id: DocumentId::from_u128(2),
            name: "child".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 1,
            parent: Some(DocumentId::from_u128(1)),
            detached: false,
            has_children: true,
        };
        let grand = ExplorerItem {
            id: DocumentId::from_u128(3),
            name: "grand".into(),
            modified: false,
            has_edits: false,
            group: None,
            path: None,
            depth: 2,
            parent: Some(DocumentId::from_u128(2)),
            detached: false,
            has_children: false,
        };
        let items = [&root, &child, &grand];
        let collapsed_root = HashSet::from([DocumentId::from_u128(1)]);
        let hidden_root = hidden_under_collapsed(&items, &collapsed_root);
        assert!(hidden_root.contains(&DocumentId::from_u128(2)));
        assert!(hidden_root.contains(&DocumentId::from_u128(3)));

        let collapsed_child = HashSet::from([DocumentId::from_u128(2)]);
        let hidden_child = hidden_under_collapsed(&items, &collapsed_child);
        assert!(!hidden_child.contains(&DocumentId::from_u128(2)));
        assert!(hidden_child.contains(&DocumentId::from_u128(3)));
    }
}
