// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Modal workflow sheet: Questionnaire shell, Stepper trail, Form body.
//!
//! Rendered as an app-owned overlay below the window title bar (same pattern as
//! ExportSheet) so the root window stays movable while the sheet is open.

use std::collections::HashMap;
use std::path::PathBuf;

use field_scripting::{
    PathBrowse, ProgressVariant, SheetPane, SheetSnapshot, ToolbarAlign, ToolbarItem,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    form::{field, Form},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    progress::{Progress, ProgressCircle},
    questionnaire::{
        Questionnaire, QuestionnaireActions, QuestionnaireDescription, QuestionnaireItem,
        QuestionnaireItemDefinition, QuestionnaireState,
    },
    separator::Separator,
    stepper::{Stepper, StepperItem},
    switch::Switch,
    ActiveTheme as _, Disableable as _, IconName,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, App, AppContext as _, Context, Entity, ExternalPaths, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, Styled as _, WeakEntity, Window,
};

use crate::app::AppView;
use crate::components::explorer::CompositionDrag;

/// Live GPUI state for an open workflow sheet.
pub struct WorkflowSheetView {
    app: WeakEntity<AppView>,
    snapshot: SheetSnapshot,
    questionnaire: Entity<QuestionnaireState>,
    inputs: HashMap<String, Entity<InputState>>,
    logs: HashMap<String, Entity<TextareaState>>,
}

impl WorkflowSheetView {
    pub fn new(
        snapshot: SheetSnapshot,
        app: WeakEntity<AppView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let questionnaire = build_questionnaire(&snapshot, cx);
        let mut this = Self {
            app,
            snapshot: SheetSnapshot {
                title: String::new(),
                panes: Vec::new(),
                current: String::new(),
            },
            questionnaire,
            inputs: HashMap::new(),
            logs: HashMap::new(),
        };
        this.apply_snapshot(snapshot, window, cx);
        this
    }

