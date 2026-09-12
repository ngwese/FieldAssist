// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Multi-lane waveform view with selection, markers, and edit overlays.
//!
//! Hosts supply sample data via [`WaveformDataProvider`] and overlays /
//! mutations via [`WaveformEditor`].

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    canvas, div, fill, hsla, point, px, relative, rems, size, App, Bounds, Context, DispatchPhase,
    Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, PathBuilder, Pixels, Render,
    Rgba, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit::component::{
    h_flex,
    menu::ContextMenuExt,
    plot::scale::{Scale as _, ScaleLinear},
    v_flex, ActiveTheme as _, StyledExt as _,
};

use crate::waveform_data::WaveformDataProvider;
use crate::waveform_editor::{LaneScope, PaintRegion, WaveformEditor};

#[allow(missing_docs)]
mod waveform_actions {
    use gpui_kit::actions;
    actions!(waveform, [ToggleZeroCrossing]);
}
/// Toggle zero-crossing snap for region / caret placement.
pub use waveform_actions::ToggleZeroCrossing;

const ZOOM_FACTOR: f64 = 1.25;
const MIN_SAMPLES_PER_PIXEL: f64 = 1.0 / 50.0;
const FRAME_PADDING: f64 = 0.1;
const MIN_LANE_HEIGHT: f32 = 96.0;
const MIN_THUMB: f32 = 24.0;
const SCROLLBAR_HEIGHT: f32 = 14.0;
const DRAG_MOVE_THRESHOLD_PX: f32 = 3.0;
const POSITION_BAR_COLOR: gpui_kit::Hsla = hsla(0.0, 0.72, 0.55, 1.0);
const GHOST_BAR_COLOR: gpui_kit::Hsla = hsla(0.0, 0.72, 0.55, 0.35);
const MODIFIED_BAR_HEIGHT: f32 = 3.0;
const MODIFIED_BAR_GAP: f32 = 1.0;
const MODIFIED_BAR_COLOR: gpui_kit::Hsla = hsla(0.08, 0.90, 0.55, 1.0);
const MODIFIED_HOVER_FILL: gpui_kit::Hsla = hsla(0.08, 0.90, 0.55, 0.18);
const MARKER_BAR_OPACITY: f32 = 0.35;
const MARKER_TRIANGLE_BASE: f32 = 5.0;
const MARKER_TRIANGLE_HEIGHT: f32 = 5.0;
/// Fraction of the visible timeline used as the marker snap latch and
/// release radius.
const MARKER_SNAP_VIEWPORT_FRACTION: f64 = 0.01;

enum Drag {
    SelectRegion {
        lane: usize,
        alt: bool,
        ctrl: bool,
        shift: bool,
        anchor_sample: usize,
        origin_x: f32,
        dragging: bool,
    },
    Scrollbar {
        grab_offset: f32,
    },
}

/// Multi-lane waveform display driven by host document traits.
pub struct WaveformDisplay<D>
where
    D: WaveformDataProvider + WaveformEditor + 'static,
{
    document: Entity<D>,
    start_sample: f64,
    samples_per_pixel: f64,
    viewport_width: f32,
    content_origin_x: f32,
    content_origin_y: f32,
    content_height: f32,
    scrollbar_origin_x: f32,
    scrollbar_width: f32,
    drag: Option<Drag>,
    hover_sample: Option<usize>,
    hovered_edit: Option<u64>,
    pointer_over: bool,
    focus_handle: FocusHandle,
}

