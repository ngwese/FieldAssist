// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    v_flex, ActiveTheme as _, Colorize as _, Icon, IconName, Sizable as _,
};
use gpui_kit::{
    actions, div, prelude::FluentBuilder as _, px, App, AppContext as _, Bounds, ClickEvent,
    Context, DragMoveEvent, Entity, EventEmitter, ExternalDragPayload, FileDragPaths, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, ParentElement as _,
    Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::model::DocumentId;

actions!(explorer, [ConfirmSelected, SelectPrev, SelectNext]);

const CONTEXT: &str = "Compositions";
const SESSION_LABEL: &str = "session";

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
}

#[derive(Clone, PartialEq)]
struct ExplorerItem {
    id: DocumentId,
    name: SharedString,
    modified: bool,
    group: Option<String>,
    path: Option<PathBuf>,
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

struct ExplorerSection<'a> {
    key: SectionKey,
    items: Vec<&'a ExplorerItem>,
}

fn group_sections(items: &[ExplorerItem]) -> Vec<ExplorerSection<'_>> {
    let mut sections = Vec::new();
    let session: Vec<&ExplorerItem> = items.iter().filter(|item| item.group.is_none()).collect();
    if !session.is_empty() {
        sections.push(ExplorerSection {
            key: SectionKey::Session,
            items: session,
        });
    }
    let mut named_order: Vec<String> = Vec::new();
    let mut named: HashMap<String, Vec<&ExplorerItem>> = HashMap::new();
    for item in items {
        let Some(name) = item.group.as_ref() else {
            continue;
        };
        if !named.contains_key(name) {
            named_order.push(name.clone());
        }
        named.entry(name.clone()).or_default().push(item);
    }
    for name in named_order {
        if let Some(group_items) = named.remove(&name) {
            sections.push(ExplorerSection {
                key: SectionKey::Named(name),
                items: group_items,
            });
        }
    }
    sections
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
    active: Option<DocumentId>,
    selected: Option<DocumentId>,
    hovered_close: Option<DocumentId>,
    /// Insertion gap while dragging: before index `0..=section.len`.
    drop_slot: Option<(SectionKey, usize)>,
    collapsed: HashSet<SectionKey>,
    on_event: Option<EventHandler>,
    focus_handle: FocusHandle,
}

