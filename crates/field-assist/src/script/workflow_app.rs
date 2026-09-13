// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use super::workflow::WorkflowMeta;

pub const SCOPE_DRAG_DROP: &str = "drag-drop";
pub const SCOPE_MENU: &str = "menu";

#[derive(Clone, Debug, PartialEq)]
pub struct DropLayout {
    pub rows: Vec<DropRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropRow {
    pub cells: Vec<DropCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropCell {
    pub name: String,
    pub display_name: String,
    pub color: [f32; 4],
    pub weight: f64,
}

/// Collect drag-drop workflows into rows. Missing row numbers are not padded.
/// Within a row, higher `priority` is first (left); ties sort by `display_name`.
/// Cell `weight` is `priority / sum(priorities in the row)`.
pub fn layout_drop_targets<'a>(
    workflows: impl IntoIterator<Item = &'a WorkflowMeta>,
    scope: &str,
) -> DropLayout {
    let mut grouped: std::collections::BTreeMap<i64, Vec<&WorkflowMeta>> =
        std::collections::BTreeMap::new();
    for workflow in workflows {
        if !workflow.scopes.iter().any(|name| name == scope) {
            continue;
        }
        grouped.entry(workflow.row).or_default().push(workflow);
    }
    let mut rows = Vec::new();
    for mut group in grouped.into_values() {
        group.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.name.cmp(&b.name))
        });
        let sum: f64 = group.iter().map(|workflow| workflow.priority).sum();
        let cells = group
            .into_iter()
            .map(|workflow| DropCell {
                name: workflow.name.clone(),
                display_name: workflow.display_name.clone(),
                color: workflow.color,
                weight: if sum > 0.0 {
                    workflow.priority / sum
                } else {
                    0.0
                },
            })
            .collect();
        rows.push(DropRow { cells });
    }
    DropLayout { rows }
}

/// Workflows bound to the `"menu"` scope, sorted by `display_name` then `name`.
pub fn workflows_for_menu(workflows: &[WorkflowMeta]) -> Vec<&WorkflowMeta> {
    let mut items: Vec<&WorkflowMeta> = workflows
        .iter()
        .filter(|workflow| workflow.scopes.iter().any(|scope| scope == SCOPE_MENU))
        .collect();
    items.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}

#[cfg(test)]
mod tests {
    use super::super::workflow::DEFAULT_WORKFLOW_COLOR;
    use super::*;

    fn meta(name: &str, display: &str, row: i64, priority: f64, scopes: &[&str]) -> WorkflowMeta {
        WorkflowMeta {
            name: name.into(),
            display_name: display.into(),
            description: String::new(),
            scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
            row,
            priority,
            color: DEFAULT_WORKFLOW_COLOR,
        }
    }

    #[test]
    fn layout_groups_rows_without_padding() {
        let workflows = [
            meta("a", "A", 1, 1.0, &[SCOPE_DRAG_DROP]),
            meta("c", "C", 3, 1.0, &[SCOPE_DRAG_DROP]),
            meta("other", "Other", 2, 1.0, &["later"]),
        ];
        let layout = layout_drop_targets(&workflows, SCOPE_DRAG_DROP);
        assert_eq!(layout.rows.len(), 2);
        assert_eq!(layout.rows[0].cells[0].name, "a");
        assert_eq!(layout.rows[1].cells[0].name, "c");
    }

    #[test]
    fn layout_sorts_priority_then_display_name_and_weights() {
        let workflows = [
            meta("low_b", "Beta", 1, 1.0, &[SCOPE_DRAG_DROP]),
            meta("high", "Wide", 1, 2.0, &[SCOPE_DRAG_DROP]),
            meta("low_a", "Alpha", 1, 1.0, &[SCOPE_DRAG_DROP]),
        ];
        let layout = layout_drop_targets(&workflows, SCOPE_DRAG_DROP);
        let names: Vec<_> = layout.rows[0]
            .cells
            .iter()
            .map(|cell| cell.name.as_str())
            .collect();
        assert_eq!(names, ["high", "low_a", "low_b"]);
        let weights: Vec<_> = layout.rows[0]
            .cells
            .iter()
            .map(|cell| (cell.weight * 100.0).round() as i32)
            .collect();
        assert_eq!(weights, [50, 25, 25]);
    }

    #[test]
    fn menu_workflows_sorted_by_display_name() {
        let workflows = [
            meta("zeta", "Zebra", 1, 1.0, &[SCOPE_MENU]),
            meta("drop", "Drop Only", 1, 1.0, &[SCOPE_DRAG_DROP]),
            meta("alpha", "Apple", 1, 1.0, &[SCOPE_DRAG_DROP, SCOPE_MENU]),
            meta("same_b", "Same", 1, 1.0, &[SCOPE_MENU]),
            meta("same_a", "Same", 1, 1.0, &[SCOPE_MENU]),
        ];
        let names: Vec<_> = workflows_for_menu(&workflows)
            .into_iter()
            .map(|workflow| workflow.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "same_a", "same_b", "zeta"]);
    }
}
