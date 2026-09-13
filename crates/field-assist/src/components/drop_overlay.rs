// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{h_flex, v_flex, StyledExt as _};
use gpui_kit::{
    div, hsla, px, ExternalPaths, InteractiveElement as _, IntoElement, ParentElement as _, Rgba,
    Styled as _, WeakEntity,
};

use crate::app::AppView;
use crate::script::DropLayout;

const DROP_INSET: f32 = 8.;

pub fn file_drop_overlay(layout: Arc<DropLayout>, app: WeakEntity<AppView>) -> impl IntoElement {
    v_flex()
        .id("file-drop-overlay")
        .absolute()
        .inset(px(DROP_INSET))
        .occlude()
        .children(layout.rows.iter().enumerate().map(|(row_ix, row)| {
            let app = app.clone();
            h_flex()
                .id(("drop-row", row_ix))
                .flex_1()
                .w_full()
                .min_h_0()
                .children(row.cells.iter().enumerate().map(move |(cell_ix, cell)| {
                    let app = app.clone();
                    let name = cell.name.clone();
                    let display = cell.display_name.clone();
                    let color = cell.color;
                    let weight = cell.weight as f32;
                    let fill = workflow_fill(color, 0.80);
                    let hover = workflow_fill(color, 0.85);
                    let border = workflow_fill(color, color[3]);
                    div()
                        .id(("drop-cell", row_ix * 100 + cell_ix))
                        .flex_grow(weight.max(0.001))
                        .h_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .overflow_hidden()
                        .border_1()
                        .border_color(border)
                        .bg(fill)
                        .text_color(hsla(0., 0., 1., 1.))
                        .text_3xl()
                        .font_bold()
                        .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(hover))
                        .on_drop({
                            let app = app.clone();
                            let name = name.clone();
                            move |paths: &ExternalPaths, window, cx| {
                                let paths: Vec<PathBuf> = paths.paths().to_vec();
                                if let Some(app) = app.upgrade() {
                                    app.update(cx, |this, cx| {
                                        this.invoke_drop_workflow(&name, &paths, window, cx);
                                    });
                                }
                            }
                        })
                        .child(display)
                }))
        }))
}

fn workflow_fill(color: [f32; 4], alpha: f32) -> gpui_kit::Hsla {
    Rgba {
        r: color[0],
        g: color[1],
        b: color[2],
        a: alpha,
    }
    .into()
}