impl ExplorerPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("enter", ConfirmSelected, Some(CONTEXT)),
            KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
            KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        ]);
        Self {
            items: Vec::new(),
            active: None,
            selected: None,
            hovered_close: None,
            drop_slot: None,
            collapsed: HashSet::new(),
            on_event: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_handler(&mut self, handler: EventHandler) {
        self.on_event = Some(handler);
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
            Option<String>,
            Option<PathBuf>,
        )],
        active: Option<DocumentId>,
        cx: &mut Context<Self>,
    ) {
        let items: Vec<ExplorerItem> = docs
            .iter()
            .map(|(id, name, modified, group, path)| ExplorerItem {
                id: *id,
                name: name.clone(),
                modified: *modified,
                group: group.clone(),
                path: path.clone(),
            })
            .collect();
        if items == self.items && active == self.active {
            return;
        }
        let keep = self
            .selected
            .filter(|id| items.iter().any(|item| item.id == *id));
        self.items = items;
        self.active = active;
        self.selected = keep.or(active);
        let live: HashSet<_> = group_sections(&self.items)
            .into_iter()
            .map(|section| section.key)
            .collect();
        self.collapsed.retain(|key| live.contains(key));
        cx.notify();
    }

    fn selected_or_active(&self) -> Option<DocumentId> {
        self.selected.or(self.active)
    }

    fn visible_ids(&self) -> Vec<DocumentId> {
        group_sections(&self.items)
            .into_iter()
            .filter(|section| !self.collapsed.contains(&section.key))
            .flat_map(|section| section.items.into_iter().map(|item| item.id))
            .collect()
    }

    fn select_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
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
        if self.drop_slot != slot {
            self.drop_slot = slot;
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

fn drop_on_section(
    explorer: &Entity<ExplorerPanel>,
    target: &SectionKey,
    drag: &CompositionDrag,
    section_len: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let drop_before = explorer
        .read(cx)
        .drop_slot
        .as_ref()
        .filter(|(key, _)| key == target)
        .map(|(_, before)| *before)
        .unwrap_or(section_len);
    let section_ids: Vec<DocumentId> = explorer
        .read(cx)
        .items
        .iter()
        .filter(|item| section_key_for_group(item.group.as_deref()) == *target)
        .map(|item| item.id)
        .collect();
    let dragged_in_section = section_ids.iter().position(|id| *id == drag.id);
    let index = index_in_group_after_remove(drop_before, dragged_in_section);
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

fn section_key_for_group(group: Option<&str>) -> SectionKey {
    match group {
        Some(name) => SectionKey::Named(name.to_string()),
        None => SectionKey::Session,
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let explorer = cx.entity();
        let hovered_close = self.hovered_close;
        let selected = self.selected;
        let active = self.active;
        let collapsed = self.collapsed.clone();
        let highlight_bg = ghost_hover_bg(cx);
        let muted = cx.theme().muted_foreground;
        let cyan = cx.theme().cyan;
        let radius = cx.theme().radius;
        let sections = group_sections(&self.items);
        let drop_slot = if cx.has_active_drag() {
            self.drop_slot.clone()
        } else {
            None
        };

        div()
            .id("compositions-list")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .on_action({
                let explorer = explorer.clone();
                move |_: &ConfirmSelected, window, cx| {
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
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                this.select_delta(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.select_delta(1, cx);
            }))
            .children(sections.into_iter().map(|section| {
                let open = !collapsed.contains(&section.key);
                let explorer = explorer.clone();
                let header_key = section.key.clone();
                let drop_key = section.key.clone();
                let section_len = section.items.len();
                let show_slot = drop_slot
                    .as_ref()
                    .filter(|(key, _)| key == &section.key)
                    .map(|(_, before)| *before);
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
                            drop_on_section(&explorer, &target, drag, section_len, window, cx);
                        }
                    })
                    .child({
                        let explorer = explorer.clone();
                        h_flex()
                            .id(SharedString::from(format!(
                                "{}-header",
                                section.key.element_id()
                            )))
                            .w_full()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .px_1p5()
                            .py_0p5()
                            .cursor_pointer()
                            .can_drop(|data, _, _| accepts_composition_drag(data))
                            .on_drag_move(cx.listener({
                                let key = drop_key.clone();
                                move |this, event: &DragMoveEvent<CompositionDrag>, _, cx| {
                                    if !event.bounds.contains(&event.event.position) {
                                        return;
                                    }
                                    this.set_drop_slot(Some((key.clone(), 0)), cx);
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
                            .child(
                                Icon::new(if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall()
                                .text_color(muted),
                            )
                            .child(div().text_xs().text_color(muted).child(section.key.label()))
                            .on_click(move |_, _, cx| {
                                explorer.update(cx, |this, cx| {
                                    this.toggle_section(header_key.clone(), cx);
                                });
                            })
                    })
                    .children(open.then(|| {
                        let explorer = explorer.clone();
                        let drop_key = drop_key.clone();
                        let items: Vec<ExplorerItem> = section.items.into_iter().cloned().collect();
                        v_flex().w_full().children({
                            let mut rows = Vec::with_capacity(items.len() + 1);
                            for (index, item) in items.into_iter().enumerate() {
                                let id = item.id;
                                let name = item.name.clone();
                                let path = item.path.clone();
                                let is_modified = item.modified;
                                let highlighted = selected == Some(id);
                                let is_active = active == Some(id);
                                let show_close = hovered_close == Some(id);
                                let explorer = explorer.clone();
                                let drop_target = drop_key.clone();
                                let marker_top = show_slot == Some(index);
                                rows.push(
                                    h_flex()
                                        .id(SharedString::from(format!("composition-{id}")))
                                        .relative()
                                        .w_full()
                                        .flex_none()
                                        .items_center()
                                        .pl_5()
                                        .pr_1p5()
                                        .py_0p5()
                                        .rounded(radius)
                                        .text_xs()
                                        .cursor_pointer()
                                        .when(highlighted, |this| this.bg(highlight_bg))
                                        .when(!highlighted, |this| {
                                            this.hover(|this| this.bg(highlight_bg))
                                        })
                                        .when(marker_top, |this| {
                                            this.child(insertion_marker_overlay(cyan))
                                        })
                                        .on_drag(
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
                                        .external_drag_payload(|drag: &CompositionDrag, _, _| {
                                            drag.path.as_ref().map(|path| {
                                                ExternalDragPayload::Files(FileDragPaths::new([(
                                                    path.clone(),
                                                    false,
                                                )]))
                                            })
                                        })
                                        .can_drop(|data, _, _| accepts_composition_drag(data))
                                        .on_drag_move(cx.listener({
                                            let key = drop_target.clone();
                                            move |this,
                                                  event: &DragMoveEvent<CompositionDrag>,
                                                  _,
                                                  cx| {
                                                if !event.bounds.contains(&event.event.position) {
                                                    return;
                                                }
                                                let before = slot_before_from_y(
                                                    event.event.position.y,
                                                    event.bounds,
                                                );
                                                let slot = if before { index } else { index + 1 };
                                                this.set_drop_slot(Some((key.clone(), slot)), cx);
                                            }
                                        }))
                                        .on_drop({
                                            let explorer = explorer.clone();
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
                                        .on_click({
                                            let explorer = explorer.clone();
                                            move |event: &ClickEvent, window, cx| {
                                                explorer.update(cx, |this, cx| {
                                                    this.selected = Some(id);
                                                    cx.notify();
                                                });
                                                let event = if event.click_count() >= 2 {
                                                    ExplorerEvent::OpenTab(id)
                                                } else {
                                                    ExplorerEvent::Activate(id)
                                                };
                                                dispatch(&explorer, event, window, cx);
                                            }
                                        })
                                        .context_menu({
                                            let explorer = explorer.clone();
                                            move |menu, _, _| {
                                                menu.item(PopupMenuItem::new("Close").on_click({
                                                    let explorer = explorer.clone();
                                                    move |_, window, cx| {
                                                        dispatch(
                                                            &explorer,
                                                            ExplorerEvent::Close(id),
                                                            window,
                                                            cx,
                                                        );
                                                    }
                                                }))
                                            }
                                        })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_xs()
                                                .when(is_active, |this| this.text_color(cyan))
                                                .child(name),
                                        )
                                        .child({
                                            let explorer = explorer.clone();
                                            div()
                                                .id(SharedString::from(format!("comp-eol-{id}")))
                                                .w(px(18.))
                                                .h(px(18.))
                                                .flex()
                                                .flex_none()
                                                .items_center()
                                                .justify_center()
                                                .on_hover({
                                                    let explorer = explorer.clone();
                                                    move |hovered: &bool, _, cx| {
                                                        explorer.update(cx, |this, cx| {
                                                            this.hovered_close = if *hovered {
                                                                Some(id)
                                                            } else {
                                                                None
                                                            };
                                                            cx.notify();
                                                        });
                                                    }
                                                })
                                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                    cx.stop_propagation();
                                                })
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
                                                        Button::new(SharedString::from(format!(
                                                            "close-comp-{id}"
                                                        )))
                                                        .ghost()
                                                        .xsmall()
                                                        .icon(IconName::Close)
                                                        .tab_stop(false)
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
                                                        }),
                                                    )
                                                })
                                                .when(!show_close && is_modified, |this| {
                                                    this.child(
                                                        div().size(px(6.)).rounded_full().bg(muted),
                                                    )
                                                })
                                        })
                                        .into_any_element(),
                                );
                            }
                            // Dedicated end gap: the last-row bottom marker used to sit
                            // outside the row hitbox (bottom: -1), so releasing on it
                            // missed every drop target and the reorder looked like a
                            // no-op / off-by-one at the end of the list.
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
                                    .can_drop(|data, _, _| accepts_composition_drag(data))
                                    .on_drag_move(cx.listener({
                                        let key = drop_target.clone();
                                        let len = section_len;
                                        move |this,
                                              event: &DragMoveEvent<CompositionDrag>,
                                              _,
                                              cx| {
                                            if !event.bounds.contains(&event.event.position) {
                                                return;
                                            }
                                            this.set_drop_slot(Some((key.clone(), len)), cx);
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
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u128, name: &str, group: Option<&str>) -> ExplorerItem {
        ExplorerItem {
            id: DocumentId::from_u128(id),
            name: name.into(),
            modified: false,
            group: group.map(str::to_string),
            path: None,
        }
    }

    fn section_names(items: &[ExplorerItem]) -> Vec<(String, Vec<u128>)> {
        group_sections(items)
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
        assert_eq!(section_names(&items), vec![("session".into(), vec![1, 2])]);
    }

    #[test]
    fn session_section_omitted_when_all_grouped() {
        let items = [
            item(1, "a.wav", Some("todo")),
            item(2, "b.wav", Some("todo")),
        ];
        assert_eq!(section_names(&items), vec![("todo".into(), vec![1, 2])]);
    }

    #[test]
    fn named_groups_follow_session_in_first_seen_order() {
        let items = [
            item(1, "keep.wav", None),
            item(2, "a.wav", Some("todo")),
            item(3, "b.wav", Some("done")),
            item(4, "c.wav", Some("todo")),
            item(5, "other.wav", None),
        ];
        assert_eq!(
            section_names(&items),
            vec![
                ("session".into(), vec![1, 5]),
                ("todo".into(), vec![2, 4]),
                ("done".into(), vec![3]),
            ]
        );
    }

    #[test]
    fn named_session_group_is_distinct_from_ungrouped() {
        let items = [
            item(1, "plain.wav", None),
            item(2, "named.wav", Some("session")),
        ];
        let keys: Vec<_> = group_sections(&items)
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
    fn index_after_remove_shifts_when_dropping_below_source() {
        assert_eq!(index_in_group_after_remove(0, Some(2)), 0);
        assert_eq!(index_in_group_after_remove(2, Some(2)), 2);
        assert_eq!(index_in_group_after_remove(3, Some(1)), 2);
        assert_eq!(index_in_group_after_remove(1, None), 1);
        // End gap (drop_before == len) for a 3-item section.
        assert_eq!(index_in_group_after_remove(3, Some(0)), 2);
        assert_eq!(index_in_group_after_remove(3, Some(2)), 2);
    }
}
