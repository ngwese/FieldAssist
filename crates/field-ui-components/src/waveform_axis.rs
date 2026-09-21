// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Vertical axis math for peak (dB) and spectrum (Hz) scale gutters.
//!
//! Peak paint stays linear in amplitude (`[-1, 1]`); gutters and hover label
//! that axis in dBFS. Spectrum paint is linear in log-band index; labels use
//! the continuous log-Hz mapping that matches analysis band edges.

use crate::waveform_data::WaveformRepresentation;

/// Minimum spacing between spectrum axis tick labels.
pub const AXIS_LABEL_MIN_GAP_PX: f32 = 24.0;
/// Minimum spacing between peak axis tick labels (~25% denser than spectrum).
pub const PEAK_AXIS_LABEL_MIN_GAP_PX: f32 = 19.2;
/// Font size used when painting gutter labels (must match paint).
pub const AXIS_LABEL_FONT_PX: f32 = 10.0;
/// Lowest frequency edge for log spectrum bands (Hz), matching analysis.
pub const SPECTRAL_FMIN_HZ: f32 = 20.0;
/// Amplitude below this maps to −∞ dB for hover / midline.
const AMP_DB_EPSILON: f32 = 1e-6;

fn label_half_height() -> f32 {
    AXIS_LABEL_FONT_PX * 0.5
}

/// Y-axis value under the pointer over a peaks or spectrum pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveformHoverAxis {
    /// Peak-pane amplitude as dBFS (`f32::NEG_INFINITY` at the zero line).
    PeakDb(f32),
    /// Spectrum-pane frequency in Hz.
    SpectrumHz(f32),
}

/// One painted tick on a vertical scale gutter.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisTick {
    /// Pixel Y of the tick (absolute, same space as pane `origin_y`).
    pub y: f32,
    /// Label text (`"0"`, `"-∞"`, `"1k"`, …).
    pub label: String,
}

/// Convert linear amplitude magnitude to dBFS (`NEG_INFINITY` near zero).
pub fn amplitude_to_db(amplitude: f32) -> f32 {
    let mag = amplitude.abs();
    if mag < AMP_DB_EPSILON {
        f32::NEG_INFINITY
    } else {
        20.0 * mag.log10()
    }
}

/// Convert dBFS to linear amplitude magnitude (`0` for −∞).
pub fn db_to_amplitude(db: f32) -> f32 {
    if db.is_infinite() && db.is_sign_negative() {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Map amplitude in `[-1, 1]` to pixel Y (top = +1, bottom = −1).
pub fn amplitude_to_y(amplitude: f32, origin_y: f32, height: f32) -> f32 {
    // Same orientation as ScaleLinear[-1, 1] → [origin_y+height, origin_y].
    let t = ((amplitude as f64 + 1.0) / 2.0) as f32;
    origin_y + height * (1.0 - t)
}

/// Inverse of [`amplitude_to_y`].
pub fn y_to_amplitude(y: f32, origin_y: f32, height: f32) -> f32 {
    if height <= 0.0 {
        return 0.0;
    }
    let t = ((y - origin_y) / height).clamp(0.0, 1.0);
    1.0 - 2.0 * t
}

/// Map pointer Y in a peaks pane to dBFS.
pub fn y_to_peak_db(y: f32, origin_y: f32, height: f32) -> f32 {
    amplitude_to_db(y_to_amplitude(y, origin_y, height))
}

/// Log-frequency domain matching analysis (`~20 Hz` … Nyquist).
pub fn spectral_freq_range(sample_rate: u32) -> (f32, f32) {
    let nyquist = sample_rate as f32 * 0.5;
    let f_min = SPECTRAL_FMIN_HZ.min(nyquist * 0.5).max(1.0);
    let f_max = nyquist.max(f_min * 1.01);
    (f_min, f_max)
}

/// Fraction `t` in `[0, 1]` for frequency on the log spectrum axis (0 = fmin).
pub fn freq_to_log_t(freq_hz: f32, sample_rate: u32) -> f32 {
    let (f_min, f_max) = spectral_freq_range(sample_rate);
    let freq = freq_hz.clamp(f_min, f_max);
    let log_min = f_min.ln();
    let log_max = f_max.ln();
    if (log_max - log_min).abs() < f32::EPSILON {
        return 0.0;
    }
    ((freq.ln() - log_min) / (log_max - log_min)).clamp(0.0, 1.0)
}

/// Inverse of [`freq_to_log_t`].
pub fn log_t_to_freq(t: f32, sample_rate: u32) -> f32 {
    let (f_min, f_max) = spectral_freq_range(sample_rate);
    let t = t.clamp(0.0, 1.0);
    let log_min = f_min.ln();
    let log_max = f_max.ln();
    (log_min + (log_max - log_min) * t).exp()
}

/// Map frequency to pixel Y (low Hz at bottom).
pub fn freq_to_y(freq_hz: f32, sample_rate: u32, origin_y: f32, height: f32) -> f32 {
    let t = freq_to_log_t(freq_hz, sample_rate);
    origin_y + height * (1.0 - t)
}

/// Map pointer Y in a spectrum pane to Hz.
pub fn y_to_freq(y: f32, sample_rate: u32, origin_y: f32, height: f32) -> f32 {
    if height <= 0.0 {
        return spectral_freq_range(sample_rate).0;
    }
    let t = 1.0 - ((y - origin_y) / height).clamp(0.0, 1.0);
    log_t_to_freq(t, sample_rate)
}

/// Format a spectrum scale tick (`"20"`, `"1k"`, `"2.5k"`).
pub fn format_hz_tick(freq_hz: f32) -> String {
    if freq_hz < 1000.0 {
        format!("{}", freq_hz.round() as i32)
    } else {
        let k = freq_hz / 1000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}k", k.round() as i32)
        } else {
            let tenths = (k * 10.0).round() / 10.0;
            if (tenths * 10.0).round() % 10.0 == 0.0 {
                format!("{}k", tenths as i32)
            } else {
                format!("{tenths:.1}k")
            }
        }
    }
}

