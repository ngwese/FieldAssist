// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Scripting search-path table for the Settings window.

use std::path::{Path, PathBuf};

use field_ui_components::content_foreground;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex, ActiveTheme as _, Disableable as _, IconName, Sizable as _, Theme,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, uniform_list, App, AppContext as _, Bounds, Context, DragMoveEvent, Entity,
    ExternalPaths, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    PathPromptOptions, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};

use crate::settings;

const ROW_HEIGHT: f32 = 26.;
const BODY_HEIGHT: f32 = 180.;
const COL_ROW_WIDTH: f32 = 36.;
const COL_BROWSE_WIDTH: f32 = 28.;
const END_DROP_MIN: f32 = 20.;

/// Interactive search-path table (config dir locked at row 0).
pub struct ScriptSearchPathEditor {
    /// Extra folders after the locked config directory.
    extras: Vec<String>,
    /// Selected UI row index (`0` = locked config).
    selected: Option<usize>,
    editing: Option<CellEdit>,
    /// Extra index while a native folder picker is open (blocks settings sync).
    browsing: Option<usize>,
    /// Insertion index among extras while reordering (`0..=extras.len()`).
    drop_slot: Option<usize>,
    config_dir: String,
}

struct CellEdit {
    /// Index into [`ScriptSearchPathEditor::extras`].
    extra_ix: usize,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// Drag payload for reordering editable search-path rows.
#[derive(Clone, Debug)]
struct SearchPathDrag {
    /// Index into extras (`0..extras.len()`).
    extra_ix: usize,
}

impl Render for SearchPathDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .text_xs()
            .bg(cx.theme().secondary)
            .text_color(cx.theme().foreground)
            .rounded(cx.theme().radius)
            .shadow_md()
            .child(format!("path {}", self.extra_ix + 2))
    }
}

impl ScriptSearchPathEditor {
    /// Create an editor seeded from the current settings store.
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        settings::ensure_store(cx);
        let config_dir = crate::commands::user_config_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(config directory unavailable)".into());
        let extras = settings::store(cx).settings.scripting.search_path.clone();
        Self {
            extras,
            selected: None,
            editing: None,
            browsing: None,
            drop_slot: None,
            config_dir,
        }
    }

    fn row_count(&self) -> usize {
        1 + self.extras.len()
    }

    fn set_drop_slot(&mut self, slot: Option<usize>, cx: &mut Context<Self>) {
        if self.drop_slot != slot {
            self.drop_slot = slot;
            cx.notify();
        }
    }

    fn clear_drop_slot(&mut self, cx: &mut Context<Self>) {
        self.set_drop_slot(None, cx);
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        let extras = self.extras.clone();
        let config = crate::commands::user_config_dir();
        let _ = settings::update_and_save(cx, |s| {
            s.scripting.search_path = extras;
            s.scripting.normalize(config.as_deref());
        });
        self.extras = settings::store(cx).settings.scripting.search_path.clone();
    }

    fn discard_edit(&mut self) {
        self.editing = None;
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        let value = edit.input.read(cx).value().to_string();
        if let Some(slot) = self.extras.get_mut(edit.extra_ix) {
            *slot = value;
        }
        self.persist(cx);
        cx.notify();
    }

    fn begin_edit(&mut self, extra_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let value = self.extras.get(extra_ix).cloned().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(value, window, cx);
            state
        });
        let entity = cx.entity().clone();
        let subscription = cx.subscribe_in(&input, window, {
            move |_this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    let value = input.read(cx).value().to_string();
                    entity.update(cx, |this, cx| {
                        // Folder browse steals focus and would otherwise commit
                        // an empty row, which normalize then drops.
                        if this.browsing.is_some() {
                            this.discard_edit();
                            cx.notify();
                            return;
                        }
                        if let Some(edit) = this.editing.take() {
                            if let Some(slot) = this.extras.get_mut(edit.extra_ix) {
                                *slot = value;
                            }
                        }
                        this.persist(cx);
                        cx.notify();
                    });
                }
            }
        });
        self.selected = Some(extra_ix + 1);
        self.editing = Some(CellEdit {
            extra_ix,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        self.extras.push(String::new());
        let extra_ix = self.extras.len() - 1;
        // Do not persist yet — normalize would drop the blank row.
        self.begin_edit(extra_ix, window, cx);
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(selected) = self.selected else {
            return;
        };
        if selected == 0 {
            return;
        }
        let extra_ix = selected - 1;
        if extra_ix >= self.extras.len() {
            return;
        }
        self.extras.remove(extra_ix);
        self.selected = None;
        self.persist(cx);
        cx.notify();
    }

    fn reorder_to_slot(&mut self, from: usize, slot: usize, cx: &mut Context<Self>) {
        let len = self.extras.len();
        if from >= len || slot > len {
            self.clear_drop_slot(cx);
            return;
        }
        // Dropping immediately before or after itself is a no-op.
        if slot == from || slot == from + 1 {
            self.clear_drop_slot(cx);
            return;
        }
        self.commit_edit(cx);
        let item = self.extras.remove(from);
        let insert_at = if from < slot { slot - 1 } else { slot };
        self.extras.insert(insert_at, item);
        self.selected = Some(insert_at + 1);
        self.drop_slot = None;
        self.persist(cx);
        cx.notify();
    }

    fn set_extra_path(&mut self, extra_ix: usize, path: PathBuf, cx: &mut Context<Self>) {
        self.discard_edit();
        self.browsing = None;
        while self.extras.len() <= extra_ix {
            self.extras.push(String::new());
        }
        if let Some(slot) = self.extras.get_mut(extra_ix) {
            *slot = path.display().to_string();
        }
        self.selected = Some(extra_ix + 1);
        self.persist(cx);
        cx.notify();
    }

    fn append_dropped_dirs(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let config = crate::commands::user_config_dir();
        for path in paths {
            if !path.is_dir() {
                continue;
            }
            let text = path.display().to_string();
            if config.as_ref().is_some_and(|c| path_eq(path, c)) {
                continue;
            }
            if self
                .extras
                .iter()
                .any(|e| e == &text || path_eq(Path::new(e), path))
            {
                continue;
            }
            self.extras.push(text);
        }
        self.persist(cx);
        cx.notify();
    }

    fn browse_folder(&mut self, extra_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Keep the blank slot alive across focus loss + settings sync.
        if self
            .editing
            .as_ref()
            .is_some_and(|e| e.extra_ix == extra_ix)
        {
            self.discard_edit();
        } else {
            self.commit_edit(cx);
        }
        while self.extras.len() <= extra_ix {
            self.extras.push(String::new());
        }
        self.browsing = Some(extra_ix);
        self.selected = Some(extra_ix + 1);
        cx.notify();

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        let entity = cx.entity().clone();
        cx.spawn_in(window, async move |_, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => {
                    let _ = cx.update(|_, cx| {
                        entity.update(cx, |this, cx| {
                            this.browsing = None;
                            cx.notify();
                        });
                    });
                    return;
                }
            };
            let Some(path) = paths.into_iter().next() else {
                let _ = cx.update(|_, cx| {
                    entity.update(cx, |this, cx| {
                        this.browsing = None;
                        cx.notify();
                    });
                });
                return;
            };
            let _ = cx.update(|_, cx| {
                entity.update(cx, |this, cx| {
                    this.set_extra_path(extra_ix, path, cx);
                });
            });
        })
        .detach();
    }
}