    pub fn set_snapshot(
        &mut self,
        snapshot: SheetSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused_input = self.focused_input_id(window, cx);
        self.apply_snapshot(snapshot, window, cx);
        if let Some(id) = focused_input {
            if let Some(input) = self.inputs.get(&id) {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
        cx.notify();
    }

    pub fn title(&self) -> &str {
        &self.snapshot.title
    }

    /// Id of a path/text Input that currently holds keyboard focus, if any.
    pub fn focused_input_id(&self, window: &Window, cx: &App) -> Option<String> {
        self.inputs.iter().find_map(|(id, input)| {
            input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
                .then(|| id.clone())
        })
    }

    fn apply_snapshot(
        &mut self,
        snapshot: SheetSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rebuild = self.snapshot.panes.len() != snapshot.panes.len()
            || self
                .snapshot
                .panes
                .iter()
                .zip(snapshot.panes.iter())
                .any(|(a, b)| a.id != b.id);
        if rebuild {
            self.questionnaire = build_questionnaire(&snapshot, cx);
            self.inputs.clear();
            self.logs.clear();
        }
        let current = snapshot.current.clone();
        // Re-selecting the current pane on every keystroke refreshes the
        // Questionnaire and steals focus from path Inputs into sibling rows.
        if rebuild || self.snapshot.current != current {
            let _ = self.questionnaire.update(cx, |state, cx| {
                let _ = state.set_current_item(current.as_str(), window, cx);
            });
        }
        for pane in &snapshot.panes {
            self.sync_pane_controls(pane, window, cx);
        }
        self.snapshot = snapshot;
    }

    fn sync_pane_controls(
        &mut self,
        pane: &SheetPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for item in pane.controls.iter().chain(pane.buttons.iter()) {
            match item {
                ToolbarItem::PathEntry { id, value, .. } | ToolbarItem::Text { id, value, .. } => {
                    if !self.inputs.contains_key(id) {
                        let control_id = id.clone();
                        let entity = cx.new(|cx| InputState::new(window, cx));
                        cx.subscribe_in(
                            &entity,
                            window,
                            move |this, state, event: &InputEvent, window, cx| {
                                if !matches!(event, InputEvent::Change) {
                                    return;
                                }
                                let value = state.read(cx).value().to_string();
                                this.dispatch_entry(&control_id, &value, window, cx);
                            },
                        )
                        .detach();
                        self.inputs.insert(id.clone(), entity);
                    }
                    let input = self.inputs.get(id).expect("just inserted");
                    // Match WorkflowBar: never clobber a focused field mid-edit.
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    let current = input.read(cx).value().to_string();
                    if !focused && current != *value {
                        input.update(cx, |state, cx| {
                            state.set_value(value.clone(), window, cx);
                        });
                    }
                }
                ToolbarItem::Log { id, text, .. } => {
                    if !self.logs.contains_key(id) {
                        self.logs.insert(
                            id.clone(),
                            cx.new(|cx| TextareaState::new(window, cx).rows(8)),
                        );
                    }
                    let log = self.logs.get(id).expect("just inserted");
                    let current = log.read(cx).value().to_string();
                    if current != *text {
                        log.update(cx, |state, cx| {
                            state.set_value(text.clone(), window, cx);
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn dispatch_entry(&self, id: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let id = id.to_owned();
        let value = value.to_owned();
        // Input subscribe runs inside WorkflowSheetView::update; defer so a
        // Lua action that refreshes the sheet cannot re-enter this entity.
        window.defer(cx, move |window, cx| {
            app.update(cx, |this, cx| {
                this.set_toolbar_entry_value(&id, &value, window, cx);
            });
        });
    }

    fn dispatch_button(&self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let id = id.to_owned();
        window.defer(cx, move |window, cx| {
            app.update(cx, |this, cx| {
                this.dispatch_toolbar_button(&id, window, cx);
            });
        });
    }

    fn dispatch_toggle(&self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let id = id.to_owned();
        window.defer(cx, move |window, cx| {
            app.update(cx, |this, cx| {
                this.dispatch_toolbar_toggle(&id, window, cx);
            });
        });
    }

    fn dispatch_path(
        &self,
        id: &str,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let id = id.to_owned();
        let paths = paths.to_vec();
        window.defer(cx, move |window, cx| {
            app.update(cx, |this, cx| {
                this.dispatch_toolbar_path(&id, &paths, window, cx);
            });
        });
    }

    fn prompt_browse(
        &self,
        id: String,
        browse: PathBrowse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (files, directories) = match browse {
            PathBrowse::File => (true, false),
            PathBrowse::Directory => (false, true),
        };
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories,
            multiple: false,
            prompt: Some("Select".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.dispatch_path(&id, &paths, window, cx);
                });
            });
        })
        .detach();
    }

    fn current_pane(&self) -> Option<&SheetPane> {
        self.snapshot
            .panes
            .iter()
            .find(|pane| pane.id == self.snapshot.current)
    }

    fn render_control(
        &self,
        item: &ToolbarItem,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match item {
            ToolbarItem::Divider { .. } => Separator::horizontal().into_any_element(),
            ToolbarItem::Message { text, .. } => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text.clone())
                .into_any_element(),
            ToolbarItem::Button {
                id, label, enabled, ..
            } => {
                let id = id.clone();
                Button::new(("sheet-btn", ix))
                    .outline()
                    .label(label.clone())
                    .disabled(!*enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch_button(&id, window, cx);
                    }))
                    .into_any_element()
            }
            ToolbarItem::Toggle {
                id, value, enabled, ..
            } => {
                let id = id.clone();
                Switch::new(("sheet-toggle", ix))
                    .checked(*value)
                    .disabled(!*enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch_toggle(&id, window, cx);
                    }))
                    .into_any_element()
            }
            ToolbarItem::PathEntry {
                id,
                browse,
                enabled,
                ..
            } => {
                let Some(input) = self.inputs.get(id) else {
                    return div().into_any_element();
                };
                let drop_highlight = cx.theme().secondary;
                let mut field_el = Input::new(input).disabled(!*enabled);
                if let Some(browse) = *browse {
                    let id = id.clone();
                    field_el = field_el.suffix(
                        Button::new(("sheet-browse", ix))
                            .ghost()
                            .icon(IconName::FolderOpen)
                            .disabled(!*enabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.prompt_browse(id.clone(), browse, window, cx);
                            })),
                    );
                }
                let id = id.clone();
                div()
                    .w_full()
                    .can_drop(|data, _, _| {
                        data.downcast_ref::<ExternalPaths>().is_some()
                            || data
                                .downcast_ref::<CompositionDrag>()
                                .is_some_and(|drag| drag.path.is_some())
                    })
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_highlight))
                    .drag_over::<CompositionDrag>(move |style, _, _, _| style.bg(drop_highlight))
                    .on_drop({
                        let id = id.clone();
                        cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.dispatch_path(&id, paths.paths(), window, cx);
                        })
                    })
                    .on_drop(
                        cx.listener(move |this, drag: &CompositionDrag, window, cx| {
                            let Some(path) = drag.path.clone() else {
                                return;
                            };
                            this.dispatch_path(&id, &[path], window, cx);
                        }),
                    )
                    .child(field_el)
                    .into_any_element()
            }
            ToolbarItem::Text { id, enabled, .. } => {
                let Some(input) = self.inputs.get(id) else {
                    return div().into_any_element();
                };
                Input::new(input).disabled(!*enabled).into_any_element()
            }
            ToolbarItem::Select {
                id,
                value,
                choices,
                enabled,
                ..
            } => {
                let id = id.clone();
                let app = self.app.clone();
                let selected = choices
                    .iter()
                    .find(|(v, _)| v == value)
                    .map(|(_, label)| label.clone())
                    .unwrap_or_else(|| value.clone());
                let choices = choices.clone();
                let current = value.clone();
                Button::new(("sheet-select", ix))
                    .outline()
                    .label(if selected.is_empty() {
                        "Select…".into()
                    } else {
                        selected
                    })
                    .disabled(!*enabled)
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu;
                        for (choice_value, choice_label) in &choices {
                            let id = id.clone();
                            let app = app.clone();
                            let choice_value = choice_value.clone();
                            let checked = choice_value == current;
                            menu = menu.item(
                                PopupMenuItem::new(choice_label.clone())
                                    .checked(checked)
                                    .on_click(move |_, window, cx| {
                                        let id = id.clone();
                                        let choice_value = choice_value.clone();
                                        let app = app.clone();
                                        window.defer(cx, move |window, cx| {
                                            if let Some(app) = app.upgrade() {
                                                app.update(cx, |this, cx| {
                                                    this.set_toolbar_entry_value(
                                                        &id,
                                                        &choice_value,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }
                                        });
                                    }),
                            );
                        }
                        menu
                    })
                    .into_any_element()
            }
            ToolbarItem::Progress {
                id,
                variant,
                value,
                loading,
                text,
                ..
            } => {
                let indicator = match variant {
                    ProgressVariant::Bar => Progress::new(format!("sheet-progress-{id}"))
                        .loading(*loading)
                        .when(!*loading, |p| p.value(value.unwrap_or(0.0)))
                        .into_any_element(),
                    ProgressVariant::Circle => {
                        ProgressCircle::new(format!("sheet-progress-c-{id}"))
                            .loading(*loading)
                            .when(!*loading, |p| p.value(value.unwrap_or(0.0)))
                            .into_any_element()
                    }
                };
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(indicator))
                    .when_some(text.clone(), |this, text| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(text),
                        )
                    })
                    .into_any_element()
            }
            ToolbarItem::Log { id, .. } => {
                let Some(log) = self.logs.get(id) else {
                    return div().into_any_element();
                };
                Textarea::new(log)
                    .readonly(true)
                    .h(px(160.))
                    .into_any_element()
            }
        }
    }

    fn render_pane_button(
        &self,
        item: &ToolbarItem,
        ix: usize,
        primary: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let ToolbarItem::Button {
            id, label, enabled, ..
        } = item
        else {
            return div().into_any_element();
        };
        let id = id.clone();
        let mut btn = Button::new(("sheet-action", ix))
            .label(label.clone())
            .disabled(!*enabled);
        btn = if primary {
            btn.primary()
        } else {
            btn.outline()
        };
        btn.on_click(cx.listener(move |this, _, window, cx| {
            this.dispatch_button(&id, window, cx);
        }))
        .into_any_element()
    }

    fn build_form(&self, pane: &SheetPane, cx: &mut Context<Self>) -> Form {
        let mut form = Form::horizontal().label_width(px(120.));
        for (ix, item) in pane.controls.iter().enumerate() {
            let label = item.label().unwrap_or("").to_owned();
            let control = self.render_control(item, ix, cx);
            let mut f = field().child(control);
            if label.is_empty() {
                f = f.label_indent(true);
            } else {
                f = f.label(label);
            }
            form = form.child(f);
        }
        form
    }
}