/// Format hover Hz readout (`"432 Hz"`, `"1.23kHz"`).
pub fn format_hz_hover(freq_hz: f32) -> String {
    if freq_hz < 1000.0 {
        format!("{} Hz", freq_hz.round() as i32)
    } else {
        let k = freq_hz / 1000.0;
        if k >= 10.0 {
            format!("{:.1}kHz", k)
        } else {
            format!("{:.2}kHz", k)
        }
    }
}

/// Format hover / axis dB (`"-12.3 dB"`, `"-∞ dB"`).
pub fn format_db_hover(db: f32) -> String {
    if db.is_infinite() && db.is_sign_negative() {
        "-∞ dB".into()
    } else {
        format!("{db:.1} dB")
    }
}

/// Format a peaks scale tick (`"0"`, `"-6"`, `"-∞"`).
pub fn format_db_tick(db: f32) -> String {
    if db.is_infinite() && db.is_sign_negative() {
        "-∞".into()
    } else if db == 0.0 {
        "0".into()
    } else {
        format!("{}", db as i32)
    }
}

/// Whether a label centered on `y` fits inside the pane without clipping.
fn label_fits_in_pane(y: f32, origin_y: f32, height: f32) -> bool {
    let half = label_half_height();
    let top = origin_y + half;
    let bottom = origin_y + height - half;
    bottom >= top && y >= top && y <= bottom
}

/// Finite dB levels for peak gutters, outer (−3 dBFS) first so thinning prefers
/// near-full-scale pairs when space is limited. `0` is omitted: it sits on the
/// pane edge and cannot both fit a label and align with ±1.0 peaks.
const PEAK_DB_CANDIDATES: &[f32] = &[-3.0, -6.0, -12.0, -24.0, -48.0];

/// Shared peak-axis tick selection (which levels to show), independent of lane Y.
///
/// Computed once from a reference pane height (first channel) and reused so every
/// lane shows the same dB labels.
#[derive(Debug, Clone, PartialEq)]
pub struct PeakAxisLayout {
    /// When true, include the center `-∞` label.
    pub show_neg_inf: bool,
    /// Finite dB levels; each is drawn as a symmetric ±amplitude pair.
    pub db_levels: Vec<f32>,
}

