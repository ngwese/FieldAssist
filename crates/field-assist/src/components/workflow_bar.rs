// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::{
    div, prelude::FluentBuilder as _, px, rems, AppContext as _, Context, Entity, ExternalPaths,
    Focusable as _, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, Rgba, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    ActiveTheme as _, Icon, IconName, IconNamed, Sizable as _, Size, StyleSized as _,
    StyledExt as _,
};

use crate::app::AppView;
use crate::components::explorer::CompositionDrag;
use crate::script::{PathBrowse, ToolbarAlign, ToolbarItem};

const PATH_FIELD_WIDTH: gpui_kit::Rems = rems(32.);
/// Approx. characters visible in the unfocused path preview at small size.
const PATH_DISPLAY_CHARS: usize = 48;

/// Shorten `text` to at most `max_chars`, keeping the start and end with `…`
/// in the middle. Used for unfocused path previews.
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

struct SquareIcon;

impl IconNamed for SquareIcon {
    fn path(self) -> SharedString {
        "icons/square.svg".into()
    }
}

pub struct WorkflowBar {
    title: SharedString,
    items: Vec<ToolbarItem>,
    inputs: HashMap<String, Entity<InputState>>,
    app: WeakEntity<AppView>,
}

impl WorkflowBar {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            title: SharedString::default(),
            items: Vec::new(),
            inputs: HashMap::new(),
            app,
        }
    }

    pub fn set_snapshot(
        &mut self,
        snapshot: Option<(String, Vec<ToolbarItem>)>,
        cx: &mut Context<Self>,
    ) {
        match snapshot {
            Some((title, items)) => {
                self.title = title.into();
                self.items = items;
            }
            None => {
                self.title = SharedString::default();
                self.items.clear();
            }
        }
        let keep: std::collections::HashSet<String> = self
            .items
            .iter()
            .filter_map(|item| match item {
                ToolbarItem::Path { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        self.inputs.retain(|id, _| keep.contains(id));
        cx.notify();
    }

    fn path_input(
        &mut self,
        id: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(existing) = self.inputs.get(id) {
            let focused = existing.read(cx).focus_handle(cx).is_focused(window);
            if !focused && existing.read(cx).value().as_ref() != value {
                existing.update(cx, |input, cx| {
                    input.set_value(value.to_string(), window, cx);
                });
            }
            return existing.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| {
            state.set_value(value.to_string(), window, cx);
        });
        let item_id = id.to_string();
        let app = self.app.clone();
        cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                // Keep local items in sync so blur does not restore a stale
                // snapshot into the Input.
                for item in &mut this.items {
                    if let ToolbarItem::Path {
                        id,
                        value: stored,
                        ..
                    } = item
                    {
                        if id == &item_id {
                            *stored = value.clone();
                            break;
                        }
                    }
                }
                let item_id = item_id.clone();
                let app = app.clone();
                // Window defer — not defer_in — so we do not re-lease WorkflowBar
                // before AppView updates (refresh would nest and panic).
                window.defer(cx, move |window, cx| {
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |this, cx| {
                            this.set_toolbar_path_value(&item_id, &value, window, cx);
                        });
                    }
                });
            },
        )
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        cx.on_focus(&focus, window, |_, _, cx| cx.notify())
            .detach();
        cx.on_blur(&focus, window, |_, _, cx| cx.notify())
            .detach();
        self.inputs.insert(id.to_string(), input.clone());
        input
    }

    fn prompt_browse(
        &self,
        id: String,
        browse: PathBrowse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (files, directories, prompt) = match browse {
            PathBrowse::File => (true, false, "Choose file"),
            PathBrowse::Directory => (false, true, "Choose folder"),
        };
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        let app = self.app.clone();
        cx.spawn_in(window, async move |_, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let _ = cx.update(|window, cx| {
                if let Some(app) = app.upgrade() {
                    app.update(cx, |this, cx| {
                        this.dispatch_toolbar_path(&id, &paths, window, cx);
                    });
                }
            });
        })
        .detach();
    }
}

impl Render for WorkflowBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let drop_highlight = theme.secondary;
        let items = self.items.clone();
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (ix, item) in items.iter().enumerate() {
            match item.align() {
                ToolbarAlign::Left => left.push((ix, item.clone())),
                ToolbarAlign::Right => right.push((ix, item.clone())),
            }
        }

        h_flex()
            .id("workflow-bar")
            .w_full()
            .flex_none()
            .px_3()
            .py_1()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(theme.foreground)
                    .child(self.title.clone()),
            )
            .child(
                h_flex()
                    .id("workflow-bar-left")
                    .gap_2()
                    .items_center()
                    .children(left.into_iter().map(|(ix, item)| {
                        self.render_item(ix, item, muted, drop_highlight, window, cx)
                    })),
            )
            .child(div().id("workflow-bar-spacer").flex_1().min_w_0())
            .child(
                h_flex()
                    .id("workflow-bar-right")
                    .gap_2()
                    .items_center()
                    .justify_end()
                    .children(right.into_iter().map(|(ix, item)| {
                        self.render_item(ix, item, muted, drop_highlight, window, cx)
                    })),
            )
    }
}

