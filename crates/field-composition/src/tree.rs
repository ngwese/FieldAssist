// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::sync::Arc;

use super::clip::{Clip, ClipId, ClipSpan};

const BALANCE_RATIO: usize = 4;

#[derive(Debug, Clone)]
enum Node {
    Leaf(Arc<Clip>),
    Branch {
        left: Arc<Node>,
        right: Arc<Node>,
        frames: u64,
        clips: usize,
    },
}

impl Node {
    fn frames(&self) -> u64 {
        match self {
            Node::Leaf(clip) => clip.len,
            Node::Branch { frames, .. } => *frames,
        }
    }

    fn clips(&self) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Branch { clips, .. } => *clips,
        }
    }
}

#[derive(Debug, Clone, Default)]
/// ClipTree.
pub struct ClipTree {
    root: Option<Arc<Node>>,
}

impl ClipTree {
    /// `empty`.
    pub fn empty() -> Self {
        Self { root: None }
    }

    /// `from_clip`.
    pub fn from_clip(clip: Clip) -> Self {
        if clip.len == 0 {
            Self::empty()
        } else {
            Self {
                root: Some(Arc::new(Node::Leaf(Arc::new(clip)))),
            }
        }
    }

    /// `from_clips`.
    pub fn from_clips<I>(clips: I) -> Self
    where
        I: IntoIterator<Item = Clip>,
    {
        let mut tree = Self::empty();
        for clip in clips {
            tree = Self::concat(tree, Self::from_clip(clip));
        }
        tree
    }

    /// `frames`.
    pub fn frames(&self) -> u64 {
        self.root.as_ref().map(|n| n.frames()).unwrap_or(0)
    }

    /// `clip_count`.
    pub fn clip_count(&self) -> usize {
        self.root.as_ref().map(|n| n.clips()).unwrap_or(0)
    }

    /// `is_empty`.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// `at`.
    pub fn at(&self, frame: u64) -> Option<ClipSpan> {
        let root = self.root.as_ref()?;
        if frame >= root.frames() {
            return None;
        }
        Some(at_node(root, frame, 0))
    }

    /// `spans`.
    pub fn spans(&self) -> Vec<ClipSpan> {
        let mut out = Vec::with_capacity(self.clip_count());
        if let Some(root) = &self.root {
            collect_spans(root, 0, &mut out);
        }
        out
    }

    /// `split`.
    pub fn split(&self, frame: u64, new_id: &mut dyn FnMut() -> ClipId) -> (Self, Self) {
        let Some(root) = &self.root else {
            return (Self::empty(), Self::empty());
        };
        let frame = frame.min(root.frames());
        let (left, right) = split_node(root, frame, new_id);
        (Self { root: left }, Self { root: right })
    }

    /// `concat`.
    pub fn concat(left: Self, right: Self) -> Self {
        Self {
            root: join(left.root, right.root),
        }
    }

    /// `replace_range`.
    pub fn replace_range(
        &self,
        start: u64,
        len: u64,
        middle: Self,
        new_id: &mut dyn FnMut() -> ClipId,
    ) -> Self {
        let start = start.min(self.frames());
        let len = len.min(self.frames().saturating_sub(start));
        let (prefix, rest) = self.split(start, new_id);
        let (_, suffix) = rest.split(len, new_id);
        Self::concat(Self::concat(prefix, middle), suffix)
    }

    /// `clips_in_range`.
    pub fn clips_in_range(
        &self,
        start: u64,
        len: u64,
        new_id: &mut dyn FnMut() -> ClipId,
    ) -> Vec<Clip> {
        let start = start.min(self.frames());
        let len = len.min(self.frames().saturating_sub(start));
        let (_, rest) = self.split(start, new_id);
        let (mid, _) = rest.split(len, new_id);
        mid.spans()
            .into_iter()
            .map(|span| (*span.clip).clone())
            .collect()
    }