/// Shared spectrum-axis tick selection (which frequencies to show).
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumAxisLayout {
    /// Frequencies in Hz to label.
    pub freqs_hz: Vec<f32>,
}

/// Choose peak-axis levels for a pane of the given height (full pane, no clip).
pub fn peak_axis_layout(height: f32) -> PeakAxisLayout {
    if height < 1.0 {
        return PeakAxisLayout {
            show_neg_inf: false,
            db_levels: Vec::new(),
        };
    }
    // Layout math is origin-independent; use a local pane at y=0.
    let origin_y = 0.0;
    let mid_y = amplitude_to_y(0.0, origin_y, height);
    if !label_fits_in_pane(mid_y, origin_y, height) {
        return PeakAxisLayout {
            show_neg_inf: false,
            db_levels: Vec::new(),
        };
    }

    let mut db_levels = Vec::new();
    let mut last_above_y = mid_y;
    for &db in PEAK_DB_CANDIDATES {
        let amp = db_to_amplitude(db);
        if amp <= AMP_DB_EPSILON {
            continue;
        }
        let y_above = amplitude_to_y(amp, origin_y, height);
        let y_below = amplitude_to_y(-amp, origin_y, height);
        if !label_fits_in_pane(y_above, origin_y, height)
            || !label_fits_in_pane(y_below, origin_y, height)
        {
            continue;
        }
        if (y_above - mid_y).abs() < PEAK_AXIS_LABEL_MIN_GAP_PX {
            continue;
        }
        if (last_above_y - y_above).abs() < PEAK_AXIS_LABEL_MIN_GAP_PX {
            continue;
        }
        db_levels.push(db);
        last_above_y = y_above;
    }

    PeakAxisLayout {
        show_neg_inf: true,
        db_levels,
    }
}

/// Place a [`PeakAxisLayout`] into a concrete pane as paint ticks.
pub fn peak_ticks_for_layout(layout: &PeakAxisLayout, origin_y: f32, height: f32) -> Vec<AxisTick> {
    if height < 1.0 {
        return Vec::new();
    }
    let mut ticks = Vec::new();
    if layout.show_neg_inf {
        ticks.push(AxisTick {
            y: amplitude_to_y(0.0, origin_y, height),
            label: format_db_tick(f32::NEG_INFINITY),
        });
    }
    for &db in &layout.db_levels {
        let amp = db_to_amplitude(db);
        if amp <= AMP_DB_EPSILON {
            continue;
        }
        let label = format_db_tick(db);
        ticks.push(AxisTick {
            y: amplitude_to_y(amp, origin_y, height),
            label: label.clone(),
        });
        ticks.push(AxisTick {
            y: amplitude_to_y(-amp, origin_y, height),
            label,
        });
    }
    ticks.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
    ticks
}