impl<D> WaveformDisplay<D>
where
    D: WaveformDataProvider + WaveformEditor + 'static,
{
    /// Observe `document` and fit an initial overview zoom.
    pub fn new(document: Entity<D>, cx: &mut Context<Self>) -> Self {
        cx.observe(&document, |_, _, cx| cx.notify()).detach();
        let frames = WaveformDataProvider::frames(document.read(cx));
        let samples_per_pixel = if frames == 0 {
            1.0
        } else {
            frames as f64 / 1000.0
        };
        Self {
            document,
            start_sample: 0.0,
            samples_per_pixel: samples_per_pixel.max(MIN_SAMPLES_PER_PIXEL),
            viewport_width: 0.0,
            content_origin_x: 0.0,
            content_origin_y: 0.0,
            content_height: 0.0,
            scrollbar_origin_x: 0.0,
            scrollbar_width: 0.0,
            drag: None,
            hover_sample: None,
            hovered_edit: None,
            pointer_over: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Sample under the pointer, if any.
    pub fn hover_sample(&self) -> Option<usize> {
        self.hover_sample
    }

    /// Whether the pointer is over this view (for keymap scoping).
    pub fn pointer_over(&self) -> bool {
        self.pointer_over
    }

    /// Highlight ranges for the hovered edit history card (`None` clears).
    pub fn set_hovered_edit(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        if self.hovered_edit == id {
            return;
        }
        self.hovered_edit = id;
        cx.notify();
    }

    /// Pan so edit `id` ranges are visible (id `0` with no ranges → frame 0).
    pub fn scroll_edit_into_view(&mut self, id: u64, cx: &mut Context<Self>) {
        let (ranges, frames) = {
            let doc = self.document.read(cx);
            (
                WaveformEditor::ranges_for_edit(doc, id),
                WaveformDataProvider::frames(doc) as f64,
            )
        };
        let before = self.start_sample;
        if ranges.is_empty() {
            if id == 0 {
                apply_scroll_to_frame(
                    &mut self.start_sample,
                    self.samples_per_pixel,
                    self.viewport_width,
                    frames,
                    0.0,
                );
            }
        } else {
            apply_scroll_ranges_into_view(
                &mut self.start_sample,
                self.samples_per_pixel,
                self.viewport_width,
                frames,
                &ranges,
            );
        }
        if (self.start_sample - before).abs() > f64::EPSILON {
            cx.notify();
        }
    }

    /// Pan so `sample` stays inside the padded viewport.
    pub fn scroll_sample_into_view(&mut self, sample: f64, cx: &mut Context<Self>) {
        let frames = self.frames(cx) as f64;
        let before = self.start_sample;
        apply_scroll_to_frame(
            &mut self.start_sample,
            self.samples_per_pixel,
            self.viewport_width,
            frames,
            sample,
        );
        if (self.start_sample - before).abs() > f64::EPSILON {
            cx.notify();
        }
    }

    /// Zoom in around the playhead (or viewport center).
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        let anchor = self.anchor_sample(cx);
        self.zoom_at(1.0 / ZOOM_FACTOR, anchor, cx);
        cx.notify();
    }

    /// Zoom out around the playhead (or viewport center).
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let anchor = self.anchor_sample(cx);
        self.zoom_at(ZOOM_FACTOR, anchor, cx);
        cx.notify();
    }

    /// Fit the full buffer into the viewport.
    pub fn fit(&mut self, cx: &mut Context<Self>) {
        self.start_sample = 0.0;
        self.samples_per_pixel = self.max_samples_per_pixel(cx);
        cx.notify();
    }

    /// Frame the selection, or scroll to the playhead when none is set.
    pub fn frame(&mut self, cx: &mut Context<Self>) {
        let frames = self.frames(cx) as f64;
        let (region, position) = {
            let doc = self.document.read(cx);
            let region = WaveformEditor::selection_span(doc)
                .map(|(start, end)| (start as f64, end as f64 + 1.0));
            let position = WaveformEditor::playhead(doc).map(|(sample, _)| sample as f64);
            (region, position)
        };
        if let Some((start, end)) = region {
            apply_fit_range(
                &mut self.start_sample,
                &mut self.samples_per_pixel,
                self.viewport_width,
                frames,
                start,
                end,
            );
        } else if let Some(sample) = position {
            apply_scroll_to_frame(
                &mut self.start_sample,
                self.samples_per_pixel,
                self.viewport_width,
                frames,
                sample,
            );
        }
        cx.notify();
    }

    /// Clear interaction state and reset overview zoom.
    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        self.hover_sample = None;
        self.hovered_edit = None;
        self.start_sample = 0.0;
        let frames = self.frames(cx);
        self.samples_per_pixel = if frames == 0 {
            1.0
        } else {
            (frames as f64 / 1000.0).max(MIN_SAMPLES_PER_PIXEL)
        };
        if frames > 0 {
            self.fit(cx);
        } else {
            cx.notify();
        }
    }

    fn frames(&self, cx: &App) -> usize {
        WaveformDataProvider::frames(self.document.read(cx))
    }

    fn anchor_sample(&self, cx: &App) -> f64 {
        WaveformEditor::selection_position_sample(self.document.read(cx))
            .map(|s| s as f64)
            .unwrap_or_else(|| self.start_sample + self.visible_samples() * 0.5)
    }

    fn max_samples_per_pixel(&self, cx: &App) -> f64 {
        let width = self.viewport_width.max(1.0) as f64;
        let frames = WaveformDataProvider::frames(self.document.read(cx)) as f64;
        (frames / width).max(MIN_SAMPLES_PER_PIXEL)
    }

    fn visible_samples(&self) -> f64 {
        self.samples_per_pixel * self.viewport_width.max(1.0) as f64
    }

    fn max_start(&self, cx: &App) -> f64 {
        let frames = self.frames(cx) as f64;
        (frames - self.visible_samples()).max(0.0)
    }

    fn clamp_scroll(&mut self, cx: &App) {
        self.samples_per_pixel = self
            .samples_per_pixel
            .clamp(MIN_SAMPLES_PER_PIXEL, self.max_samples_per_pixel(cx));
        self.start_sample = self.start_sample.clamp(0.0, self.max_start(cx));
    }

    fn zoom_at(&mut self, factor: f64, anchor_sample: f64, cx: &App) {
        let old = self.samples_per_pixel;
        if old <= 0.0 {
            return;
        }
        let pixel = (anchor_sample - self.start_sample) / old;
        self.samples_per_pixel =
            (old * factor).clamp(MIN_SAMPLES_PER_PIXEL, self.max_samples_per_pixel(cx));
        self.start_sample = anchor_sample - pixel * self.samples_per_pixel;
        self.clamp_scroll(cx);
    }

    fn sample_at_x(&self, x: f32) -> f64 {
        let local = (x - self.content_origin_x).max(0.0) as f64;
        self.start_sample + local * self.samples_per_pixel
    }

    fn set_hover_at(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        if self.content_height > 0.0
            && (y < self.content_origin_y || y > self.content_origin_y + self.content_height)
        {
            self.clear_hover(cx);
            return;
        }
        let next = hover_sample_from_x(
            x,
            self.content_origin_x,
            self.viewport_width,
            self.start_sample,
            self.samples_per_pixel,
            self.frames(cx),
        );
        if self.hover_sample != next {
            self.hover_sample = next;
            cx.notify();
        }
    }

    fn clear_hover(&mut self, cx: &mut Context<Self>) {
        if self.hover_sample.take().is_some() {
            cx.notify();
        }
    }

    fn set_pointer_over(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.pointer_over == hovered {
            return;
        }
        self.pointer_over = hovered;
        cx.notify();
    }

    fn pan_pixels(&mut self, dx: f32, cx: &App) {
        self.start_sample -= dx as f64 * self.samples_per_pixel;
        self.clamp_scroll(cx);
    }

    fn set_start_from_scrollbar_x(&mut self, x: f32, grab_offset: f32, cx: &App) {
        let track = self.scrollbar_width.max(1.0);
        let frames = self.frames(cx);
        let (thumb_w, _) = scrollbar_geom(frames, self.start_sample, self.samples_per_pixel, track);
        let max_travel = (track - thumb_w).max(1.0);
        let thumb_x = (x - self.scrollbar_origin_x - grab_offset).clamp(0.0, max_travel);
        let max_start = self.max_start(cx);
        self.start_sample = (thumb_x as f64 / max_travel as f64) * max_start;
        self.clamp_scroll(cx);
    }

    fn remember_viewport(
        &mut self,
        bounds: Bounds<Pixels>,
        reset_union: bool,
        cx: &mut Context<Self>,
    ) {
        let width = bounds.size.width.as_f32();
        let height = bounds.size.height.as_f32();
        let x = bounds.origin.x.as_f32();
        let y = bounds.origin.y.as_f32();
        if reset_union {
            self.content_origin_y = y;
            self.content_height = height;
        } else {
            let bottom = (self.content_origin_y + self.content_height).max(y + height);
            self.content_origin_y = self.content_origin_y.min(y);
            self.content_height = bottom - self.content_origin_y;
        }
        if width <= 1.0 {
            return;
        }
        let first = self.viewport_width <= 1.0;
        let changed = (self.viewport_width - width).abs() > 2.0;
        self.viewport_width = width;
        self.content_origin_x = x;
        if first {
            self.start_sample = 0.0;
            self.samples_per_pixel = self.max_samples_per_pixel(cx);
            cx.notify();
        } else if changed {
            self.clamp_scroll(cx);
            cx.notify();
        }
    }

    fn remember_scrollbar(&mut self, bounds: Bounds<Pixels>) {
        self.scrollbar_width = bounds.size.width.as_f32();
        self.scrollbar_origin_x = bounds.origin.x.as_f32();
    }

    fn marker_snap_radius(&self) -> usize {
        (MARKER_SNAP_VIEWPORT_FRACTION
            * f64::from(self.viewport_width.max(1.0))
            * self.samples_per_pixel)
            .round()
            .max(0.0) as usize
    }

    fn handle_drag_move(&mut self, x: f32, cx: &mut Context<Self>) {
        let drag = self.drag.take();
        self.drag = match drag {
            Some(Drag::SelectRegion {
                lane,
                alt,
                ctrl,
                shift,
                anchor_sample,
                origin_x,
                mut dragging,
            }) => {
                if !dragging && (x - origin_x).abs() >= DRAG_MOVE_THRESHOLD_PX {
                    dragging = true;
                }
                if dragging {
                    let sample = self.sample_at_x(x).round() as usize;
                    let radius = self.marker_snap_radius();
                    self.document.update(cx, |doc, cx| {
                        WaveformEditor::update_drag(doc, sample, radius);
                        cx.notify();
                    });
                    cx.notify();
                }
                Some(Drag::SelectRegion {
                    lane,
                    alt,
                    ctrl,
                    shift,
                    anchor_sample,
                    origin_x,
                    dragging,
                })
            }
            Some(Drag::Scrollbar { grab_offset }) => {
                self.set_start_from_scrollbar_x(x, grab_offset, cx);
                cx.notify();
                Some(Drag::Scrollbar { grab_offset })
            }
            None => None,
        };
    }

    fn end_drag(&mut self, x: f32, cx: &mut Context<Self>) {
        let drag = self.drag.take();
        let Some(drag) = drag else {
            return;
        };

        match drag {
            Drag::SelectRegion {
                lane,
                alt,
                ctrl,
                shift,
                anchor_sample,
                dragging,
                ..
            } => {
                let radius = self.marker_snap_radius();
                let sample = self.sample_at_x(x).round() as usize;
                self.document.update(cx, |doc, cx| {
                    let scope = WaveformEditor::channel_lanes(doc, lane, alt);
                    if dragging || shift {
                        if dragging {
                            WaveformEditor::update_drag(doc, sample, radius);
                        }
                        WaveformEditor::finish_drag(doc);
                    } else {
                        WaveformEditor::click_without_drag(doc, anchor_sample, scope, ctrl);
                    }
                    cx.notify();
                });
            }
            _ => {}
        }
        cx.notify();
    }
}