fn path_eq(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn render_header(muted: Hsla, border: Hsla) -> impl IntoElement {
    h_flex()
        .id("script-search-header")
        .w_full()
        .flex_none()
        .items_center()
        .px_1p5()
        .py_0p5()
        .border_b_1()
        .border_color(border)
        .child(
            div()
                .w(px(COL_ROW_WIDTH))
                .flex_none()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(muted)
                .child("#"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(muted)
                .child("path"),
        )
        .child(div().w(px(COL_BROWSE_WIDTH)).flex_none())
}

fn render_footer(
    can_remove: bool,
    border: Hsla,
    entity: Entity<ScriptSearchPathEditor>,
) -> impl IntoElement {
    h_flex()
        .id("script-search-footer")
        .w_full()
        .flex_none()
        .items_center()
        .justify_end()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .border_t_1()
        .border_color(border)
        .child(
            Button::new("script-search-add")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .tooltip("Add folder")
                .on_click({
                    let entity = entity.clone();
                    move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.add_row(window, cx);
                        });
                    }
                }),
        )
        .child(
            Button::new("script-search-remove")
                .ghost()
                .xsmall()
                .icon(IconName::Minus)
                .tooltip("Remove selected")
                .disabled(!can_remove)
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.remove_selected(cx);
                        });
                    }
                }),
        )
}

fn slot_before_from_y(y: Pixels, bounds: Bounds<Pixels>) -> bool {
    y < bounds.center().y
}

/// Overlay line that does not consume layout height.
fn insertion_marker_overlay(color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(px(6.))
        .right(px(6.))
        .top(px(-1.))
        .h(px(2.))
        .rounded_full()
        .bg(color)
}

fn insertion_marker_bottom(color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(px(6.))
        .right(px(6.))
        .bottom(px(-1.))
        .h(px(2.))
        .rounded_full()
        .bg(color)
}