fn build_questionnaire(snapshot: &SheetSnapshot, cx: &mut App) -> Entity<QuestionnaireState> {
    let items: Vec<_> = snapshot
        .panes
        .iter()
        .map(|pane| {
            QuestionnaireItemDefinition::new(pane.id.clone(), pane.name.clone())
                .with_required(false)
                .with_description(pane.text.clone())
        })
        .collect();
    cx.new(|cx| QuestionnaireState::new(items, cx).expect("valid sheet panes"))
}

/// Open or refresh the workflow sheet overlay from AppView.
///
/// `snapshot` must come from the caller's `&mut AppView` — do not
/// `Entity::read` AppView here; callers are already inside `update`.
///
/// The sheet is drawn by [`AppView`] below the title bar so window drag
/// keeps working (Root `open_dialog` overlays cover the title bar and block
/// macOS `start_window_move`).
pub fn refresh_workflow_sheet(
    app: Entity<AppView>,
    snapshot: Option<SheetSnapshot>,
    sheet: &mut Option<Entity<WorkflowSheetView>>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    match snapshot {
        Some(snap) => {
            if let Some(view) = sheet {
                view.update(cx, |view, cx| {
                    view.set_snapshot(snap, window, cx);
                });
            } else {
                let weak = app.downgrade();
                *sheet = Some(cx.new(|cx| WorkflowSheetView::new(snap, weak, window, cx)));
            }
            cx.notify();
        }
        None => {
            if sheet.take().is_some() {
                cx.notify();
            }
        }
    }
}

