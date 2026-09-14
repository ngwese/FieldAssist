// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Rebuild parent→child composition trees from open composition UUIDs.

use std::collections::{HashMap, HashSet};

use crate::model::composition::CompositionId;
use crate::model::DocumentId;

/// One open document's composition identity and optional parent link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineageNode {
    /// Session document handle.
    pub document: DocumentId,
    /// Composition UUID from the `.facomp` / runtime composition.
    pub composition: CompositionId,
    /// Parent composition UUID when broken out.
    pub parent: Option<CompositionId>,
}

/// Session-local parent map derived from open composition UUIDs.
#[derive(Debug, Clone, Default)]
pub struct LineageTree {
    /// Child document → parent document when the parent is open.
    parent_of: HashMap<DocumentId, DocumentId>,
}

impl LineageTree {
    /// Build a tree from open documents. Duplicate composition ids keep the
    /// first match; cycles are ignored (treated as roots).
    pub fn from_nodes(nodes: &[LineageNode]) -> Self {
        let mut by_composition = HashMap::new();
        for node in nodes {
            by_composition
                .entry(node.composition)
                .or_insert(node.document);
        }

        let mut parent_of = HashMap::new();
        for node in nodes {
            let Some(parent_comp) = node.parent else {
                continue;
            };
            let Some(&parent_doc) = by_composition.get(&parent_comp) else {
                continue;
            };
            if parent_doc == node.document {
                continue;
            }
            if would_cycle(&parent_of, node.document, parent_doc) {
                continue;
            }
            parent_of.insert(node.document, parent_doc);
        }

        Self { parent_of }
    }

    /// Parent document when the parent composition is open in this session.
    pub fn parent(&self, id: DocumentId) -> Option<DocumentId> {
        self.parent_of.get(&id).copied()
    }

    /// Children of `id` among `ordered` (session order preserved).
    pub fn children<'a>(
        &self,
        id: DocumentId,
        ordered: impl IntoIterator<Item = &'a DocumentId>,
    ) -> Vec<DocumentId> {
        ordered
            .into_iter()
            .copied()
            .filter(|child| self.parent(*child) == Some(id))
            .collect()
    }

    /// Depth from the nearest open root (0 = root / orphan).
    pub fn depth(&self, id: DocumentId) -> usize {
        let mut depth = 0;
        let mut seen = HashSet::new();
        let mut current = id;
        while let Some(parent) = self.parent(current) {
            if !seen.insert(current) {
                break;
            }
            depth += 1;
            current = parent;
        }
        depth
    }

    /// Walk open ancestors starting at `id` (inclusive).
    pub fn ancestors(&self, id: DocumentId) -> Vec<DocumentId> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut current = id;
        loop {
            if !seen.insert(current) {
                break;
            }
            out.push(current);
            match self.parent(current) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        out
    }

    /// True when `id` is a lineage descendant of `ancestor` (not equal).
    pub fn is_descendant(&self, id: DocumentId, ancestor: DocumentId) -> bool {
        if id == ancestor {
            return false;
        }
        let mut current = id;
        let mut seen = HashSet::new();
        while let Some(parent) = self.parent(current) {
            if !seen.insert(current) {
                break;
            }
            if parent == ancestor {
                return true;
            }
            current = parent;
        }
        false
    }

    /// Documents in `ordered` that sit in a contiguous descendant block
    /// immediately after their open ancestor (session / visual order).
    pub fn attached_in_order(&self, ordered: &[DocumentId]) -> HashSet<DocumentId> {
        let mut attached = HashSet::new();
        for (i, &id) in ordered.iter().enumerate() {
            let mut j = i + 1;
            while j < ordered.len() && self.is_descendant(ordered[j], id) {
                attached.insert(ordered[j]);
                j += 1;
            }
        }
        attached
    }

    /// Contiguous attached descendants of `parent` in `ordered`.
    pub fn attached_children_of(
        &self,
        parent: DocumentId,
        ordered: &[DocumentId],
    ) -> Vec<DocumentId> {
        let Some(i) = ordered.iter().position(|&id| id == parent) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut j = i + 1;
        while j < ordered.len() && self.is_descendant(ordered[j], parent) {
            out.push(ordered[j]);
            j += 1;
        }
        out
    }

    /// Indent / disclosure / link flags for explorer rows in `ordered`
    /// (one session group, session order).
    pub fn explorer_flags(
        &self,
        ordered: &[DocumentId],
    ) -> HashMap<DocumentId, ExplorerLineageFlags> {
        let attached = self.attached_in_order(ordered);
        ordered
            .iter()
            .copied()
            .map(|id| {
                let parent = self.parent(id);
                (
                    id,
                    ExplorerLineageFlags {
                        depth: self.depth(id),
                        parent,
                        detached: parent.is_some() && !attached.contains(&id),
                        has_children: !self.attached_children_of(id, ordered).is_empty(),
                    },
                )
            })
            .collect()
    }
}

