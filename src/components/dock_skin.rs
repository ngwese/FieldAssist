// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, prelude::FluentBuilder as _, rems, AnyElement, AnyView, App, AppContext as _, Axis, Div,
    Entity, Global, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    dock::{
        BasePanelView, DockArea, DockAreaRenderer, DockContext, DockSkin, NodeId, PanelHandle,
        TabGroupContext, TabGroupRenderer, TilesRenderer,
    },
    h_flex,
    tab::{Tab, TabBar},
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
};

/// Host callback used to close a center tab without dropping the document.
pub struct CenterTabCloseHandler {
    pub close: Rc<dyn Fn(u64, &mut Window, &mut App)>,
}

impl Global for CenterTabCloseHandler {}

/// Dock appearance with small tabs, built on [`DockSkin`].
pub struct CompactDockSkin {
    skin: Rc<DockSkin>,
}

impl CompactDockSkin {
    pub fn dock_area(
        id: impl Into<SharedString>,
        version: Option<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> (Entity<DockArea>, Rc<DockSkin>) {
        let mut skin = None;
        let area = cx.new(|cx| {
            let this = DockSkin::new(cx);
            skin = Some(this.clone());
            DockArea::new(id, version, window, cx)
                .with_renderer(Rc::new(Self { skin: this }) as Rc<dyn DockAreaRenderer>)
        });
        (
            area,
            skin.expect("DockSkin::new ran inside the constructor"),
        )
    }
}

impl DockAreaRenderer for CompactDockSkin {
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        DockAreaRenderer::frame(self.skin.as_ref(), window, cx)
    }

    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        DockAreaRenderer::center_frame(self.skin.as_ref(), window, cx)
    }

    fn split_frame(
        &self,
        node: NodeId,
        axis: Axis,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        DockAreaRenderer::split_frame(self.skin.as_ref(), node, axis, window, cx)
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        DockAreaRenderer::render_dock(self.skin.as_ref(), dock, content, window, cx)
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(CompactTabGroup {
            inner: DockAreaRenderer::tab_group_renderer(self.skin.as_ref()),
        })
    }

    fn tiles_renderer(&self) -> Rc<dyn TilesRenderer> {
        DockAreaRenderer::tiles_renderer(self.skin.as_ref())
    }
}

struct CompactTabGroup {
    inner: Rc<dyn TabGroupRenderer>,
}

impl CompactTabGroup {
    fn is_tool_dock_group(&self, group: &TabGroupContext, cx: &App) -> bool {
        let panels = group.panels();
        if panels.is_empty() {
            return false;
        }
        if panels
            .iter()
            .all(|panel| panel.panel_name(cx) == "EmptyEditorsPanel")
        {
            return false;
        }
        panels.iter().all(|panel| !panel.closable(cx))
    }

    fn panel_title(
        panel: &Arc<dyn BasePanelView>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match PanelHandle::of(panel) {
            Some(handle) => handle
                .tab_name(cx)
                .map(|name| name.into_any_element())
                .unwrap_or_else(|| handle.title(window, cx)),
            None => SharedString::from(panel.panel_name(cx)).into_any_element(),
        }
    }

    fn render_side_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let active_id = group.active_panel().map(|panel| panel.panel_id(cx));
        let muted = cx.theme().muted_foreground;
        let selected_bg = cx.theme().foreground.opacity(0.05);
        let radius = cx.theme().radius;
        let visible: Vec<usize> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .map(|(ix, _)| ix)
            .collect();
        let mut tabs = Vec::with_capacity(visible.len());
        for ix in visible {
            let panel = &group.panels()[ix];
            let title = Self::panel_title(panel, window, cx);
            let selected = !group.is_collapsed() && Some(panel.panel_id(cx)) == active_id;
            let group = group.clone();
            tabs.push(
                div()
                    .id(("side-tab", panel.panel_id(cx).as_u64()))
                    .flex()
                    .flex_none()
                    .items_center()
                    .px_1p5()
                    .py_0p5()
                    .rounded(radius)
                    .cursor_pointer()
                    .text_sm()
                    .text_color(muted)
                    .when(selected, |this| this.bg(selected_bg))
                    .on_click(move |_, window, cx| {
                        group.select_tab(ix, window, cx);
                    })
                    .child(title),
            );
        }
        h_flex()
            .id("side-tab-bar")
            .w_full()
            .flex_none()
            .items_center()
            .gap_1()
            .py_1()
            .children(tabs)
            .into_any_element()
    }
}

impl TabGroupRenderer for CompactTabGroup {
    fn frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        let frame = self.inner.frame(group, window, cx);
        if self.is_tool_dock_group(group, cx) {
            frame.px(rems(0.5))
        } else {
            frame
        }
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.inner.content_frame(group, window, cx)
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if self.is_tool_dock_group(group, cx) {
            return self.render_side_tab_bar(group, window, cx);
        }

        let muted = cx.theme().muted_foreground;
        let active_id = group.active_panel().map(|panel| panel.panel_id(cx));
        let visible: Vec<usize> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .map(|(ix, _)| ix)
            .collect();
        if visible.len() == 1 {
            let panel = &group.panels()[visible[0]];
            if !panel.closable(cx) && panel.panel_name(cx) == "EmptyEditorsPanel" {
                return div().into_any_element();
            }
        }
        let mut tabs = Vec::with_capacity(visible.len());
        for ix in visible {
            let panel = &group.panels()[ix];
            let selected = !group.is_collapsed() && Some(panel.panel_id(cx)) == active_id;
            let closable = panel.closable(cx);
            let panel_id = panel.panel_id(cx).as_u64();
            let group_for_select = group.clone();
            let group_for_close = group.clone();
            let mut tab = Tab::new()
                .small()
                .child(
                    div()
                        .text_color(muted)
                        .child(Self::panel_title(panel, window, cx)),
                )
                .selected(selected)
                .on_click(move |_, window, cx| {
                    group_for_select.select_tab(ix, window, cx);
                });
            if closable {
                tab = tab.suffix(
                    Button::new(("close-tab", panel_id))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tab_stop(false)
                        .on_click(move |_, window, cx| {
                            if let Some(handler) = cx.try_global::<CenterTabCloseHandler>() {
                                let close = handler.close.clone();
                                close(panel_id, window, cx);
                            } else {
                                group_for_close.close(
                                    gpui_component::dock::PanelId::from_u64(panel_id),
                                    window,
                                    cx,
                                );
                            }
                        }),
                );
            }
            tabs.push(tab);
        }
        TabBar::new("tab-bar")
            .small()
            .children(tabs)
            .into_any_element()
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.inner.render_active_panel(panel, group, window, cx)
    }

    fn render_drop_indicator(
        &self,
        indicator: gpui_component::dock::DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_drop_indicator(indicator, window, cx)
    }

    fn render_empty(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_empty(group, window, cx)
    }
}
