// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use crate::model::document::BufferDocument;

pub fn collect_anchors(doc: &BufferDocument) -> Vec<usize> {
    let composition = doc.composition.read().unwrap();
    let mut anchors: Vec<usize> = composition
        .markers()
        .iter()
        .map(|marker| marker.frame as usize)
        .chain(doc.selection.regions.iter().flat_map(|r| [r.start, r.end]))
        .chain(
            composition
                .collections()
                .iter()
                .flat_map(|col| col.regions.iter().flat_map(|r| [r.start, r.end])),
        )
        .collect();
    anchors.sort_unstable();
    anchors.dedup();
    anchors
}

pub fn previous_anchor(anchors: &[usize], pos: usize) -> Option<usize> {
    anchors.iter().copied().filter(|a| *a < pos).max()
}

pub fn next_anchor(anchors: &[usize], pos: usize) -> Option<usize> {
    anchors.iter().copied().filter(|a| *a > pos).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::buffer::ChannelScope;
    use crate::model::composition::{Composition, MediaId, MediaRef, MARKER_TYPE_BLUE};
    use crate::model::document::BufferDocument;

    fn test_document() -> BufferDocument {
        let samples = vec![vec![0.0; 1000]];
        let media = MediaRef::from_memory(MediaId(0), 44100, samples);
        let mut doc = BufferDocument::new(Composition::from_media(media).unwrap());
        doc.add_marker(50, MARKER_TYPE_BLUE, None);
        doc.add_labeled_region(100, 200, ChannelScope::all(), None, "cues");
        doc
    }

    #[test]
    fn collects_markers_and_region_bounds() {
        let anchors = collect_anchors(&test_document());
        assert_eq!(anchors, vec![50, 100, 200]);
    }

    #[test]
    fn navigates_anchors() {
        let anchors = collect_anchors(&test_document());
        assert_eq!(previous_anchor(&anchors, 150), Some(100));
        assert_eq!(next_anchor(&anchors, 150), Some(200));
        assert_eq!(next_anchor(&anchors, 250), None);
    }
}
