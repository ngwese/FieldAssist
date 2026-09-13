// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex, v_flex, ActiveTheme as _, Disableable as _,
};
use gpui_kit::{
    div, prelude::FluentBuilder as _, px, App, ClickEvent, Context, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::model::DocumentId;

#[derive(Clone, Copy, Debug)]
pub enum QuitUnsavedAction {
    SaveAll,
    SaveSelected,
    Discard,
}

type ActionHandler = Rc<dyn Fn(QuitUnsavedAction, Vec<DocumentId>, &mut Window, &mut App)>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct MultiSelect {
    selected: BTreeSet<usize>,
    anchor: usize,
}

impl MultiSelect {
    fn all(count: usize) -> Self {
        Self {
            selected: (0..count).collect(),
            anchor: 0,
        }
    }

    fn click(&mut self, index: usize, shift: bool, disjoint: bool, count: usize) {
        if index >= count {
            return;
        }
        if shift {
            let start = self.anchor.min(index);
            let end = self.anchor.max(index);
            let range = start..=end;
            if disjoint {
                self.selected.extend(range);
            } else {
                self.selected = range.collect();
            }
        } else if disjoint {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = index;
        } else if self.selected.len() == 1 && self.selected.contains(&index) {
            self.selected.clear();
            self.anchor = index;
        } else {
            self.selected.clear();
            self.selected.insert(index);
            self.anchor = index;
        }
    }
}

pub struct QuitUnsavedList {
    items: Vec<(DocumentId, SharedString)>,
    selection: MultiSelect,
    on_action: Option<ActionHandler>,
    focus_handle: FocusHandle,
}

impl QuitUnsavedList {
    pub fn new(items: Vec<(DocumentId, SharedString)>, cx: &mut Context<Self>) -> Self {
        let count = items.len();
        Self {
            items,
            selection: MultiSelect::all(count),
            on_action: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_handler(&mut self, handler: ActionHandler) {
        self.on_action = Some(handler);
    }

    fn selected_ids(&self) -> Vec<DocumentId> {
        self.selection
            .selected
            .iter()
            .filter_map(|ix| self.items.get(*ix).map(|(id, _)| *id))
            .collect()
    }

    fn emit(&self, action: QuitUnsavedAction, window: &mut Window, cx: &mut App) {
        let ids = match action {
            QuitUnsavedAction::SaveAll => self.items.iter().map(|(id, _)| *id).collect(),
            QuitUnsavedAction::SaveSelected => self.selected_ids(),
            QuitUnsavedAction::Discard => Vec::new(),
        };
        if let Some(handler) = self.on_action.clone() {
            handler(action, ids, window, cx);
        }
    }
}

impl Focusable for QuitUnsavedList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QuitUnsavedList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_bg = theme.list_active;
        let hover_bg = theme.list_hover;
        let radius = theme.radius;
        let list = cx.entity();
        let has_selection = !self.selection.selected.is_empty();

        v_flex()
            .w_full()
            .gap_8()
            .child(
                div()
                    .id("quit-unsaved-list")
                    .w_full()
                    .max_h(px(440.))
                    .min_h(px(144.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(radius)
                    .p_1()
                    .text_xs()
                    .children(self.items.iter().enumerate().map(|(ix, (id, name))| {
                        let selected = self.selection.selected.contains(&ix);
                        let name = name.clone();
                        let list = list.clone();
                        let row_id = SharedString::from(format!("quit-unsaved-{id}"));
                        h_flex()
                            .id(row_id)
                            .w_full()
                            .flex_none()
                            .items_center()
                            .px_1p5()
                            .py_0p5()
                            .rounded(radius)
                            .cursor_pointer()
                            .when(selected, |this| this.bg(selected_bg))
                            .when(!selected, |this| this.hover(|this| this.bg(hover_bg)))
                            .on_click(move |event: &ClickEvent, _, cx| {
                                let modifiers = event.modifiers();
                                list.update(cx, |this, cx| {
                                    this.selection.click(
                                        ix,
                                        modifiers.shift,
                                        modifiers.secondary(),
                                        this.items.len(),
                                    );
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_xs()
                                    .child(name),
                            )
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("quit-without-saving")
                            .outline()
                            .label("Quit Without Saving")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.emit(QuitUnsavedAction::Discard, window, cx);
                            })),
                    )
                    .child(
                        Button::new("save-selected")
                            .outline()
                            .label("Save Selected")
                            .disabled(!has_selection)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.emit(QuitUnsavedAction::SaveSelected, window, cx);
                            })),
                    )
                    .child(
                        Button::new("save-all")
                            .primary()
                            .label("Save All")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.emit(QuitUnsavedAction::SaveAll, window, cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_selects_exclusively_and_toggles_off() {
        let mut sel = MultiSelect::all(3);
        sel.click(1, false, false, 3);
        assert_eq!(sel.selected.iter().copied().collect::<Vec<_>>(), vec![1]);
        sel.click(1, false, false, 3);
        assert!(sel.selected.is_empty());
    }

    #[test]
    fn shift_click_extends_range_from_anchor() {
        let mut sel = MultiSelect {
            selected: BTreeSet::from([0]),
            anchor: 0,
        };
        sel.click(2, true, false, 3);
        assert_eq!(
            sel.selected.iter().copied().collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn secondary_click_toggles_disjoint_items() {
        let mut sel = MultiSelect {
            selected: BTreeSet::from([0]),
            anchor: 0,
        };
        sel.click(2, false, true, 3);
        assert_eq!(sel.selected.iter().copied().collect::<Vec<_>>(), vec![0, 2]);
        sel.click(0, false, true, 3);
        assert_eq!(sel.selected.iter().copied().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn shift_secondary_adds_range_to_selection() {
        let mut sel = MultiSelect {
            selected: BTreeSet::from([0]),
            anchor: 0,
        };
        sel.click(2, false, true, 3);
        sel.click(1, true, true, 3);
        assert_eq!(
            sel.selected.iter().copied().collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
}