impl WorkflowBar {
    fn render_item(
        &mut self,
        ix: usize,
        item: ToolbarItem,
        muted: Hsla,
        drop_highlight: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match item {
            ToolbarItem::Button { command, label, .. } => {
                let app = self.app.clone();
                Button::new(("workflow-cmd", ix))
                    .ghost()
                    .small()
                    .label(label)
                    .on_click(move |_, window, cx| {
                        if let Some(app) = app.upgrade() {
                            app.update(cx, |this, cx| {
                                this.dispatch_workflow_command(&command, window, cx);
                            });
                        }
                    })
                    .into_any_element()
            }
            ToolbarItem::Path {
                id,
                label,
                value,
                browse,
                ..
            } => {
                let input = self.path_input(&id, &value, window, cx);
                let app = self.app.clone();
                let field_id = id.clone();
                let focused = input.read(cx).focus_handle(cx).is_focused(window);
                let full = input.read(cx).value().to_string();
                let theme = cx.theme().clone();
                let row = h_flex()
                    .id(("workflow-path", ix))
                    .gap_2()
                    .items_center()
                    .w(PATH_FIELD_WIDTH)
                    .min_w_0()
                    .when_some(label, |this, label| {
                        this.child(div().flex_none().text_sm().text_color(muted).child(label))
                    })
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
                            .on_drop({
                                let app = app.clone();
                                let field_id = field_id.clone();
                                move |paths: &ExternalPaths, window, cx| {
                                    let paths: Vec<PathBuf> = paths.paths().to_vec();
                                    let app = app.clone();
                                    let field_id = field_id.clone();
                                    window.defer(cx, move |window, cx| {
                                        if let Some(app) = app.upgrade() {
                                            app.update(cx, |this, cx| {
                                                this.dispatch_toolbar_path(
                                                    &field_id, &paths, window, cx,
                                                );
                                            });
                                        }
                                    });
                                }
                            })
                            .on_drop({
                                let app = app.clone();
                                let field_id = field_id.clone();
                                move |drag: &CompositionDrag, window, cx| {
                                    let Some(path) = drag.path.clone() else {
                                        return;
                                    };
                                    let app = app.clone();
                                    let field_id = field_id.clone();
                                    window.defer(cx, move |window, cx| {
                                        if let Some(app) = app.upgrade() {
                                            app.update(cx, |this, cx| {
                                                this.dispatch_toolbar_path(
                                                    &field_id,
                                                    &[path],
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    });
                                }
                            })
                            .map(|this| {
                                // Keep a fixed small-input height so focus/blur
                                // does not resize the toolbar.
                                this.h_6().map(|this| {
                                    if focused {
                                        this.child(Input::new(&input).small().w_full().h_full())
                                    } else {
                                        let focus = input.read(cx).focus_handle(cx);
                                        let preview =
                                            middle_ellipsis(&full, PATH_DISPLAY_CHARS);
                                        this.child(
                                            div()
                                                .id(("workflow-path-preview", ix))
                                                .w_full()
                                                .h_full()
                                                .flex()
                                                .items_center()
                                                .input_px(Size::Small)
                                                .rounded(cx.theme().radius)
                                                .border_1()
                                                .border_color(theme.input)
                                                .bg(theme.background)
                                                .input_text_size(Size::Small)
                                                .text_color(theme.foreground)
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .cursor_text()
                                                .tooltip({
                                                    let full = full.clone();
                                                    move |window, cx| {
                                                        Tooltip::new(full.clone())
                                                            .build(window, cx)
                                                    }
                                                })
                                                .child(preview)
                                                .on_click(move |_, window, cx| {
                                                    focus.focus(window, cx);
                                                }),
                                        )
                                    }
                                })
                            }),
                    )
                    .when_some(browse, |this, browse| {
                        let app_id = id.clone();
                        this.child(
                            Button::new(("workflow-browse", ix))
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .tooltip("Browse")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.prompt_browse(app_id.clone(), browse, window, cx);
                                })),
                        )
                    });
                row.into_any_element()
            }
            ToolbarItem::Toggle {
                id,
                label,
                value,
                on_color,
                off_color,
                ..
            } => {
                let color = rgba(if value { on_color } else { off_color });
                let icon = if value {
                    Icon::new(IconName::Check).text_color(color)
                } else {
                    Icon::new(SquareIcon).text_color(color)
                };
                let app = self.app.clone();
                Button::new(("workflow-toggle", ix))
                    .ghost()
                    .small()
                    .icon(icon)
                    .label(label)
                    .text_color(color)
                    .toggled(value)
                    .on_click(move |_, window, cx| {
                        if let Some(app) = app.upgrade() {
                            app.update(cx, |this, cx| {
                                this.dispatch_toolbar_toggle(&id, window, cx);
                            });
                        }
                    })
                    .into_any_element()
            }
            ToolbarItem::Message { text, color, .. } => div()
                .id(("workflow-msg", ix))
                .text_sm()
                .text_color(color.map(rgba).unwrap_or(muted))
                .child(text)
                .into_any_element(),
            ToolbarItem::Divider { .. } => div()
                .id(("workflow-div", ix))
                .flex_none()
                .w(px(1.))
                .self_stretch()
                .my_1()
                .bg(cx.theme().border)
                .into_any_element(),
        }
    }
}

fn rgba(color: [f32; 4]) -> Hsla {
    Rgba {
        r: color[0],
        g: color[1],
        b: color[2],
        a: color[3],
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::middle_ellipsis;

    #[test]
    fn middle_ellipsis_keeps_short_strings() {
        assert_eq!(middle_ellipsis("short", 10), "short");
    }

    #[test]
    fn middle_ellipsis_shows_head_and_tail() {
        let path = r"C:\Users\g1988\proj\takes\MixPre-003_Ambix.WAV";
        let shown = middle_ellipsis(path, 24);
        assert!(shown.contains('…'), "{shown}");
        assert!(shown.starts_with(r"C:\Users"), "{shown}");
        assert!(shown.ends_with("Ambix.WAV"), "{shown}");
        assert!(shown.chars().count() <= 24, "{shown}");
    }
}
