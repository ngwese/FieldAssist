// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Pure policy for when to start, upgrade, defer, or ignore analysis jobs.

use field_audio_process::AnalysisKind;

/// Labels that share [`crate::progress::ProgressHandle`] with analysis but must
/// not be cancelled by an analysis spawn.
const PROTECTED_PROGRESS_LABELS: &[&str] = &["opening", "exporting"];

/// What [`crate::app::AppView::spawn_analysis_pass`] should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisSpawnDecision {
    /// No job owns progress; begin a new pass.
    Start,
    /// An equivalent (or covering) analysis job is already running.
    NoOp,
    /// A single-kind (or smaller) analysis job is running; cancel it and start
    /// the requested superset shared pass.
    Upgrade,
    /// Progress is busy with an unrelated job; re-queue and try later.
    Defer,
}

/// Decide how to handle a filtered analysis request against the current job.
///
/// `progress_label` is the active [`ProgressState`](field_core::ProgressState)
/// label when progress is owned. `running_kinds` is the in-flight analysis
/// kinds tracked on the document (absent for open/export).
pub fn resolve_analysis_spawn(
    progress_label: Option<&str>,
    running_kinds: Option<&[AnalysisKind]>,
    requested: &[AnalysisKind],
) -> AnalysisSpawnDecision {
    if requested.is_empty() {
        return AnalysisSpawnDecision::NoOp;
    }
    if progress_label.is_some_and(|label| PROTECTED_PROGRESS_LABELS.contains(&label)) {
        return AnalysisSpawnDecision::Defer;
    }
    match running_kinds {
        None => {
            if progress_label.is_some() {
                // Progress owned without tracked analysis kinds (unexpected for
                // analysis; treat as busy rather than cancelling blindly).
                AnalysisSpawnDecision::Defer
            } else {
                AnalysisSpawnDecision::Start
            }
        }
        Some(running) if running.is_empty() => AnalysisSpawnDecision::Start,
        Some(running) => {
            if requested.iter().all(|kind| running.contains(kind)) {
                AnalysisSpawnDecision::NoOp
            } else if running.iter().all(|kind| requested.contains(kind))
                && requested.len() > running.len()
            {
                AnalysisSpawnDecision::Upgrade
            } else {
                AnalysisSpawnDecision::Defer
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field_core::ProgressHandle;

    #[test]
    fn idle_starts() {
        assert_eq!(
            resolve_analysis_spawn(None, None, &[AnalysisKind::MinMax, AnalysisKind::Spectral]),
            AnalysisSpawnDecision::Start
        );
    }

    #[test]
    fn duplicate_shared_request_is_noop() {
        let running = [AnalysisKind::MinMax, AnalysisKind::Spectral];
        assert_eq!(
            resolve_analysis_spawn(
                Some("building peaks + spectrum"),
                Some(&running),
                &[AnalysisKind::MinMax, AnalysisKind::Spectral]
            ),
            AnalysisSpawnDecision::NoOp
        );
    }

    #[test]
    fn subset_request_is_noop() {
        let running = [AnalysisKind::MinMax, AnalysisKind::Spectral];
        assert_eq!(
            resolve_analysis_spawn(
                Some("building peaks + spectrum"),
                Some(&running),
                &[AnalysisKind::Spectral]
            ),
            AnalysisSpawnDecision::NoOp
        );
    }

    #[test]
    fn true_upgrade_cancels_once() {
        let running = [AnalysisKind::MinMax];
        assert_eq!(
            resolve_analysis_spawn(
                Some("building peaks"),
                Some(&running),
                &[AnalysisKind::MinMax, AnalysisKind::Spectral]
            ),
            AnalysisSpawnDecision::Upgrade
        );
    }

    #[test]
    fn unrelated_analysis_defers() {
        let running = [AnalysisKind::EnvelopePeak];
        assert_eq!(
            resolve_analysis_spawn(
                Some("building envelope"),
                Some(&running),
                &[AnalysisKind::MinMax, AnalysisKind::Spectral]
            ),
            AnalysisSpawnDecision::Defer
        );
    }

    #[test]
    fn opening_and_exporting_are_protected() {
        assert_eq!(
            resolve_analysis_spawn(Some("opening"), None, &[AnalysisKind::Spectral]),
            AnalysisSpawnDecision::Defer
        );
        assert_eq!(
            resolve_analysis_spawn(Some("exporting"), None, &[AnalysisKind::MinMax]),
            AnalysisSpawnDecision::Defer
        );
    }

    #[test]
    fn noop_keeps_progress_epoch_and_fraction() {
        let progress = ProgressHandle::new();
        let epoch = progress.begin("building peaks + spectrum");
        progress.set_fraction(epoch, 0.42);
        let running = [AnalysisKind::MinMax, AnalysisKind::Spectral];
        assert_eq!(
            resolve_analysis_spawn(
                Some("building peaks + spectrum"),
                Some(&running),
                &[AnalysisKind::MinMax, AnalysisKind::Spectral]
            ),
            AnalysisSpawnDecision::NoOp
        );
        // Callers must not cancel/begin on NoOp — fraction stays put.
        assert!(progress.is_epoch(epoch));
        let snap = progress.snapshot().unwrap();
        assert_eq!(snap.percent(), 42);
    }
}