fn install_global_drag_listeners<D>(entity: Entity<WaveformDisplay<D>>, window: &mut Window)
where
    D: WaveformDataProvider + WaveformEditor + 'static,
{
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            entity.update(cx, |this, cx| {
                if this.drag.is_some() {
                    this.handle_drag_move(event.position.x.as_f32(), cx);
                }
                this.set_hover_at(event.position.x.as_f32(), event.position.y.as_f32(), cx);
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
                this.end_drag(event.position.x.as_f32(), cx);
            });
        }
    });
}

fn scrollbar_geom(frames: usize, start: f64, spp: f64, track: f32) -> (f32, f32) {
    if frames == 0 || track <= 0.0 {
        return (track, 0.0);
    }
    let visible = (spp * track as f64).max(1.0);
    let ratio = (visible / frames as f64).clamp(0.0, 1.0) as f32;
    let thumb_w = (track * ratio).max(MIN_THUMB).min(track);
    let max_start = (frames as f64 - visible).max(0.0);
    let thumb_x = if max_start <= f64::EPSILON {
        0.0
    } else {
        (start / max_start) as f32 * (track - thumb_w)
    };
    (thumb_w, thumb_x)
}

fn channel_color(theme: &gpui_kit::component::Theme, index: usize) -> gpui_kit::Hsla {
    match index % 5 {
        0 => theme.chart_1,
        1 => theme.chart_2,
        2 => theme.chart_3,
        3 => theme.chart_4,
        _ => theme.chart_5,
    }
}

fn rotate_hue(color: gpui_kit::Hsla, degrees: f32) -> gpui_kit::Hsla {
    let mut h = color.h + degrees / 360.0;
    if h >= 1.0 {
        h -= 1.0;
    }
    gpui_kit::Hsla { h, ..color }
}

fn sample_to_x(sample: f64, start_sample: f64, samples_per_pixel: f64, origin_x: f32) -> f32 {
    origin_x + ((sample - start_sample) / samples_per_pixel) as f32
}

/// Keep a 1px vertical marker inside the lane so the last sample stays visible
/// instead of painting on (or past) the clip edge.
fn clamp_bar_x(x: f32, origin_x: f32, width: f32) -> f32 {
    let max_x = (origin_x + width - 1.0).max(origin_x);
    x.clamp(origin_x, max_x)
}

fn region_tint(base_color: gpui_kit::Hsla, alpha: f32) -> gpui_kit::Hsla {
    rotate_hue(base_color, 10.0).alpha(alpha)
}

fn paint_region_endpoint(
    bounds: Bounds<Pixels>,
    sample: usize,
    channel: usize,
    channels: &LaneScope,
    start_sample: f64,
    samples_per_pixel: f64,
    base_color: gpui_kit::Hsla,
    window: &mut Window,
) {
    if !channels.applies_to(channel) {
        return;
    }
    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let height = bounds.size.height.as_f32();
    let x = clamp_bar_x(
        sample_to_x(sample as f64, start_sample, samples_per_pixel, origin_x),
        origin_x,
        bounds.size.width.as_f32(),
    );
    window.paint_quad(fill(
        Bounds {
            origin: point(px(x), px(origin_y)),
            size: size(px(1.0), px(height)),
        },
        region_tint(base_color, 0.2),
    ));
}