fn render_row(
    ui_ix: usize,
    path_text: SharedString,
    locked: bool,
    is_selected: bool,
    selected_bg: Hsla,
    muted: Hsla,
    hover_bg: Hsla,
    marker_color: Hsla,
    drop_slot: Option<usize>,
    extras_len: usize,
    editing_input: Option<Entity<InputState>>,
    entity: Entity<ScriptSearchPathEditor>,
) -> gpui_kit::AnyElement {
    let extra_ix = ui_ix.saturating_sub(1);
    let show_before = !locked && drop_slot == Some(extra_ix);
    let show_after = !locked && extra_ix + 1 == extras_len && drop_slot == Some(extras_len);

    let mut row = h_flex()
        .id(SharedString::from(format!("script-search-row-{ui_ix}")))
        .relative()
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .items_center()
        .px_1p5()
        .gap_1();
    if is_selected {
        row = row.bg(selected_bg);
    }

    row = row.on_mouse_down(MouseButton::Left, {
        let entity = entity.clone();
        move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.selected = Some(ui_ix);
                this.clear_drop_slot(cx);
                cx.notify();
            });
        }
    });

    if !locked {
        row = row
            .can_drop(|data, _, _| data.downcast_ref::<SearchPathDrag>().is_some())
            .on_drag_move({
                let entity = entity.clone();
                move |event: &DragMoveEvent<SearchPathDrag>, _, cx| {
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    let before = slot_before_from_y(event.event.position.y, event.bounds);
                    let slot = if before { extra_ix } else { extra_ix + 1 };
                    entity.update(cx, |this, cx| {
                        this.set_drop_slot(Some(slot), cx);
                    });
                }
            })
            .on_drop({
                let entity = entity.clone();
                move |drag: &SearchPathDrag, _, cx| {
                    entity.update(cx, |this, cx| {
                        let slot = this.drop_slot.unwrap_or(extra_ix);
                        this.reorder_to_slot(drag.extra_ix, slot, cx);
                    });
                }
            });
    }

    let path_cell = if locked {
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_xs()
            .text_color(muted)
            .child(path_text)
            .into_any_element()
    } else if let Some(input) = editing_input {
        div()
            .flex_1()
            .min_w_0()
            .child(Input::new(&input).xsmall().w_full())
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .into_any_element()
    } else {
        let empty = path_text.is_empty();
        div()
            .id(SharedString::from(format!(
                "script-search-path-text-{ui_ix}"
            )))
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_xs()
            .cursor_text()
            .when(empty, |this| this.text_color(muted))
            .child(if empty {
                SharedString::from("(empty)")
            } else {
                path_text
            })
            .on_click({
                let entity = entity.clone();
                move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.begin_edit(extra_ix, window, cx);
                    });
                }
            })
            .into_any_element()
    };

    let number_cell = if locked {
        div()
            .w(px(COL_ROW_WIDTH))
            .flex_none()
            .text_xs()
            .text_color(muted)
            .child((ui_ix + 1).to_string())
            .into_any_element()
    } else {
        div()
            .id(SharedString::from(format!("script-search-handle-{ui_ix}")))
            .w(px(COL_ROW_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .text_xs()
            .text_color(muted)
            .cursor_grab()
            .rounded(px(3.))
            .hover(|this| this.bg(hover_bg))
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new("Drag to reorder").build(window, cx)
            })
            .on_drag(SearchPathDrag { extra_ix }, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child((ui_ix + 1).to_string())
            .into_any_element()
    };

    let browse_cell = if locked {
        div().w(px(COL_BROWSE_WIDTH)).flex_none().into_any_element()
    } else {
        div()
            .w(px(COL_BROWSE_WIDTH))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                Button::new(("script-search-browse", ui_ix as u64))
                    .ghost()
                    .xsmall()
                    .icon(IconName::FolderOpen)
                    .tooltip("Browse")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.browse_folder(extra_ix, window, cx);
                            });
                        }
                    }),
            )
            .into_any_element()
    };

    row.when(show_before, |this| {
        this.child(insertion_marker_overlay(marker_color))
    })
    .when(show_after, |this| {
        this.child(insertion_marker_bottom(marker_color))
    })
    .child(number_cell)
    .child(path_cell)
    .child(browse_cell)
    .into_any_element()
}

fn render_end_drop_zone(
    extras_len: usize,
    drop_slot: Option<usize>,
    marker_color: Hsla,
    entity: Entity<ScriptSearchPathEditor>,
) -> impl IntoElement {
    let show_marker = drop_slot == Some(extras_len) && extras_len > 0;
    div()
        .id("script-search-end-drop")
        .relative()
        .flex_1()
        .min_h(px(END_DROP_MIN))
        .w_full()
        .can_drop(|data, _, _| data.downcast_ref::<SearchPathDrag>().is_some())
        .on_drag_move({
            let entity = entity.clone();
            move |event: &DragMoveEvent<SearchPathDrag>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                entity.update(cx, |this, cx| {
                    this.set_drop_slot(Some(extras_len), cx);
                });
            }
        })
        .on_drop({
            let entity = entity.clone();
            move |drag: &SearchPathDrag, _, cx| {
                entity.update(cx, |this, cx| {
                    this.reorder_to_slot(drag.extra_ix, extras_len, cx);
                });
            }
        })
        .when(show_marker, |this| {
            this.child(insertion_marker_overlay(marker_color))
        })
}

