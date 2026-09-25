// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Aggregated problems from a single open (session or composition) operation.

use crate::Location;

/// Category for a load problem shown in the load-problems sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProblemCategory {
    /// Referenced file or media could not be found.
    MissingReference,
    /// File exists but does not match the recorded descriptor.
    MediaMismatch,
    /// Document JSON / envelope could not be parsed.
    CorruptDocument,
    /// Format version is unsupported (too old or too new).
    UnsupportedFormat,
    /// URL could not be parsed or relative URL lacked a document base.
    InvalidLocation,
    /// Catch-all for other open failures.
    Other,
}

impl ProblemCategory {
    /// Stable display title for UI group boxes.
    pub fn title(self) -> &'static str {
        match self {
            Self::MissingReference => "Missing references",
            Self::MediaMismatch => "Media mismatches",
            Self::CorruptDocument => "Corrupt documents",
            Self::UnsupportedFormat => "Unsupported format",
            Self::InvalidLocation => "Invalid locations",
            Self::Other => "Other problems",
        }
    }
}

/// One problem encountered while opening a session or composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadProblem {
    /// Problem category for grouping in the UI.
    pub category: ProblemCategory,
    /// Short subject (basename, document name, …).
    pub subject: String,
    /// Human-readable detail.
    pub detail: String,
    /// Related URL when applicable.
    pub url: Option<Location>,
}

impl LoadProblem {
    /// Construct a problem row.
    pub fn new(
        category: ProblemCategory,
        subject: impl Into<String>,
        detail: impl Into<String>,
        url: Option<Location>,
    ) -> Self {
        Self {
            category,
            subject: subject.into(),
            detail: detail.into(),
            url,
        }
    }
}

/// Aggregated result of opening one session or composition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenReport {
    /// File the user asked to open.
    pub target: Location,
    /// Problems discovered during the open (may be empty on success).
    pub problems: Vec<LoadProblem>,
}

impl OpenReport {
    /// Empty report for `target`.
    pub fn new(target: Location) -> Self {
        Self {
            target,
            problems: Vec::new(),
        }
    }

    /// True when no problems were recorded.
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }

    /// Append a problem.
    pub fn push(&mut self, problem: LoadProblem) {
        self.problems.push(problem);
    }

    /// Append all problems from `other` (does not change `self.target`).
    pub fn extend_from(&mut self, other: OpenReport) {
        self.problems.extend(other.problems);
    }

    /// Problems grouped by category in display order (empty categories omitted).
    pub fn grouped(&self) -> Vec<(ProblemCategory, Vec<&LoadProblem>)> {
        const ORDER: [ProblemCategory; 6] = [
            ProblemCategory::MissingReference,
            ProblemCategory::MediaMismatch,
            ProblemCategory::CorruptDocument,
            ProblemCategory::UnsupportedFormat,
            ProblemCategory::InvalidLocation,
            ProblemCategory::Other,
        ];
        let mut out = Vec::new();
        for category in ORDER {
            let items: Vec<&LoadProblem> = self
                .problems
                .iter()
                .filter(|p| p.category == category)
                .collect();
            if !items.is_empty() {
                out.push((category, items));
            }
        }
        out
    }
}