fn paint_region_overlay(
    bounds: Bounds<Pixels>,
    region: &PaintRegion,
    channel: usize,
    start_sample: f64,
    samples_per_pixel: f64,
    base_color: gpui_kit::Hsla,
    window: &mut Window,
) {
    if !region.channels.applies_to(channel) {
        return;
    }
    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let height = bounds.size.height.as_f32();
    let x0 = sample_to_x(
        region.start as f64,
        start_sample,
        samples_per_pixel,
        origin_x,
    );
    let x1 = sample_to_x(region.end as f64, start_sample, samples_per_pixel, origin_x);
    let left = x0.min(x1);
    let width = (x1 - x0).abs().max(1.0);
    window.paint_quad(fill(
        Bounds {
            origin: point(px(left), px(origin_y)),
            size: size(px(width), px(height)),
        },
        region_tint(base_color, 0.1),
    ));
    paint_region_endpoint(
        bounds,
        region.start,
        channel,
        &region.channels,
        start_sample,
        samples_per_pixel,
        base_color,
        window,
    );
    paint_region_endpoint(
        bounds,
        region.end,
        channel,
        &region.channels,
        start_sample,
        samples_per_pixel,
        base_color,
        window,
    );
}

fn marker_hsla(color: [f32; 4]) -> gpui_kit::Hsla {
    Rgba {
        r: color[0],
        g: color[1],
        b: color[2],
        a: color[3],
    }
    .into()
}

fn paint_marker_triangle(bounds: Bounds<Pixels>, x: f32, builder: &mut PathBuilder) {
    let origin_y = bounds.origin.y.as_f32();
    let half = MARKER_TRIANGLE_BASE / 2.0;
    builder.move_to(point(px(x - half), px(origin_y)));
    builder.line_to(point(px(x + half), px(origin_y)));
    builder.line_to(point(px(x), px(origin_y + MARKER_TRIANGLE_HEIGHT)));
    builder.close();
}

/// One slot per pixel column. Markers are ordered by frame, so adjacent
/// overview hits share a column and collapse to a single bar/triangle.
fn marker_paint_slots(
    markers: &[(u64, [f32; 4])],
    start_sample: f64,
    samples_per_pixel: f64,
    origin_x: f32,
    width: f32,
) -> Vec<(f32, [f32; 4])> {
    let vis_start = start_sample.max(0.0);
    let vis_end = start_sample + width as f64 * samples_per_pixel;
    let mut slots: Vec<(f32, [f32; 4])> = Vec::new();
    let mut last_col: Option<i32> = None;
    for &(frame, color) in markers {
        let sample = frame as f64;
        if sample + samples_per_pixel < vis_start || sample > vis_end {
            continue;
        }
        let x = clamp_bar_x(
            sample_to_x(sample, start_sample, samples_per_pixel, origin_x),
            origin_x,
            width,
        );
        let col = x.floor() as i32;
        if last_col == Some(col) {
            if let Some(slot) = slots.last_mut() {
                slot.1 = color;
            }
            continue;
        }
        last_col = Some(col);
        slots.push((x, color));
    }
    slots
}

fn paint_markers(
    bounds: Bounds<Pixels>,
    markers: &[(u64, [f32; 4])],
    channel: usize,
    start_sample: f64,
    samples_per_pixel: f64,
    window: &mut Window,
) {
    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let width = bounds.size.width.as_f32();
    let height = bounds.size.height.as_f32();
    let slots = marker_paint_slots(markers, start_sample, samples_per_pixel, origin_x, width);
    let mut triangles: Vec<([f32; 4], PathBuilder)> = Vec::new();
    for &(x, color) in &slots {
        let fill_color = marker_hsla(color);
        window.paint_quad(fill(
            Bounds {
                origin: point(px(x), px(origin_y)),
                size: size(px(1.0), px(height)),
            },
            fill_color.opacity(MARKER_BAR_OPACITY),
        ));
        if channel != 0 {
            continue;
        }
        let builder = if let Some((_, builder)) = triangles
            .iter_mut()
            .find(|(existing, _)| *existing == color)
        {
            builder
        } else {
            triangles.push((color, PathBuilder::fill()));
            &mut triangles.last_mut().unwrap().1
        };
        paint_marker_triangle(bounds, x + 0.5, builder);
    }
    if channel == 0 {
        for (color, builder) in triangles {
            if let Ok(path) = builder.build() {
                window.paint_path(path, marker_hsla(color));
            }
        }
    }
}

fn paint_vertical_bar(
    bounds: Bounds<Pixels>,
    sample: usize,
    start_sample: f64,
    samples_per_pixel: f64,
    color: gpui_kit::Hsla,
    window: &mut Window,
) {
    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let height = bounds.size.height.as_f32();
    let x = clamp_bar_x(
        sample_to_x(sample as f64, start_sample, samples_per_pixel, origin_x),
        origin_x,
        bounds.size.width.as_f32(),
    );
    window.paint_quad(fill(
        Bounds {
            origin: point(px(x), px(origin_y)),
            size: size(px(1.0), px(height)),
        },
        color,
    ));
}

fn range_x(
    start: u64,
    end: u64,
    start_sample: f64,
    samples_per_pixel: f64,
    origin_x: f32,
    width: f32,
) -> Option<(f32, f32)> {
    let x0 = sample_to_x(start as f64, start_sample, samples_per_pixel, origin_x);
    let x1 = sample_to_x(end as f64, start_sample, samples_per_pixel, origin_x);
    let left = x0.min(x1).max(origin_x);
    let right = x0.max(x1).min(origin_x + width);
    (right > left).then_some((left, right))
}

fn paint_range_overlays(
    bounds: Bounds<Pixels>,
    ranges: &[(u64, u64)],
    start_sample: f64,
    samples_per_pixel: f64,
    color: gpui_kit::Hsla,
    window: &mut Window,
) {
    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let width = bounds.size.width.as_f32();
    let height = bounds.size.height.as_f32();
    for &(start, end) in ranges {
        let Some((left, right)) =
            range_x(start, end, start_sample, samples_per_pixel, origin_x, width)
        else {
            continue;
        };
        window.paint_quad(fill(
            Bounds {
                origin: point(px(left), px(origin_y)),
                size: size(px((right - left).max(1.0)), px(height)),
            },
            color,
        ));
    }
}

fn paint_modified_bars(
    bounds: Bounds<Pixels>,
    ranges: &[(u64, u64)],
    start_sample: f64,
    samples_per_pixel: f64,
    window: &mut Window,
) {
    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let width = bounds.size.width.as_f32();
    let height = bounds.size.height.as_f32();
    if height < MODIFIED_BAR_HEIGHT {
        return;
    }
    let bar_y = origin_y + height - MODIFIED_BAR_HEIGHT;
    for (ix, &(start, end)) in ranges.iter().enumerate() {
        let Some((left, mut right)) =
            range_x(start, end, start_sample, samples_per_pixel, origin_x, width)
        else {
            continue;
        };
        let adjacent = ranges
            .get(ix + 1)
            .is_some_and(|&(next_start, _)| next_start == end);
        if adjacent {
            right = (right - MODIFIED_BAR_GAP).max(left);
        }
        let bar_w = right - left;
        if bar_w < 1.0 {
            continue;
        }
        window.paint_quad(fill(
            Bounds {
                origin: point(px(left), px(bar_y)),
                size: size(px(bar_w), px(MODIFIED_BAR_HEIGHT)),
            },
            MODIFIED_BAR_COLOR,
        ));
    }
}