impl Render for WorkflowSheetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.questionnaire.clone();
        let current_ix = self
            .snapshot
            .panes
            .iter()
            .position(|pane| pane.id == self.snapshot.current)
            .unwrap_or(0);
        let current_id = self.snapshot.current.clone();
        let guide = self
            .current_pane()
            .map(|pane| pane.text.clone())
            .unwrap_or_default();

        let stepper =
            (self.snapshot.panes.len() > 1).then(|| {
                Stepper::new("workflow-sheet-trail")
                    .selected_index(current_ix)
                    .disabled(true)
                    .items(self.snapshot.panes.iter().map(|pane| {
                        StepperItem::new().child(SharedString::from(pane.name.clone()))
                    }))
            });

        let actions = self.current_pane().and_then(|pane| {
            if pane.buttons.is_empty() {
                return None;
            }
            let left: Vec<_> = pane
                .buttons
                .iter()
                .enumerate()
                .filter(|(_, b)| b.align() == ToolbarAlign::Left)
                .collect();
            let right: Vec<_> = pane
                .buttons
                .iter()
                .enumerate()
                .filter(|(_, b)| b.align() != ToolbarAlign::Left)
                .collect();
            let mut row = h_flex().w_full().gap_2().items_center();
            let has_left = !left.is_empty();
            if !has_left {
                row = row.justify_end();
            }
            for (ix, item) in left {
                row = row.child(self.render_pane_button(item, ix, false, cx));
            }
            for (i, (ix, item)) in right.iter().enumerate() {
                let primary = i + 1 == right.len();
                let btn = self.render_pane_button(item, *ix + 100, primary, cx);
                row = if i == 0 && has_left {
                    row.child(div().flex_1()).child(btn)
                } else {
                    row.child(btn)
                };
            }
            Some(row)
        });

        let mut root = Questionnaire::new(&q);
        if let Some(stepper) = stepper {
            root = root.child(stepper);
        }
        for pane in &self.snapshot.panes {
            let id = pane.id.clone();
            let is_current = id == current_id;
            let mut item = QuestionnaireItem::new(&q, id.clone()).child(
                QuestionnaireDescription::new(&q, id).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if is_current {
                            guide.clone()
                        } else {
                            pane.text.clone()
                        }),
                ),
            );
            if is_current {
                item = item.child(self.build_form(pane, cx));
            }
            root = root.child(item);
        }
        root.child(
            QuestionnaireActions::new(&q).when_some(actions, |this, actions| this.child(actions)),
        )
    }
}