/// Explorer presentation derived from open composition lineage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplorerLineageFlags {
    /// Indent depth from the nearest open root.
    pub depth: usize,
    /// Open parent document when the parent composition is in the session.
    pub parent: Option<DocumentId>,
    /// Parent is open but this row is not in the contiguous block under it.
    pub detached: bool,
    /// Has contiguous attached descendants that can be disclosed.
    pub has_children: bool,
}

fn would_cycle(
    parent_of: &HashMap<DocumentId, DocumentId>,
    child: DocumentId,
    parent: DocumentId,
) -> bool {
    let mut current = parent;
    let mut seen = HashSet::from([child]);
    loop {
        if !seen.insert(current) {
            return true;
        }
        match parent_of.get(&current) {
            Some(&next) => current = next,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn doc(n: u128) -> DocumentId {
        DocumentId::from_u128(n)
    }

    fn comp(n: u128) -> CompositionId {
        CompositionId(Uuid::from_u128(n))
    }

    #[test]
    fn parent_and_child_relink() {
        let a = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let b = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let tree = LineageTree::from_nodes(&[a, b]);
        assert_eq!(tree.parent(doc(2)), Some(doc(1)));
        assert_eq!(tree.children(doc(1), &[doc(1), doc(2)]), vec![doc(2)]);
        assert_eq!(tree.depth(doc(2)), 1);
    }

    #[test]
    fn child_alone_is_root() {
        let b = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let tree = LineageTree::from_nodes(&[b]);
        assert_eq!(tree.parent(doc(2)), None);
        assert_eq!(tree.depth(doc(2)), 0);
    }

    #[test]
    fn adding_parent_later_relinks() {
        let child = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        assert!(LineageTree::from_nodes(&[child]).parent(doc(2)).is_none());
        let parent = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let tree = LineageTree::from_nodes(&[child, parent]);
        assert_eq!(tree.parent(doc(2)), Some(doc(1)));
    }

    #[test]
    fn cycles_ignored() {
        let a = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: Some(comp(20)),
        };
        let b = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let tree = LineageTree::from_nodes(&[a, b]);
        // First edge a→b may stick; b→a would cycle and is rejected.
        assert_eq!(tree.parent(doc(1)), Some(doc(2)));
        assert_eq!(tree.parent(doc(2)), None);
    }

    #[test]
    fn attached_contiguous_block_marks_nested_not_detached() {
        let root = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let child = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let other = LineageNode {
            document: doc(3),
            composition: comp(30),
            parent: None,
        };
        let detached = LineageNode {
            document: doc(4),
            composition: comp(40),
            parent: Some(comp(10)),
        };
        let tree = LineageTree::from_nodes(&[root, child, other, detached]);
        let ordered = [doc(1), doc(2), doc(3), doc(4)];
        let attached = tree.attached_in_order(&ordered);
        assert!(attached.contains(&doc(2)));
        assert!(!attached.contains(&doc(4)));
        assert!(tree.is_descendant(doc(4), doc(1)));
    }

    #[test]
    fn explorer_missing_parent_treats_child_as_root() {
        // Child's .facomp still names a parent UUID, but that composition is
        // not open in the session.
        let child = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let sibling = LineageNode {
            document: doc(3),
            composition: comp(30),
            parent: None,
        };
        let tree = LineageTree::from_nodes(&[child, sibling]);
        let ordered = [doc(2), doc(3)];
        let flags = tree.explorer_flags(&ordered);
        let child_flags = flags[&doc(2)];
        assert_eq!(child_flags.parent, None);
        assert_eq!(child_flags.depth, 0);
        assert!(!child_flags.detached);
        assert!(!child_flags.has_children);
        assert!(!flags[&doc(3)].has_children);
    }

    #[test]
    fn explorer_missing_child_parent_has_no_disclosure() {
        // Parent is open; its break-out child is not in the session.
        let parent = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let unrelated = LineageNode {
            document: doc(3),
            composition: comp(30),
            parent: None,
        };
        let tree = LineageTree::from_nodes(&[parent, unrelated]);
        let ordered = [doc(1), doc(3)];
        let flags = tree.explorer_flags(&ordered);
        assert!(!flags[&doc(1)].has_children);
        assert_eq!(flags[&doc(1)].depth, 0);
        assert!(tree.children(doc(1), &ordered).is_empty());
        assert!(tree.attached_children_of(doc(1), &ordered).is_empty());
    }

    #[test]
    fn explorer_nested_break_out_chain_when_all_open() {
        // Root → child → grandchild, all open and contiguous in session order.
        let root = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let child = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let grand = LineageNode {
            document: doc(3),
            composition: comp(30),
            parent: Some(comp(20)),
        };
        let tree = LineageTree::from_nodes(&[root, child, grand]);
        let ordered = [doc(1), doc(2), doc(3)];
        let flags = tree.explorer_flags(&ordered);

        assert_eq!(flags[&doc(1)].parent, None);
        assert_eq!(flags[&doc(1)].depth, 0);
        assert!(flags[&doc(1)].has_children);
        assert!(!flags[&doc(1)].detached);

        assert_eq!(flags[&doc(2)].parent, Some(doc(1)));
        assert_eq!(flags[&doc(2)].depth, 1);
        assert!(flags[&doc(2)].has_children);
        assert!(!flags[&doc(2)].detached);

        assert_eq!(flags[&doc(3)].parent, Some(doc(2)));
        assert_eq!(flags[&doc(3)].depth, 2);
        assert!(!flags[&doc(3)].has_children);
        assert!(!flags[&doc(3)].detached);

        assert_eq!(
            tree.attached_children_of(doc(1), &ordered),
            vec![doc(2), doc(3)]
        );
        assert_eq!(tree.attached_children_of(doc(2), &ordered), vec![doc(3)]);
    }

    #[test]
    fn explorer_nested_break_out_with_missing_middle() {
        // Grandchild and root are open; the middle child composition is not.
        // Grand's parent UUID points at the missing child, so it is an explorer
        // root even though its .facomp still records a parent.
        let root = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let grand = LineageNode {
            document: doc(3),
            composition: comp(30),
            parent: Some(comp(20)),
        };
        let tree = LineageTree::from_nodes(&[root, grand]);
        let ordered = [doc(1), doc(3)];
        let flags = tree.explorer_flags(&ordered);

        assert!(!flags[&doc(1)].has_children);
        assert_eq!(flags[&doc(3)].parent, None);
        assert_eq!(flags[&doc(3)].depth, 0);
        assert!(!flags[&doc(3)].detached);
        assert!(!tree.is_descendant(doc(3), doc(1)));
    }

    #[test]
    fn explorer_nested_break_out_detached_grandchild() {
        let root = LineageNode {
            document: doc(1),
            composition: comp(10),
            parent: None,
        };
        let child = LineageNode {
            document: doc(2),
            composition: comp(20),
            parent: Some(comp(10)),
        };
        let other = LineageNode {
            document: doc(4),
            composition: comp(40),
            parent: None,
        };
        let grand = LineageNode {
            document: doc(3),
            composition: comp(30),
            parent: Some(comp(20)),
        };
        let tree = LineageTree::from_nodes(&[root, child, other, grand]);
        let ordered = [doc(1), doc(2), doc(4), doc(3)];
        let flags = tree.explorer_flags(&ordered);

        assert!(flags[&doc(1)].has_children);
        assert!(!flags[&doc(2)].detached);
        assert!(!flags[&doc(2)].has_children);
        assert_eq!(flags[&doc(3)].parent, Some(doc(2)));
        assert_eq!(flags[&doc(3)].depth, 2);
        assert!(flags[&doc(3)].detached);
    }
}