fn paint_lane(
    bounds: Bounds<Pixels>,
    provider: &(impl WaveformDataProvider + WaveformEditor),
    channel: usize,
    start_sample: f64,
    samples_per_pixel: f64,
    color: gpui_kit::Hsla,
    zero_color: gpui_kit::Hsla,
    hover_sample: Option<usize>,
    modified_ranges: &[(u64, u64)],
    hover_ranges: &[(u64, u64)],
    markers: &[(u64, [f32; 4])],
    window: &mut Window,
) {
    let width = bounds.size.width.as_f32();
    let height = bounds.size.height.as_f32();
    if width < 1.0 || height < 1.0 || channel >= WaveformDataProvider::channel_count(provider) {
        return;
    }

    for region in WaveformEditor::named_regions(provider) {
        paint_region_overlay(
            bounds,
            &region,
            channel,
            start_sample,
            samples_per_pixel,
            color.opacity(0.55),
            window,
        );
    }
    for region in WaveformEditor::selection_regions(provider) {
        paint_region_overlay(
            bounds,
            &region,
            channel,
            start_sample,
            samples_per_pixel,
            color,
            window,
        );
    }

    let origin_x = bounds.origin.x.as_f32();
    let origin_y = bounds.origin.y.as_f32();
    let y_scale = ScaleLinear::new(vec![-1.0_f64, 1.0], vec![origin_y + height, origin_y]);

    if let Some(mid) = y_scale.tick(&0.0) {
        let mut builder = PathBuilder::stroke(px(1.0));
        builder.move_to(point(px(origin_x), px(mid)));
        builder.line_to(point(px(origin_x + width), px(mid)));
        if let Ok(path) = builder.build() {
            window.paint_path(path, zero_color);
        }
    }

    let frames = WaveformDataProvider::frames(provider);
    let cols = width.ceil() as usize;
    let peak_block = WaveformDataProvider::peak_block(provider);

    // Overview paint uses peak bins. Folding PCM while caches are still
    // building would decode on the UI thread and delay the progress UI.
    if WaveformDataProvider::peaks_ready(provider) {
        if samples_per_pixel < 1.0 {
            let mut builder = PathBuilder::stroke(px(1.2));
            let mut started = false;
            let first = start_sample.max(0.0).floor() as usize;
            let last = ((start_sample + width as f64 * samples_per_pixel).ceil() as usize)
                .min(frames.saturating_sub(1));
            if first <= last && frames > 0 {
                let mut samples = vec![0.0; last - first + 1];
                WaveformDataProvider::read_channel(provider, channel, first, &mut samples);
                for (offset, sample) in samples.iter().enumerate() {
                    let i = first + offset;
                    let x = origin_x + ((i as f64 - start_sample) / samples_per_pixel) as f32;
                    let y = y_scale
                        .tick(&(*sample as f64))
                        .unwrap_or(origin_y + height * 0.5);
                    if !started {
                        builder.move_to(point(px(x), px(y)));
                        started = true;
                    } else {
                        builder.line_to(point(px(x), px(y)));
                    }
                }
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        } else {
            let first = start_sample.max(0.0).floor() as usize;
            let last =
                ((start_sample + cols as f64 * samples_per_pixel).ceil() as usize).min(frames);
            let visible = last.saturating_sub(first);
            let fold_from_samples = samples_per_pixel < peak_block as f64
                && visible > 0
                && visible <= cols.saturating_mul(64);

            if fold_from_samples {
                let mut samples = vec![0.0; visible];
                WaveformDataProvider::read_channel(provider, channel, first, &mut samples);
                for col in 0..cols {
                    let bin_start = start_sample + col as f64 * samples_per_pixel;
                    let bin_end = bin_start + samples_per_pixel;
                    if bin_start >= frames as f64 {
                        break;
                    }
                    let a = (bin_start.floor() as usize)
                        .saturating_sub(first)
                        .min(samples.len());
                    let b = (bin_end.ceil() as usize)
                        .saturating_sub(first)
                        .clamp(a, samples.len());
                    let (min, max) = min_max_of(&samples[a..b]);
                    paint_column(
                        origin_x, col, min, max, &y_scale, origin_y, height, color, window,
                    );
                }
            } else {
                let mut columns = vec![(0.0f32, 0.0f32); cols];
                WaveformDataProvider::fill_minmax_columns(
                    provider,
                    channel,
                    start_sample,
                    samples_per_pixel,
                    &mut columns,
                );
                for (col, &(min, max)) in columns.iter().enumerate() {
                    let bin_start = start_sample + col as f64 * samples_per_pixel;
                    if bin_start >= frames as f64 {
                        break;
                    }
                    paint_column(
                        origin_x, col, min, max, &y_scale, origin_y, height, color, window,
                    );
                }
            }
        }
    }

    paint_range_overlays(
        bounds,
        hover_ranges,
        start_sample,
        samples_per_pixel,
        MODIFIED_HOVER_FILL,
        window,
    );
    paint_modified_bars(
        bounds,
        modified_ranges,
        start_sample,
        samples_per_pixel,
        window,
    );
    paint_markers(
        bounds,
        markers,
        channel,
        start_sample,
        samples_per_pixel,
        window,
    );

    if let Some(sample) = hover_sample {
        paint_vertical_bar(
            bounds,
            sample,
            start_sample,
            samples_per_pixel,
            GHOST_BAR_COLOR,
            window,
        );
    }

    if let Some((sample, lanes)) = WaveformEditor::playhead(provider) {
        if lanes.applies_to(channel) {
            paint_vertical_bar(
                bounds,
                sample,
                start_sample,
                samples_per_pixel,
                POSITION_BAR_COLOR,
                window,
            );
        }
    }
}

fn min_max_of(samples: &[f32]) -> (f32, f32) {
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for &s in samples {
        min = min.min(s);
        max = max.max(s);
    }
    if min > max {
        (0.0, 0.0)
    } else {
        (min, max)
    }
}

fn paint_column(
    origin_x: f32,
    col: usize,
    min: f32,
    max: f32,
    y_scale: &ScaleLinear<f64>,
    origin_y: f32,
    height: f32,
    color: gpui_kit::Hsla,
    window: &mut Window,
) {
    let y_max = y_scale.tick(&(max as f64)).unwrap_or(origin_y);
    let y_min = y_scale.tick(&(min as f64)).unwrap_or(origin_y + height);
    let top = y_max.min(y_min);
    let bar_h = (y_max - y_min).abs().max(1.0);
    window.paint_quad(fill(
        Bounds {
            origin: point(px(origin_x + col as f32), px(top)),
            size: size(px(1.0), px(bar_h)),
        },
        color,
    ));
}

fn paint_scrollbar(
    bounds: Bounds<Pixels>,
    frames: usize,
    start_sample: f64,
    samples_per_pixel: f64,
    track: gpui_kit::Hsla,
    thumb: gpui_kit::Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(bounds, track));
    let (thumb_w, thumb_x) = scrollbar_geom(
        frames,
        start_sample,
        samples_per_pixel,
        bounds.size.width.as_f32(),
    );
    window.paint_quad(fill(
        Bounds {
            origin: point(px(bounds.origin.x.as_f32() + thumb_x), bounds.origin.y),
            size: size(px(thumb_w), bounds.size.height),
        },
        thumb,
    ));
}