    /// `map_clip_at`.
    pub fn map_clip_at(&self, frame: u64, map: impl FnOnce(&Clip) -> Clip) -> Self {
        let Some(root) = &self.root else {
            return Self::empty();
        };
        if frame >= root.frames() {
            return self.clone();
        }
        Self {
            root: Some(map_clip_at_node(root, frame, map)),
        }
    }

    /// `map_clips`.
    pub fn map_clips(&self, mut map: impl FnMut(&Clip) -> Clip) -> Self {
        let Some(root) = &self.root else {
            return Self::empty();
        };
        Self {
            root: Some(map_clips_node(root, &mut map)),
        }
    }
}

fn collect_spans(node: &Node, start: u64, out: &mut Vec<ClipSpan>) {
    match node {
        Node::Leaf(clip) => out.push(ClipSpan {
            start,
            clip: clip.clone(),
        }),
        Node::Branch { left, right, .. } => {
            collect_spans(left, start, out);
            collect_spans(right, start + left.frames(), out);
        }
    }
}

fn at_node(node: &Node, frame: u64, start: u64) -> ClipSpan {
    match node {
        Node::Leaf(clip) => ClipSpan {
            start,
            clip: clip.clone(),
        },
        Node::Branch { left, right, .. } => {
            let left_frames = left.frames();
            if frame < left_frames {
                at_node(left, frame, start)
            } else {
                at_node(right, frame - left_frames, start + left_frames)
            }
        }
    }
}

fn split_node(
    node: &Node,
    frame: u64,
    new_id: &mut dyn FnMut() -> ClipId,
) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
    match node {
        Node::Leaf(clip) => {
            if frame == 0 {
                (None, Some(Arc::new(Node::Leaf(clip.clone()))))
            } else if frame >= clip.len {
                (Some(Arc::new(Node::Leaf(clip.clone()))), None)
            } else {
                let (left, right) = clip.split(frame, new_id());
                (nonempty_leaf(left), nonempty_leaf(right))
            }
        }
        Node::Branch { left, right, .. } => {
            let left_frames = left.frames();
            if frame < left_frames {
                let (ll, lr) = split_node(left, frame, new_id);
                (ll, join(lr, Some(right.clone())))
            } else {
                let (rl, rr) = split_node(right, frame - left_frames, new_id);
                (join(Some(left.clone()), rl), rr)
            }
        }
    }
}

fn nonempty_leaf(clip: Clip) -> Option<Arc<Node>> {
    if clip.len == 0 {
        None
    } else {
        Some(Arc::new(Node::Leaf(Arc::new(clip))))
    }
}

fn join(left: Option<Arc<Node>>, right: Option<Arc<Node>>) -> Option<Arc<Node>> {
    match (left, right) {
        (None, r) => r,
        (l, None) => l,
        (Some(l), Some(r)) => Some(join_nodes(l, r)),
    }
}

fn join_nodes(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    let wl = left.clips();
    let wr = right.clips();
    if wl > wr.saturating_mul(BALANCE_RATIO) && wl > 1 {
        if let Node::Branch {
            left: ll,
            right: lr,
            ..
        } = &*left
        {
            return rotate(branch(ll.clone(), join_nodes(lr.clone(), right)));
        }
    }
    if wr > wl.saturating_mul(BALANCE_RATIO) && wr > 1 {
        if let Node::Branch {
            left: rl,
            right: rr,
            ..
        } = &*right
        {
            return rotate(branch(join_nodes(left, rl.clone()), rr.clone()));
        }
    }
    branch(left, right)
}

fn branch(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    Arc::new(Node::Branch {
        frames: left.frames() + right.frames(),
        clips: left.clips() + right.clips(),
        left,
        right,
    })
}

fn rotate(node: Arc<Node>) -> Arc<Node> {
    let Node::Branch { left, right, .. } = &*node else {
        return node;
    };
    let wl = left.clips();
    let wr = right.clips();
    if wl > wr.saturating_mul(BALANCE_RATIO) {
        if let Node::Branch {
            left: ll,
            right: lr,
            ..
        } = &**left
        {
            return branch(ll.clone(), branch(lr.clone(), right.clone()));
        }
    }
    if wr > wl.saturating_mul(BALANCE_RATIO) {
        if let Node::Branch {
            left: rl,
            right: rr,
            ..
        } = &**right
        {
            return branch(branch(left.clone(), rl.clone()), rr.clone());
        }
    }
    node
}

