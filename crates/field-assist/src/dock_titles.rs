// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Dock tab titles owned by the FieldAssist application.

/// Detail-dock Markers panel tab.
pub const DETAIL_TAB_MARKER: &str = "Marker";
/// Detail-dock Regions panel tab.
pub const DETAIL_TAB_REGIONS: &str = "Regions";
/// Detail-dock History (edits) panel tab.
pub const DETAIL_TAB_HISTORY: &str = "History";
/// Detail-dock Monitor panel tab.
pub const DETAIL_TAB_MONITOR: &str = "Monitor";

/// Titles used to measure the right detail dock minimum width.
pub const DETAIL_DOCK_TAB_TITLES: &[&str] = &[
    DETAIL_TAB_MARKER,
    DETAIL_TAB_REGIONS,
    DETAIL_TAB_HISTORY,
    DETAIL_TAB_MONITOR,
];

/// Explorer-dock Compositions panel tab.
pub const EXPLORER_TAB_COMPOSITIONS: &str = "Compositions";

/// Titles used to measure the left explorer dock minimum width.
pub const EXPLORER_DOCK_TAB_TITLES: &[&str] = &[EXPLORER_TAB_COMPOSITIONS];