impl<D> Focusable for WaveformDisplay<D>
where
    D: WaveformDataProvider + WaveformEditor + 'static,
{
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D> Render for WaveformDisplay<D>
where
    D: WaveformDataProvider + WaveformEditor + 'static,
{
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.clamp_scroll(cx);
        let theme = cx.theme().clone();
        let document = self.document.clone();
        let snap = WaveformEditor::snap_zero_crossings(self.document.read(cx));
        let start_sample = self.start_sample;
        let samples_per_pixel = self.samples_per_pixel;
        let hover_sample = self.hover_sample;
        let (modified_ranges, hover_ranges, markers) = {
            let doc = self.document.read(cx);
            let hovered = self.hovered_edit;
            let modified = WaveformEditor::modified_ranges(doc);
            let hover = hovered
                .map(|id| WaveformEditor::ranges_for_edit(doc, id))
                .unwrap_or_default();
            let markers = WaveformEditor::markers_for_paint(doc);
            (modified, hover, markers)
        };
        let entity = cx.entity();
        let channel_count = WaveformDataProvider::channel_count(self.document.read(cx));
        let is_empty = WaveformDataProvider::frames(self.document.read(cx)) == 0;
        let job_progress = WaveformEditor::peak_status(self.document.read(cx));

        v_flex()
            .id("waveform-root")
            .key_context("Waveform")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.focus_handle.focus(window, cx);
                }),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.set_pointer_over(*hovered, cx);
            }))
            .child(
                div()
                    .id("waveform")
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .overflow_y_scroll()
                    .on_action(cx.listener(|this, _: &ToggleZeroCrossing, _, cx| {
                        this.document
                            .update(cx, |doc, _| WaveformEditor::toggle_zero_crossing_snap(doc));
                        cx.notify();
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        if WaveformDataProvider::frames(this.document.read(cx)) == 0 {
                            return;
                        }
                        let delta = event.delta.pixel_delta(px(16.));
                        let dx = delta.x.as_f32();
                        let dy = delta.y.as_f32();
                        if event.modifiers.shift {
                            let mag = if dx.abs() > dy.abs() { dx } else { dy };
                            let factor = if mag < 0.0 {
                                1.0 / ZOOM_FACTOR
                            } else {
                                ZOOM_FACTOR
                            };
                            let anchor = this.sample_at_x(event.position.x.as_f32());
                            this.zoom_at(factor, anchor, cx);
                        } else {
                            let pan = if dx.abs() > dy.abs() { dx } else { dy };
                            this.pan_pixels(pan, cx);
                        }
                        cx.notify();
                    }))
                    .when(is_empty, |this| {
                        this.flex().items_center().justify_center().child(
                            div()
                                .text_center()
                                .text_color(theme.muted_foreground)
                                .child("Drop an audio file here or use File → Open…"),
                        )
                    })
                    .when(!is_empty, |this| {
                        this.child(
                            v_flex()
                                .id("waveform-lanes")
                                .w_full()
                                .h_full()
                                .on_mouse_move(cx.listener(
                                    |this, event: &MouseMoveEvent, _, cx| {
                                        this.set_hover_at(
                                            event.position.x.as_f32(),
                                            event.position.y.as_f32(),
                                            cx,
                                        );
                                    },
                                ))
                                .children((0..channel_count).map(|ch| {
                                    let document = document.clone();
                                    let entity = entity.clone();
                                    let color = channel_color(&theme, ch);
                                    let zero = theme.border;
                                    let channel_label =
                                        WaveformDataProvider::channel_label(document.read(cx), ch);
                                    h_flex()
                                    .id(SharedString::from(format!("lane-{ch}")))
                                    .w_full()
                                    .flex_1()
                                    .min_h(px(MIN_LANE_HEIGHT))
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .w(px(48.))
                                            .flex_none()
                                            .h_full()
                                            .pt(rems(0.5))
                                            .pl(rems(0.5))
                                            .border_r_1()
                                            .border_color(theme.border)
                                            .text_xs()
                                            .font_semibold()
                                            .text_color(theme.muted_foreground)
                                            .child(channel_label),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .h_full()
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    move |this, event: &MouseDownEvent, _, cx| {
                                                        let x = event.position.x.as_f32();
                                                        let sample =
                                                            this.sample_at_x(x).round() as usize;
                                                        if event.modifiers.shift {
                                                            this.drag =
                                                                Some(Drag::SelectRegion {
                                                                    lane: ch,
                                                                    alt: event.modifiers.alt,
                                                                    ctrl: event.modifiers.control,
                                                                    shift: true,
                                                                    anchor_sample: sample,
                                                                    origin_x: x,
                                                                    dragging: false,
                                                                });
                                                            let alt = event.modifiers.alt;
                                                            let radius = this.marker_snap_radius();
                                                            this.document.update(cx, |doc, cx| {
                                                                let scope = WaveformEditor::channel_lanes(
                                                                    doc, ch, alt,
                                                                );
                                                                WaveformEditor::begin_extend(
                                                                    doc, sample, scope, radius,
                                                                );
                                                                cx.notify();
                                                            });
                                                        } else {
                                                            let alt = event.modifiers.alt;
                                                            let ctrl = event.modifiers.control;
                                                            let radius = this.marker_snap_radius();
                                                            this.document.update(cx, |doc, cx| {
                                                                let scope = WaveformEditor::channel_lanes(
                                                                    doc, ch, alt,
                                                                );
                                                                if ctrl {
                                                                    WaveformEditor::begin_disjoint(
                                                                        doc, sample, scope, radius,
                                                                    );
                                                                } else {
                                                                    WaveformEditor::begin_replace(
                                                                        doc, sample, scope, radius,
                                                                    );
                                                                }
                                                                cx.notify();
                                                            });
                                                            this.drag = Some(Drag::SelectRegion {
                                                                lane: ch,
                                                                alt,
                                                                ctrl,
                                                                shift: false,
                                                                anchor_sample: sample,
                                                                origin_x: x,
                                                                dragging: false,
                                                            });
                                                        }
                                                        cx.notify();
                                                    },
                                                ),
                                            )
                                            .child(
                                                canvas(
                                                    {
                                                        let entity = entity.clone();
                                                        move |bounds, _, cx| {
                                                            entity.update(cx, |this, cx| {
                                                                this.remember_viewport(
                                                                    bounds,
                                                                    ch == 0,
                                                                    cx,
                                                                );
                                                            });
                                                            bounds
                                                        }
                                                    },
                                                    {
                                                        let document = document.clone();
                                                        let modified_ranges =
                                                            modified_ranges.clone();
                                                        let hover_ranges = hover_ranges.clone();
                                                        let markers = markers.clone();
                                                        move |bounds, _, window, cx| {
                                                            let provider = document.read(cx);
                                                            paint_lane(
                                                                bounds,
                                                                &*provider,
                                                                ch,
                                                                start_sample,
                                                                samples_per_pixel,
                                                                color,
                                                                zero,
                                                                hover_sample,
                                                                &modified_ranges,
                                                                &hover_ranges,
                                                                &markers,
                                                                window,
                                                            );
                                                        }
                                                    },
                                                )
                                                .size_full(),
                                            ),
                                    )
                                })),
                        )
                    })
                    .context_menu(move |menu, _, _| {
                        menu.menu_with_check("Zero Crossing", snap, Box::new(ToggleZeroCrossing))
                    }),
            )
            .when(!is_empty, |this| {
                this.child({
                    let frames = WaveformDataProvider::frames(self.document.read(cx));
                    let start = start_sample;
                    let spp = samples_per_pixel;
                    let track = theme.scrollbar;
                    let thumb = theme.scrollbar_thumb;
                    let entity = entity.clone();
                    h_flex()
                        .id("h-scroll")
                        .w_full()
                        .h(px(SCROLLBAR_HEIGHT))
                        .flex_none()
                        .border_t_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .w(px(48.))
                                .h_full()
                                .flex_none()
                                .border_r_1()
                                .border_color(theme.border),
                        )
                        .child(
                            div()
                                .id("h-scroll-track")
                                .flex_1()
                                .h_full()
                                .child(
                                    canvas(
                                        {
                                            let entity = entity.clone();
                                            move |bounds, _, cx| {
                                                entity.update(cx, |this, _cx| {
                                                    this.remember_scrollbar(bounds);
                                                });
                                                bounds
                                            }
                                        },
                                        move |bounds, _, window, _cx| {
                                            paint_scrollbar(
                                                bounds, frames, start, spp, track, thumb, window,
                                            );
                                            install_global_drag_listeners(entity.clone(), window);
                                        },
                                    )
                                    .size_full(),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                        let x = event.position.x.as_f32();
                                        let track_w = this.scrollbar_width.max(1.0);
                                        let frames = this.frames(cx);
                                        let (thumb_w, thumb_x) = scrollbar_geom(
                                            frames,
                                            this.start_sample,
                                            this.samples_per_pixel,
                                            track_w,
                                        );
                                        let local = x - this.scrollbar_origin_x;
                                        let grab_offset =
                                            if local >= thumb_x && local <= thumb_x + thumb_w {
                                                local - thumb_x
                                            } else {
                                                thumb_w * 0.5
                                            };
                                        this.set_start_from_scrollbar_x(x, grab_offset, cx);
                                        this.drag = Some(Drag::Scrollbar { grab_offset });
                                        cx.notify();
                                    }),
                                ),
                        )
                })
            })
            .when_some(job_progress, |this, state| {
                let fill = theme.primary;
                let track = theme.border;
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(2.))
                        .flex_none()
                        .bg(track)
                        .child(div().h_full().w(relative(state.fraction)).bg(fill)),
                )
            })
    }
}

