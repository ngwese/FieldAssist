// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! EditOp → edit-card DTO mappers (panel lives in field-ui-components).

use field_ui_components::{EditCard, EditsData};

use crate::model::composition::EditOp;
use crate::model::document::BufferDocument;

/// Short title for an edit operation card.
pub fn edit_title(op: &EditOp) -> &'static str {
    match op {
        EditOp::Init => "Initial",
        EditOp::Cut { .. } => "Cut",
        EditOp::Copy { .. } => "Copy",
        EditOp::Paste { .. } => "Paste",
        EditOp::Remove { .. } => "Remove",
        EditOp::Delete { .. } => "Delete",
        EditOp::Trim { .. } => "Trim",
        EditOp::Move { .. } => "Move",
        EditOp::Duplicate { .. } => "Duplicate",
        EditOp::Roll { .. } => "Roll",
    }
}

/// Detail line for an edit operation card.
pub fn edit_detail(op: &EditOp, sample_rate: u32) -> String {
    match op {
        EditOp::Init => "start of composition".into(),
        EditOp::Cut { start, len }
        | EditOp::Copy { start, len }
        | EditOp::Remove { start, len }
        | EditOp::Delete { start, len }
        | EditOp::Trim { start, len }
        | EditOp::Duplicate { start, len } => {
            format!(
                "{} · {}",
                format_stamp(*start, sample_rate),
                format_len(*len)
            )
        }
        EditOp::Paste { at, len } => {
            format!(
                "at {} · {}",
                format_stamp(*at, sample_rate),
                format_len(*len)
            )
        }
        EditOp::Move { from, len, dest } => format!(
            "{} → {} · {}",
            format_stamp(*from, sample_rate),
            format_stamp(*dest, sample_rate),
            format_len(*len)
        ),
        EditOp::Roll { at, delta } => {
            format!("at {} · {delta:+} smp", format_stamp(*at, sample_rate))
        }
    }
}

fn format_stamp(frame: u64, sample_rate: u32) -> String {
    let secs = frame as f64 / f64::from(sample_rate.max(1));
    format!("{secs:.2}s")
}

fn format_len(len: u64) -> String {
    format!("{len} smp")
}

impl EditsData for BufferDocument {
    fn fingerprint(&self) -> u64 {
        let composition = self.composition.read().unwrap();
        composition.current_edit().0
            ^ ((composition.edits().len() as u64) << 32)
            ^ ((composition.undo_floor() as u64) << 48)
    }

    fn snapshot(&self) -> Vec<EditCard> {
        let composition = self.composition.read().unwrap();
        let sample_rate = composition.sample_rate();
        let current = composition.current_edit();
        let floor = composition.undo_floor();
        composition
            .edits()
            .iter()
            .enumerate()
            .rev()
            .map(|(index, edit)| EditCard {
                id: edit.id.0,
                title: edit_title(&edit.op).to_string(),
                detail: edit_detail(&edit.op, sample_rate),
                is_current: edit.id == current,
                is_future: edit.id.0 > current.0,
                // Newest-first: separator sits above the first founding card.
                separator_before: floor > 0 && index == floor,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_two_line_edit_cards() {
        assert_eq!(edit_title(&EditOp::Init), "Initial");
        assert_eq!(edit_detail(&EditOp::Init, 44100), "start of composition");
        assert_eq!(
            edit_detail(
                &EditOp::Cut {
                    start: 44100,
                    len: 22050
                },
                44100
            ),
            "1.00s · 22050 smp"
        );
        assert_eq!(
            edit_detail(
                &EditOp::Move {
                    from: 0,
                    len: 100,
                    dest: 44100
                },
                44100
            ),
            "0.00s → 1.00s · 100 smp"
        );
        assert_eq!(
            edit_detail(
                &EditOp::Roll {
                    at: 4410,
                    delta: -12
                },
                44100
            ),
            "at 0.10s · -12 smp"
        );
    }
}