fn map_clip_at_node(node: &Node, frame: u64, map: impl FnOnce(&Clip) -> Clip) -> Arc<Node> {
    match node {
        Node::Leaf(clip) => Arc::new(Node::Leaf(Arc::new(map(clip.as_ref())))),
        Node::Branch { left, right, .. } => {
            let left_frames = left.frames();
            if frame < left_frames {
                branch(map_clip_at_node(left, frame, map), right.clone())
            } else {
                branch(
                    left.clone(),
                    map_clip_at_node(right, frame - left_frames, map),
                )
            }
        }
    }
}

fn map_clips_node(node: &Node, map: &mut impl FnMut(&Clip) -> Clip) -> Arc<Node> {
    match node {
        Node::Leaf(clip) => Arc::new(Node::Leaf(Arc::new(map(clip.as_ref())))),
        Node::Branch { left, right, .. } => {
            branch(map_clips_node(left, map), map_clips_node(right, map))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field_audio_model::MediaId;

    fn leaf(id: u64, len: u64) -> Clip {
        Clip::from_media(ClipId(id), MediaId(1), id * 10, len)
    }

    fn ids() -> impl FnMut() -> ClipId {
        let mut n = 100u64;
        move || {
            n += 1;
            ClipId(n)
        }
    }

    #[test]
    fn at_and_spans_cover_timeline() {
        let tree = ClipTree::from_clips([leaf(1, 10), leaf(2, 5), leaf(3, 20)]);
        assert_eq!(tree.frames(), 35);
        assert_eq!(tree.clip_count(), 3);
        assert_eq!(tree.at(0).unwrap().clip.id, ClipId(1));
        assert_eq!(tree.at(10).unwrap().clip.id, ClipId(2));
        assert_eq!(tree.at(15).unwrap().clip.id, ClipId(3));
        assert!(tree.at(35).is_none());
        let spans = tree.spans();
        assert_eq!(
            spans.iter().map(|s| s.start).collect::<Vec<_>>(),
            vec![0, 10, 15]
        );
    }

    #[test]
    fn split_and_concat_round_trip() {
        let tree = ClipTree::from_clips([leaf(1, 10), leaf(2, 10), leaf(3, 10)]);
        let mut new_id = ids();
        let (left, right) = tree.split(15, &mut new_id);
        assert_eq!(left.frames(), 15);
        assert_eq!(right.frames(), 15);
        let joined = ClipTree::concat(left, right);
        assert_eq!(joined.frames(), 30);
        assert_eq!(joined.clip_count(), 4);
    }

    #[test]
    fn replace_range_inserts_and_removes() {
        let tree = ClipTree::from_clips([leaf(1, 10), leaf(2, 10)]);
        let mut new_id = ids();
        let removed = tree.replace_range(5, 10, ClipTree::empty(), &mut new_id);
        assert_eq!(removed.frames(), 10);
        let inserted = tree.replace_range(10, 0, ClipTree::from_clip(leaf(9, 3)), &mut new_id);
        assert_eq!(inserted.frames(), 23);
        assert_eq!(inserted.at(10).unwrap().clip.id, ClipId(9));
    }

    #[test]
    fn concat_many_clips_stays_balanced_enough_to_query() {
        let clips: Vec<_> = (0..64).map(|i| leaf(i, 3)).collect();
        let tree = ClipTree::from_clips(clips);
        assert_eq!(tree.frames(), 192);
        assert_eq!(tree.clip_count(), 64);
        assert_eq!(tree.at(0).unwrap().clip.id, ClipId(0));
        assert_eq!(tree.at(191).unwrap().clip.id, ClipId(63));
    }
}