fn hover_sample_from_x(
    x: f32,
    content_origin_x: f32,
    viewport_width: f32,
    start_sample: f64,
    samples_per_pixel: f64,
    frames: usize,
) -> Option<usize> {
    if frames == 0 || viewport_width <= 0.0 {
        return None;
    }
    if x < content_origin_x || x > content_origin_x + viewport_width {
        return None;
    }
    let local = (x - content_origin_x).max(0.0) as f64;
    let sample = start_sample + local * samples_per_pixel;
    Some(sample.round().clamp(0.0, (frames - 1) as f64) as usize)
}

fn max_samples_per_pixel_for(frames: f64, viewport_width: f32) -> f64 {
    let width = viewport_width.max(1.0) as f64;
    (frames / width).max(MIN_SAMPLES_PER_PIXEL)
}

fn clamp_viewport(
    start_sample: &mut f64,
    samples_per_pixel: &mut f64,
    viewport_width: f32,
    frames: f64,
) {
    *samples_per_pixel = samples_per_pixel.clamp(
        MIN_SAMPLES_PER_PIXEL,
        max_samples_per_pixel_for(frames, viewport_width),
    );
    let visible = *samples_per_pixel * viewport_width.max(1.0) as f64;
    let max_start = (frames - visible).max(0.0);
    *start_sample = start_sample.clamp(0.0, max_start);
}

fn apply_fit_range(
    start_sample: &mut f64,
    samples_per_pixel: &mut f64,
    viewport_width: f32,
    frames: f64,
    range_start: f64,
    range_end: f64,
) {
    let width = viewport_width.max(1.0) as f64;
    let span = (range_end - range_start).max(1.0);
    let pad = span * FRAME_PADDING;
    let padded_start = (range_start - pad).max(0.0);
    let padded_end = (range_end + pad).min(frames.max(padded_start + 1.0));
    *samples_per_pixel = ((padded_end - padded_start) / width).max(MIN_SAMPLES_PER_PIXEL);
    *start_sample = padded_start;
    clamp_viewport(start_sample, samples_per_pixel, viewport_width, frames);
}