impl Render for ScriptSearchPathEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Stay aligned with Settings reset / external settings.json writes.
        // Skip while editing or browsing so an unpersisted blank row is not wiped.
        if self.editing.is_none() && self.browsing.is_none() {
            let stored = settings::store(cx).settings.scripting.search_path.clone();
            if self.extras != stored {
                self.extras = stored;
                if let Some(sel) = self.selected {
                    if sel >= self.row_count() {
                        self.selected = None;
                    }
                }
            }
        }
        let content = content_foreground(cx);
        Theme::global_mut(cx).foreground = content;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let selected_bg = cx.theme().list_active;
        let drop_highlight = cx.theme().secondary;
        let entity = cx.entity().clone();
        let count = self.row_count();
        let config_dir = SharedString::from(self.config_dir.clone());
        let extras: Vec<SharedString> = self
            .extras
            .iter()
            .map(|s| SharedString::from(s.clone()))
            .collect();
        let selected = self.selected;
        let editing_extra = self.editing.as_ref().map(|e| e.extra_ix);
        let editing_input = self.editing.as_ref().map(|e| e.input.clone());
        let can_remove = self.selected.is_some_and(|ix| ix > 0);
        let drop_slot = self.drop_slot;
        let extras_len = self.extras.len();
        let hover_bg = cx.theme().secondary;
        let marker_color = cx.theme().accent;

        v_flex()
            .id("script-search-path")
            .w_full()
            .gap_1()
            .child(div().text_xs().text_color(muted).child(
                "Extends Lua package.path / package.cpath and folders searched \
                         for resolver_*.lua and workflow_*.lua (path extras apply \
                         on next launch).",
            ))
            .child(
                v_flex()
                    .w_full()
                    .border_1()
                    .border_color(border)
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .bg(cx.theme().background)
                    .can_drop(|data, _, _| data.downcast_ref::<ExternalPaths>().is_some())
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_highlight))
                    .on_drop({
                        let entity = entity.clone();
                        move |paths: &ExternalPaths, _, cx| {
                            let paths = paths.paths().to_vec();
                            entity.update(cx, |this, cx| {
                                this.append_dropped_dirs(&paths, cx);
                            });
                        }
                    })
                    .child(render_header(muted, border))
                    .child(
                        v_flex()
                            .h(px(BODY_HEIGHT))
                            .w_full()
                            .child(
                                uniform_list("script-search-rows", count, {
                                    let entity = entity.clone();
                                    move |range, _, _cx| {
                                        range
                                            .map(|ui_ix| {
                                                let (path_text, locked) = if ui_ix == 0 {
                                                    (config_dir.clone(), true)
                                                } else {
                                                    (
                                                        extras
                                                            .get(ui_ix - 1)
                                                            .cloned()
                                                            .unwrap_or_default(),
                                                        false,
                                                    )
                                                };
                                                let edit_input =
                                                    editing_extra.and_then(|extra_ix| {
                                                        if extra_ix + 1 == ui_ix {
                                                            editing_input.clone()
                                                        } else {
                                                            None
                                                        }
                                                    });
                                                render_row(
                                                    ui_ix,
                                                    path_text,
                                                    locked,
                                                    selected == Some(ui_ix),
                                                    selected_bg,
                                                    muted,
                                                    hover_bg,
                                                    marker_color,
                                                    drop_slot,
                                                    extras_len,
                                                    edit_input,
                                                    entity.clone(),
                                                )
                                            })
                                            .collect()
                                    }
                                })
                                .h(px((count as f32 * ROW_HEIGHT)
                                    .min(BODY_HEIGHT - END_DROP_MIN)
                                    .max(ROW_HEIGHT)))
                                .w_full(),
                            )
                            .child(render_end_drop_zone(
                                extras_len,
                                drop_slot,
                                marker_color,
                                entity.clone(),
                            )),
                    )
                    .child(render_footer(can_remove, border, entity)),
            )
    }
}

/// Whether the search path differs from the empty default.
pub fn script_search_path_is_dirty(cx: &App) -> bool {
    !settings::store(cx)
        .settings
        .scripting
        .search_path
        .is_empty()
}

/// Clear extra search-path folders.
pub fn script_search_path_reset(_window: &mut Window, cx: &mut App) {
    let _ = settings::update_and_save(cx, |s| {
        s.scripting.search_path.clear();
    });
}