fn hz_candidates(f_min: f32, f_max: f32) -> Vec<f32> {
    let mut out = Vec::new();
    let mut decade = 1.0f32;
    while decade * 0.1 < f_max * 10.0 {
        for &m in &[1.0f32, 2.0, 5.0] {
            let f = m * decade;
            if f >= f_min * 0.99 && f <= f_max * 1.01 {
                out.push(f.clamp(f_min, f_max));
            }
        }
        decade *= 10.0;
        if decade > f_max * 100.0 {
            break;
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    out
}

/// Choose spectrum-axis frequencies for a pane of the given height.
pub fn spectrum_axis_layout(sample_rate: u32, height: f32) -> SpectrumAxisLayout {
    if height < 1.0 || sample_rate == 0 {
        return SpectrumAxisLayout {
            freqs_hz: Vec::new(),
        };
    }
    let origin_y = 0.0;
    let (f_min, f_max) = spectral_freq_range(sample_rate);
    let mut pairs: Vec<(f32, f32)> = hz_candidates(f_min, f_max)
        .into_iter()
        .map(|freq| (freq, freq_to_y(freq, sample_rate, origin_y, height)))
        .filter(|&(_, y)| label_fits_in_pane(y, origin_y, height))
        .collect();
    pairs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut freqs_hz = Vec::new();
    let mut last_y: Option<f32> = None;
    for (freq, y) in pairs {
        if let Some(y0) = last_y {
            if (y - y0).abs() < AXIS_LABEL_MIN_GAP_PX {
                continue;
            }
        }
        freqs_hz.push(freq);
        last_y = Some(y);
    }
    SpectrumAxisLayout { freqs_hz }
}

/// Place a [`SpectrumAxisLayout`] into a concrete pane as paint ticks.
pub fn spectrum_ticks_for_layout(
    layout: &SpectrumAxisLayout,
    sample_rate: u32,
    origin_y: f32,
    height: f32,
) -> Vec<AxisTick> {
    if height < 1.0 || sample_rate == 0 {
        return Vec::new();
    }
    let mut ticks: Vec<_> = layout
        .freqs_hz
        .iter()
        .map(|&freq| AxisTick {
            y: freq_to_y(freq, sample_rate, origin_y, height),
            label: format_hz_tick(freq),
        })
        .collect();
    ticks.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
    ticks
}

/// Which pane a Y falls into for the active representation / split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisPane {
    /// Linear peak amplitude pane.
    Peaks,
    /// Log-frequency spectrum pane.
    Spectrum,
}

/// Resolve peaks / spectrum pane geometry for a lane.
pub fn pane_bounds_for_lane(
    representation: WaveformRepresentation,
    split: f32,
    lane_top: f32,
    lane_height: f32,
    splitter_px: f32,
) -> (Option<(f32, f32)>, Option<(f32, f32)>) {
    match representation {
        WaveformRepresentation::Peaks => (Some((lane_top, lane_height)), None),
        WaveformRepresentation::Spectrum => (None, Some((lane_top, lane_height))),
        WaveformRepresentation::PeaksSpectrum => {
            let peaks_h = (lane_height * split)
                .max(1.0)
                .min(lane_height - splitter_px - 1.0);
            let spec_top = lane_top + peaks_h + splitter_px;
            let spec_h = (lane_top + lane_height - spec_top).max(1.0);
            (Some((lane_top, peaks_h)), Some((spec_top, spec_h)))
        }
    }
}

/// Pick the pane under `y`, or `None` on the splitter gap.
pub fn pane_at_y(
    representation: WaveformRepresentation,
    split: f32,
    lane_top: f32,
    lane_height: f32,
    splitter_px: f32,
    y: f32,
) -> Option<(AxisPane, f32, f32)> {
    let (peaks, spectrum) =
        pane_bounds_for_lane(representation, split, lane_top, lane_height, splitter_px);
    if let Some((top, h)) = peaks {
        if y >= top && y <= top + h {
            return Some((AxisPane::Peaks, top, h));
        }
    }
    if let Some((top, h)) = spectrum {
        if y >= top && y <= top + h {
            return Some((AxisPane::Spectrum, top, h));
        }
    }
    None
}

/// Map pointer Y to a hover axis value for one lane.
pub fn hover_axis_at_y(
    representation: WaveformRepresentation,
    split: f32,
    sample_rate: u32,
    lane_top: f32,
    lane_height: f32,
    splitter_px: f32,
    y: f32,
) -> Option<WaveformHoverAxis> {
    let (pane, top, h) = pane_at_y(representation, split, lane_top, lane_height, splitter_px, y)?;
    match pane {
        AxisPane::Peaks => Some(WaveformHoverAxis::PeakDb(y_to_peak_db(y, top, h))),
        AxisPane::Spectrum => Some(WaveformHoverAxis::SpectrumHz(y_to_freq(
            y,
            sample_rate,
            top,
            h,
        ))),
    }
}

/// Stable key for hover-axis change detection (avoids notify spam).
pub fn hover_axis_quantize(axis: WaveformHoverAxis) -> u64 {
    match axis {
        WaveformHoverAxis::PeakDb(db) => {
            if db.is_infinite() && db.is_sign_negative() {
                0
            } else {
                // ~0.1 dB steps.
                let q = (db * 10.0).round() as i32;
                (1u64 << 32) | (q as u32 as u64)
            }
        }
        WaveformHoverAxis::SpectrumHz(hz) => {
            // ~1 Hz below 1k, ~10 Hz above.
            let q = if hz < 1000.0 {
                hz.round() as i32
            } else {
                (hz / 10.0).round() as i32 * 10
            };
            (2u64 << 32) | (q as u32 as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label_fits_in_visible(y: f32, visible_top: f32, visible_bottom: f32) -> bool {
        let half = label_half_height();
        let top = visible_top + half;
        let bottom = visible_bottom - half;
        bottom >= top && y >= top && y <= bottom
    }

    fn label_fits(
        y: f32,
        origin_y: f32,
        height: f32,
        visible_top: f32,
        visible_bottom: f32,
    ) -> bool {
        label_fits_in_pane(y, origin_y, height)
            && label_fits_in_visible(y, visible_top, visible_bottom)
    }

    fn peak_db_ticks(origin_y: f32, height: f32) -> Vec<AxisTick> {
        peak_ticks_for_layout(&peak_axis_layout(height), origin_y, height)
    }

    fn peak_db_ticks_in_view(
        origin_y: f32,
        height: f32,
        visible_top: f32,
        visible_bottom: f32,
    ) -> Vec<AxisTick> {
        if height < 1.0 {
            return Vec::new();
        }
        let mid_y = amplitude_to_y(0.0, origin_y, height);
        let mut ticks = Vec::new();
        if label_fits(mid_y, origin_y, height, visible_top, visible_bottom) {
            ticks.push(AxisTick {
                y: mid_y,
                label: format_db_tick(f32::NEG_INFINITY),
            });
        } else {
            return ticks;
        }

        let mut last_above_y = mid_y;
        for &db in PEAK_DB_CANDIDATES {
            let amp = db_to_amplitude(db);
            if amp <= AMP_DB_EPSILON {
                continue;
            }
            let y_above = amplitude_to_y(amp, origin_y, height);
            let y_below = amplitude_to_y(-amp, origin_y, height);
            if !label_fits(y_above, origin_y, height, visible_top, visible_bottom)
                || !label_fits(y_below, origin_y, height, visible_top, visible_bottom)
            {
                continue;
            }
            if (y_above - mid_y).abs() < PEAK_AXIS_LABEL_MIN_GAP_PX {
                continue;
            }
            if (last_above_y - y_above).abs() < PEAK_AXIS_LABEL_MIN_GAP_PX {
                continue;
            }
            let label = format_db_tick(db);
            ticks.push(AxisTick {
                y: y_above,
                label: label.clone(),
            });
            ticks.push(AxisTick { y: y_below, label });
            last_above_y = y_above;
        }

        ticks.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        ticks
    }

    fn spectrum_hz_ticks(sample_rate: u32, origin_y: f32, height: f32) -> Vec<AxisTick> {
        spectrum_ticks_for_layout(
            &spectrum_axis_layout(sample_rate, height),
            sample_rate,
            origin_y,
            height,
        )
    }

    #[test]
    fn amplitude_zero_is_neg_inf_db() {
        assert!(amplitude_to_db(0.0).is_infinite());
        assert!(amplitude_to_db(0.0).is_sign_negative());
        assert!(amplitude_to_db(1e-7).is_infinite());
    }

    #[test]
    fn amplitude_one_is_zero_db() {
        assert!((amplitude_to_db(1.0) - 0.0).abs() < 1e-5);
        assert!((amplitude_to_db(-1.0) - 0.0).abs() < 1e-5);
    }

    #[test]
    fn mid_y_is_neg_inf_db() {
        let origin = 100.0;
        let height = 200.0;
        let mid = amplitude_to_y(0.0, origin, height);
        assert!((mid - 200.0).abs() < 1e-3);
        let db = y_to_peak_db(mid, origin, height);
        assert!(db.is_infinite() && db.is_sign_negative());
    }

    #[test]
    fn top_and_bottom_are_zero_db() {
        let origin = 0.0;
        let height = 100.0;
        assert!((y_to_peak_db(origin, origin, height) - 0.0).abs() < 0.1);
        assert!((y_to_peak_db(origin + height, origin, height) - 0.0).abs() < 0.1);
    }

    #[test]
    fn db_amp_roundtrip() {
        for db in [0.0f32, -6.0, -12.0, -24.0] {
            let amp = db_to_amplitude(db);
            assert!((amplitude_to_db(amp) - db).abs() < 1e-4);
        }
    }

    #[test]
    fn freq_log_t_bounds() {
        let sr = 48_000u32;
        let (f_min, f_max) = spectral_freq_range(sr);
        assert!((freq_to_log_t(f_min, sr) - 0.0).abs() < 1e-5);
        assert!((freq_to_log_t(f_max, sr) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn one_khz_near_expected_log_position() {
        let sr = 48_000u32;
        let t = freq_to_log_t(1000.0, sr);
        let (f_min, f_max) = spectral_freq_range(sr);
        let expected = (1000.0f32.ln() - f_min.ln()) / (f_max.ln() - f_min.ln());
        assert!((t - expected).abs() < 1e-5);
        let y = freq_to_y(1000.0, sr, 0.0, 100.0);
        let back = y_to_freq(y, sr, 0.0, 100.0);
        assert!((back - 1000.0).abs() < 1.0);
    }

    #[test]
    fn format_hz_tick_styles() {
        assert_eq!(format_hz_tick(20.0), "20");
        assert_eq!(format_hz_tick(1000.0), "1k");
        assert_eq!(format_hz_tick(2500.0), "2.5k");
        assert_eq!(format_hz_tick(10_000.0), "10k");
    }

    #[test]
    fn format_db_and_hz_hover() {
        assert_eq!(format_db_hover(f32::NEG_INFINITY), "-∞ dB");
        assert_eq!(format_db_hover(-12.34), "-12.3 dB");
        assert_eq!(format_hz_hover(432.0), "432 Hz");
        assert_eq!(format_hz_hover(1230.0), "1.23kHz");
    }

    #[test]
    fn peak_ticks_never_overlap_neg_inf() {
        for height in [64.0f32, 96.0, 128.0, 200.0, 400.0] {
            let origin = 0.0;
            let ticks = peak_db_ticks(origin, height);
            let mid = amplitude_to_y(0.0, origin, height);
            assert!(ticks.iter().any(|t| t.label == "-∞"));
            for tick in &ticks {
                if tick.label == "-∞" {
                    continue;
                }
                assert!(
                    (tick.y - mid).abs() >= PEAK_AXIS_LABEL_MIN_GAP_PX - 1e-3,
                    "{} at y={} overlaps -∞ at mid={} (height={})",
                    tick.label,
                    tick.y,
                    mid,
                    height
                );
            }
            // Near-zero dB levels sit on the midline in linear amplitude.
            if height < 760.0 {
                assert!(
                    !ticks.iter().any(|t| t.label == "-24" || t.label == "-48"),
                    "-24/-48 must stay off small panes (height={height}): {:?}",
                    ticks.iter().map(|t| t.label.as_str()).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn peak_ticks_always_include_neg_inf_and_are_balanced() {
        for height in [24.0f32, 40.0, 80.0, 128.0, 200.0, 400.0] {
            let ticks = peak_db_ticks(10.0, height);
            let mid = amplitude_to_y(0.0, 10.0, height);
            let neg_inf: Vec<_> = ticks.iter().filter(|t| t.label == "-∞").collect();
            assert_eq!(neg_inf.len(), 1, "height={height}");
            assert!((neg_inf[0].y - mid).abs() < 1e-3);

            let mut counts = std::collections::HashMap::<&str, usize>::new();
            for tick in &ticks {
                if tick.label != "-∞" {
                    *counts.entry(tick.label.as_str()).or_default() += 1;
                }
            }
            for (label, n) in counts {
                assert_eq!(
                    n, 2,
                    "{label} must appear above and below (height={height})"
                );
            }
        }
    }

    #[test]
    fn ticks_do_not_clip_pane_edges() {
        let origin = 100.0f32;
        let height = 200.0f32;
        let half = AXIS_LABEL_FONT_PX * 0.5;
        for tick in peak_db_ticks(origin, height) {
            if tick.label == "-∞" {
                continue;
            }
            assert!(tick.y >= origin + half - 1e-3);
            assert!(tick.y <= origin + height - half + 1e-3);
        }
        for tick in spectrum_hz_ticks(48_000, origin, height) {
            assert!(tick.y >= origin + half - 1e-3);
            assert!(tick.y <= origin + height - half + 1e-3);
        }
    }

    #[test]
    fn peak_ticks_drop_pairs_when_viewport_clips_bottom() {
        let origin = 0.0f32;
        let height = 200.0f32;
        let mid = amplitude_to_y(0.0, origin, height);
        // Keep room for the centered -∞ glyph, but clip everything below it.
        let ticks = peak_db_ticks_in_view(origin, height, origin, mid + AXIS_LABEL_FONT_PX * 0.5);
        assert!(ticks.iter().any(|t| t.label == "-∞"));
        let finite: Vec<_> = ticks.iter().filter(|t| t.label != "-∞").collect();
        assert!(
            finite.is_empty(),
            "clipped bottom must not leave unpaired above-center ticks: {finite:?}"
        );
    }

    #[test]
    fn peak_ticks_balanced_when_viewport_partially_clips_lane() {
        let origin = 0.0f32;
        let height = 200.0f32;
        // Clip the bottom 40px of the lane (typical bottom-channel window clip).
        let ticks = peak_db_ticks_in_view(origin, height, origin, origin + height - 40.0);
        assert!(ticks.iter().any(|t| t.label == "-∞"));
        let mut counts = std::collections::HashMap::<&str, usize>::new();
        for tick in &ticks {
            if tick.label != "-∞" {
                *counts.entry(tick.label.as_str()).or_default() += 1;
            }
        }
        for (label, n) in counts {
            assert_eq!(n, 2, "{label} must stay balanced under viewport clip");
        }
    }

    #[test]
    fn outer_tick_is_neg_three_and_aligns_with_amplitude() {
        let ticks = peak_db_ticks(0.0, 200.0);
        assert!(
            ticks.iter().any(|t| t.label == "-3"),
            "expected -3 dB outer tick: {:?}",
            ticks.iter().map(|t| t.label.as_str()).collect::<Vec<_>>()
        );
        assert!(!ticks.iter().any(|t| t.label == "0"));
        let amp = db_to_amplitude(-3.0);
        let expected = amplitude_to_y(amp, 0.0, 200.0);
        let above = ticks
            .iter()
            .find(|t| t.label == "-3" && t.y < 100.0)
            .expect("above-mid -3");
        assert!((above.y - expected).abs() < 1e-3);
    }

    #[test]
    fn shared_layout_is_origin_independent() {
        let layout = peak_axis_layout(200.0);
        let a = peak_ticks_for_layout(&layout, 0.0, 200.0);
        let b = peak_ticks_for_layout(&layout, 500.0, 200.0);
        assert_eq!(a.len(), b.len());
        for (ta, tb) in a.iter().zip(b.iter()) {
            assert_eq!(ta.label, tb.label);
            assert!(((ta.y - 0.0) - (tb.y - 500.0)).abs() < 1e-3);
        }
    }

    #[test]
    fn spectrum_shared_layout_matches_across_lanes() {
        let layout = spectrum_axis_layout(48_000, 300.0);
        let a = spectrum_ticks_for_layout(&layout, 48_000, 0.0, 300.0);
        let b = spectrum_ticks_for_layout(&layout, 48_000, 400.0, 300.0);
        assert_eq!(
            a.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
            b.iter().map(|t| t.label.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn combined_pane_pick() {
        let split: f32 = 0.2;
        let top: f32 = 0.0;
        let h: f32 = 100.0;
        let splitter: f32 = 1.0;
        let peaks_h = (h * split).max(1.0).min(h - splitter - 1.0);
        let axis = hover_axis_at_y(
            WaveformRepresentation::PeaksSpectrum,
            split,
            48_000,
            top,
            h,
            splitter,
            peaks_h * 0.5,
        );
        assert!(matches!(axis, Some(WaveformHoverAxis::PeakDb(_))));
        let axis = hover_axis_at_y(
            WaveformRepresentation::PeaksSpectrum,
            split,
            48_000,
            top,
            h,
            splitter,
            peaks_h + splitter + 10.0,
        );
        assert!(matches!(axis, Some(WaveformHoverAxis::SpectrumHz(_))));
        let axis = hover_axis_at_y(
            WaveformRepresentation::PeaksSpectrum,
            split,
            48_000,
            top,
            h,
            splitter,
            peaks_h + 0.5,
        );
        assert!(axis.is_none());
    }
}