fn apply_scroll_to_frame(
    start_sample: &mut f64,
    samples_per_pixel: f64,
    viewport_width: f32,
    frames: f64,
    sample: f64,
) {
    let mut spp = samples_per_pixel;
    let visible = spp * viewport_width.max(1.0) as f64;
    let margin = visible * FRAME_PADDING;
    if sample < *start_sample + margin {
        *start_sample = (sample - margin).max(0.0);
    } else if sample > *start_sample + visible - margin {
        *start_sample = sample + margin - visible;
    }
    clamp_viewport(start_sample, &mut spp, viewport_width, frames);
}

fn apply_scroll_ranges_into_view(
    start_sample: &mut f64,
    samples_per_pixel: f64,
    viewport_width: f32,
    frames: f64,
    ranges: &[(u64, u64)],
) {
    if ranges.is_empty() {
        return;
    }
    let mut spp = samples_per_pixel;
    let visible = spp * viewport_width.max(1.0) as f64;
    let view_start = *start_sample;
    let view_end = view_start + visible;
    let already_visible = ranges
        .iter()
        .any(|&(start, end)| (start as f64) < view_end && (end as f64) > view_start);
    if already_visible {
        return;
    }
    let (range_start, range_end) = ranges[0];
    let range_start = range_start as f64;
    let range_end = range_end as f64;
    let span = (range_end - range_start).max(1.0);
    let margin = visible * FRAME_PADDING;
    if span + 2.0 * margin <= visible {
        *start_sample = range_start - (visible - span) / 2.0;
    } else {
        *start_sample = range_start - margin;
    }
    clamp_viewport(start_sample, &mut spp, viewport_width, frames);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_range_zooms_to_region_with_padding() {
        let mut start = 0.0;
        let mut spp = 10.0;
        apply_fit_range(&mut start, &mut spp, 100.0, 10_000.0, 1000.0, 2000.0);
        let span = 1000.0;
        let pad = span * FRAME_PADDING;
        let expected_spp = (span + 2.0 * pad) / 100.0;
        assert!((spp - expected_spp).abs() < 1e-9);
        assert!((start - (1000.0 - pad)).abs() < 1e-9);
    }

    #[test]
    fn scroll_to_frame_pans_without_changing_zoom() {
        let mut start = 0.0;
        let spp = 10.0;
        apply_scroll_to_frame(&mut start, spp, 100.0, 10_000.0, 8000.0);
        // visible = 1000, margin = 100; start = 8000 + 100 - 1000 = 7100
        assert!((start - 7100.0).abs() < 1e-9);
        let visible = spp * 100.0;
        assert!((visible - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn scroll_to_frame_is_noop_when_position_already_visible() {
        let mut start = 4000.0;
        let spp = 10.0;
        apply_scroll_to_frame(&mut start, spp, 100.0, 10_000.0, 4500.0);
        assert!((start - 4000.0).abs() < 1e-9);
    }

    #[test]
    fn scroll_ranges_into_view_pans_when_offscreen() {
        let mut start = 0.0;
        let spp = 10.0;
        apply_scroll_ranges_into_view(&mut start, spp, 100.0, 10_000.0, &[(5000, 5100)]);
        let visible = 1000.0;
        let span = 100.0;
        let expected = 5000.0 - (visible - span) / 2.0;
        assert!((start - expected).abs() < 1e-9);
    }

    #[test]
    fn scroll_ranges_into_view_is_noop_when_any_range_visible() {
        let mut start = 4000.0;
        let spp = 10.0;
        apply_scroll_ranges_into_view(&mut start, spp, 100.0, 10_000.0, &[(4500, 4600)]);
        assert!((start - 4000.0).abs() < 1e-9);
    }

    #[test]
    fn hover_sample_maps_pixel_inside_viewport() {
        let sample = hover_sample_from_x(150.0, 50.0, 100.0, 1000.0, 10.0, 10_000);
        assert_eq!(sample, Some(2000));
    }

    #[test]
    fn hover_sample_is_none_outside_viewport_or_empty_buffer() {
        assert_eq!(
            hover_sample_from_x(40.0, 50.0, 100.0, 0.0, 10.0, 10_000),
            None
        );
        assert_eq!(
            hover_sample_from_x(160.0, 50.0, 100.0, 0.0, 10.0, 10_000),
            None
        );
        assert_eq!(hover_sample_from_x(80.0, 50.0, 100.0, 0.0, 10.0, 0), None);
    }

    #[test]
    fn hover_sample_clamps_to_last_frame() {
        let sample = hover_sample_from_x(149.0, 50.0, 100.0, 0.0, 100.0, 50);
        assert_eq!(sample, Some(49));
    }

    #[test]
    fn bar_x_stays_on_last_pixel_at_buffer_end() {
        // Fitted: 1000 frames across 100px → last sample is 0.1px short of the
        // right edge, which would clip a 1px bar. Keep it on the last pixel.
        let origin = 10.0;
        let width = 100.0;
        let x = sample_to_x(999.0, 0.0, 10.0, origin);
        assert!((x - 109.9).abs() < 1e-3);
        assert_eq!(clamp_bar_x(x, origin, width), origin + width - 1.0);
    }

    #[test]
    fn overview_markers_collapse_to_one_column() {
        let blue = [0.0, 0.0, 1.0, 1.0];
        let yellow = [1.0, 1.0, 0.0, 1.0];
        let markers = [(0, blue), (100, yellow), (200, blue)];
        let zoomed = marker_paint_slots(&markers, 0.0, 100.0, 0.0, 10.0);
        assert_eq!(zoomed.len(), 3);
        let overview = marker_paint_slots(&markers, 0.0, 1000.0, 0.0, 10.0);
        assert_eq!(overview.len(), 1);
        assert_eq!(overview[0].1, blue);
    }

    #[test]
    fn offscreen_markers_are_not_slotted() {
        let blue = [0.0, 0.0, 1.0, 1.0];
        let markers = [(0, blue), (1_400, blue), (9_000, blue)];
        let slots = marker_paint_slots(&markers, 1000.0, 10.0, 0.0, 100.0);
        assert_eq!(slots.len(), 1);
        assert!((slots[0].0 - 40.0).abs() < 1e-3);
    }
}
