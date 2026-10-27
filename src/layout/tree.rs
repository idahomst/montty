//! Binary split tree describing how panes tile a tab.

use super::geom::{DIVIDER_HIT_PX, DIVIDER_PX, Direction, MIN_PANE_PX, Orientation, PaneId, Rect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DividerId(pub Vec<u8>);

pub struct Divider {
    pub id: DividerId,
    pub orientation: Orientation,
    pub line: Rect,
    pub hit: Rect,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RemoveResult {
    NotFound,
    Removed,
    Empty,
}

#[derive(Clone, Debug)]
enum Node {
    Leaf(PaneId),
    Split {
        orientation: Orientation,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

#[derive(Clone, Debug)]
pub struct SplitTree {
    root: Node,
    maximized: Option<PaneId>,
    /// Smallest pane (width, height) in px; never below `MIN_PANE_PX`.
    min: (i32, i32),
}

/// Rewrite leaf `a` as `b` and `b` as `a`.
fn swap_leaves(n: &mut Node, a: PaneId, b: PaneId) {
    match n {
        Node::Leaf(id) if *id == a => *id = b,
        Node::Leaf(id) if *id == b => *id = a,
        Node::Leaf(_) => {}
        Node::Split { a: l, b: r, .. } => {
            swap_leaves(l, a, b);
            swap_leaves(r, a, b);
        }
    }
}

/// Extent of `r` along the split axis.
fn extent(o: Orientation, r: Rect) -> i32 {
    match o {
        Orientation::Horizontal => r.w,
        Orientation::Vertical => r.h,
    }
}

/// The per-pane minimum (width, height) along `axis`.
fn axis_min(min: (i32, i32), axis: Orientation) -> i32 {
    match axis {
        Orientation::Horizontal => min.0,
        Orientation::Vertical => min.1,
    }
}

/// Minimum extent of a subtree along `axis`, given the per-pane minimum `min`.
fn min_extent(n: &Node, axis: Orientation, min: (i32, i32)) -> i32 {
    match n {
        Node::Leaf(_) => axis_min(min, axis),
        Node::Split {
            orientation, a, b, ..
        } => {
            let (ma, mb) = (min_extent(a, axis, min), min_extent(b, axis, min));
            if *orientation == axis {
                ma + DIVIDER_PX + mb
            } else {
                ma.max(mb)
            }
        }
    }
}

/// Size of the first child along the axis for total extent `e`, given each
/// child's minimum extent.
fn first_size(e: i32, ratio: f32, min_a: i32, min_b: i32) -> i32 {
    let avail = (e - DIVIDER_PX).max(0);
    let raw = (avail as f32 * ratio).round() as i32;
    let (lo, hi) = (min_a, avail - min_b);
    if lo > hi {
        // Too small for both minimums: share proportionally to the minimums.
        avail * min_a / (min_a + min_b)
    } else {
        raw.clamp(lo, hi)
    }
}

/// Split `r` into (first, divider line, second) for a split node's parts.
fn split_rects(
    o: Orientation,
    ratio: f32,
    (a, b): (&Node, &Node),
    r: Rect,
    min: (i32, i32),
) -> (Rect, Rect, Rect) {
    let first = first_size(
        extent(o, r),
        ratio,
        min_extent(a, o, min),
        min_extent(b, o, min),
    );
    let second = (extent(o, r) - DIVIDER_PX - first).max(0);
    match o {
        Orientation::Horizontal => (
            Rect { w: first, ..r },
            Rect {
                x: r.x + first,
                w: DIVIDER_PX,
                ..r
            },
            Rect {
                x: r.x + first + DIVIDER_PX,
                w: second,
                ..r
            },
        ),
        Orientation::Vertical => (
            Rect { h: first, ..r },
            Rect {
                y: r.y + first,
                h: DIVIDER_PX,
                ..r
            },
            Rect {
                y: r.y + first + DIVIDER_PX,
                h: second,
                ..r
            },
        ),
    }
}

fn hit_rect(o: Orientation, line: Rect, node: Rect) -> Rect {
    let (x0, y0, x1, y1) = match o {
        Orientation::Horizontal => (
            line.x - DIVIDER_HIT_PX,
            line.y,
            line.x + line.w + DIVIDER_HIT_PX,
            line.y + line.h,
        ),
        Orientation::Vertical => (
            line.x,
            line.y - DIVIDER_HIT_PX,
            line.x + line.w,
            line.y + line.h + DIVIDER_HIT_PX,
        ),
    };
    let x0 = x0.max(node.x);
    let y0 = y0.max(node.y);
    let x1 = x1.min(node.x + node.w);
    let y1 = y1.min(node.y + node.h);
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0),
        h: (y1 - y0).max(0),
    }
}

fn collect_layout(n: &Node, r: Rect, min: (i32, i32), out: &mut Vec<(PaneId, Rect)>) {
    match n {
        Node::Leaf(id) => out.push((*id, r)),
        Node::Split {
            orientation,
            ratio,
            a,
            b,
        } => {
            let (ra, _, rb) = split_rects(*orientation, *ratio, (a, b), r, min);
            collect_layout(a, ra, min, out);
            collect_layout(b, rb, min, out);
        }
    }
}

fn collect_dividers(
    n: &Node,
    r: Rect,
    min: (i32, i32),
    path: &mut Vec<u8>,
    out: &mut Vec<Divider>,
) {
    if let Node::Split {
        orientation,
        ratio,
        a,
        b,
    } = n
    {
        let (ra, line, rb) = split_rects(*orientation, *ratio, (a, b), r, min);
        out.push(Divider {
            id: DividerId(path.clone()),
            orientation: *orientation,
            line,
            hit: hit_rect(*orientation, line, r),
        });
        path.push(0);
        collect_dividers(a, ra, min, path, out);
        path.pop();
        path.push(1);
        collect_dividers(b, rb, min, path, out);
        path.pop();
    }
}

fn collect_panes(n: &Node, out: &mut Vec<PaneId>) {
    match n {
        Node::Leaf(id) => out.push(*id),
        Node::Split { a, b, .. } => {
            collect_panes(a, out);
            collect_panes(b, out);
        }
    }
}

/// Path (0 = first, 1 = second) from `n` to the leaf `pane`.
fn path_to(n: &Node, pane: PaneId, path: &mut Vec<u8>) -> bool {
    match n {
        Node::Leaf(id) => *id == pane,
        Node::Split { a, b, .. } => {
            path.push(0);
            if path_to(a, pane, path) {
                return true;
            }
            path.pop();
            path.push(1);
            if path_to(b, pane, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}

/// Replace leaf `target` with a split holding it and `new`; `new` comes first when `before`.
fn split_leaf(n: &mut Node, target: PaneId, o: Orientation, new: PaneId, before: bool) -> bool {
    match n {
        Node::Leaf(id) if *id == target => {
            let (a, b) = if before { (new, target) } else { (target, new) };
            *n = Node::Split {
                orientation: o,
                ratio: 0.5,
                a: Box::new(Node::Leaf(a)),
                b: Box::new(Node::Leaf(b)),
            };
            true
        }
        Node::Leaf(_) => false,
        Node::Split { a, b, .. } => {
            split_leaf(a, target, o, new, before) || split_leaf(b, target, o, new, before)
        }
    }
}

/// Remove leaf `pane` from a split tree whose root is a `Split`.
fn remove_in(n: &mut Node, pane: PaneId) -> bool {
    let Node::Split { a, b, .. } = n else {
        return false;
    };
    let promote = if matches!(**a, Node::Leaf(id) if id == pane) {
        Some(true)
    } else if matches!(**b, Node::Leaf(id) if id == pane) {
        Some(false)
    } else {
        None
    };
    match promote {
        Some(drop_a) => {
            let placeholder = Node::Leaf(0);
            let Node::Split { a, b, .. } = std::mem::replace(n, placeholder) else {
                unreachable!()
            };
            *n = if drop_a { *b } else { *a };
            true
        }
        None => remove_in(a, pane) || remove_in(b, pane),
    }
}

fn node_at<'a>(mut n: &'a Node, path: &[u8]) -> Option<&'a Node> {
    for &step in path {
        match n {
            Node::Split { a, b, .. } => n = if step == 0 { a } else { b },
            Node::Leaf(_) => return None,
        }
    }
    Some(n)
}

/// Give every part of a run of `o` splits the same share; returns how many parts the
/// run lines up (a pane or an other-axis subtree is one part).
fn equalize_run(n: &mut Node, o: Orientation) -> u32 {
    match n {
        Node::Split {
            orientation,
            ratio,
            a,
            b,
        } if *orientation == o => {
            let (pa, pb) = (equalize_run(a, o), equalize_run(b, o));
            *ratio = pa as f32 / (pa + pb) as f32;
            pa + pb
        }
        _ => 1,
    }
}

fn node_at_mut<'a>(mut n: &'a mut Node, path: &[u8]) -> Option<&'a mut Node> {
    for &step in path {
        match n {
            Node::Split { a, b, .. } => n = if step == 0 { a } else { b },
            Node::Leaf(_) => return None,
        }
    }
    Some(n)
}

/// A tree's structure with arbitrary leaf payloads: what a saved session stores and
/// what `SplitTree` is rebuilt from.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape<L> {
    Leaf(L),
    Split {
        orientation: Orientation,
        ratio: f32,
        a: Box<Shape<L>>,
        b: Box<Shape<L>>,
    },
}

impl<L> Shape<L> {
    pub fn map<M>(self, f: &mut impl FnMut(L) -> M) -> Shape<M> {
        match self {
            Shape::Leaf(l) => Shape::Leaf(f(l)),
            Shape::Split {
                orientation,
                ratio,
                a,
                b,
            } => {
                let a = Box::new(a.map(f));
                let b = Box::new(b.map(f));
                Shape::Split {
                    orientation,
                    ratio,
                    a,
                    b,
                }
            }
        }
    }

    /// The leaves, depth-first, `a` before `b`.
    pub fn leaves(&self) -> Vec<&L> {
        match self {
            Shape::Leaf(l) => vec![l],
            Shape::Split { a, b, .. } => {
                let mut v = a.leaves();
                v.extend(b.leaves());
                v
            }
        }
    }
}

fn node_shape(n: &Node) -> Shape<PaneId> {
    match n {
        Node::Leaf(id) => Shape::Leaf(*id),
        Node::Split {
            orientation,
            ratio,
            a,
            b,
        } => Shape::Split {
            orientation: *orientation,
            ratio: *ratio,
            a: Box::new(node_shape(a)),
            b: Box::new(node_shape(b)),
        },
    }
}

fn shape_node(s: Shape<PaneId>) -> Node {
    match s {
        Shape::Leaf(id) => Node::Leaf(id),
        Shape::Split {
            orientation,
            ratio,
            a,
            b,
        } => Node::Split {
            orientation,
            ratio: ratio.clamp(0.0, 1.0),
            a: Box::new(shape_node(*a)),
            b: Box::new(shape_node(*b)),
        },
    }
}

impl SplitTree {
    pub fn new(root: PaneId) -> Self {
        Self {
            root: Node::Leaf(root),
            maximized: None,
            min: (MIN_PANE_PX, MIN_PANE_PX),
        }
    }

    pub fn shape(&self) -> Shape<PaneId> {
        node_shape(&self.root)
    }

    /// Not maximized, default minimum extent; ratios clamped to 0.0..=1.0.
    pub fn from_shape(s: Shape<PaneId>) -> Self {
        Self {
            root: shape_node(s),
            maximized: None,
            min: (MIN_PANE_PX, MIN_PANE_PX),
        }
    }

    /// Smallest pane size (e.g. one cell plus padding); `MIN_PANE_PX` is the floor on each axis.
    pub fn set_min_extent(&mut self, w: i32, h: i32) {
        self.min = (w.max(MIN_PANE_PX), h.max(MIN_PANE_PX));
    }

    /// The smallest pane size in force (`set_min_extent`, at least `MIN_PANE_PX`).
    pub fn min_extent(&self) -> (i32, i32) {
        self.min
    }

    pub fn split(&mut self, target: PaneId, o: Orientation, new: PaneId) -> bool {
        self.split_with(target, o, new, false)
    }

    /// Like `split`, but `new` becomes the first child (left or top).
    pub fn split_before(&mut self, target: PaneId, o: Orientation, new: PaneId) -> bool {
        self.split_with(target, o, new, true)
    }

    fn split_with(&mut self, target: PaneId, o: Orientation, new: PaneId, before: bool) -> bool {
        let ok = split_leaf(&mut self.root, target, o, new, before);
        if ok {
            self.maximized = None;
        }
        ok
    }

    pub fn can_split(&self, target: PaneId, o: Orientation, area: Rect) -> bool {
        // Simulate the split and require both halves to hold the minimum extent, on the
        // other axis too (the new pane's minimum can exceed the room the target has there).
        const PROBE: PaneId = PaneId::MAX;
        let mut t = self.clone();
        t.maximized = None;
        if target == PROBE || !t.split(target, o, PROBE) {
            return false;
        }
        let rects = t.full_layout(area);
        [target, PROBE].iter().all(|id| {
            rects.iter().find(|(p, _)| p == id).is_some_and(|(_, r)| {
                [Orientation::Horizontal, Orientation::Vertical]
                    .iter()
                    .all(|&axis| extent(axis, *r) >= axis_min(self.min, axis))
            })
        })
    }

    pub fn remove(&mut self, pane: PaneId) -> RemoveResult {
        if let Node::Leaf(id) = self.root {
            return if id == pane {
                RemoveResult::Empty
            } else {
                RemoveResult::NotFound
            };
        }
        if !remove_in(&mut self.root, pane) {
            return RemoveResult::NotFound;
        }
        if self.maximized == Some(pane) {
            self.maximized = None;
        }
        RemoveResult::Removed
    }

    /// Trade the slots of `a` and `b`: each takes the other's place and size.
    /// False, and nothing changes, when `a == b` or either is missing.
    pub fn swap(&mut self, a: PaneId, b: PaneId) -> bool {
        let panes = self.panes();
        if a == b || !panes.contains(&a) || !panes.contains(&b) {
            return false;
        }
        swap_leaves(&mut self.root, a, b);
        true
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        collect_panes(&self.root, &mut out);
        out
    }

    fn full_layout(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        collect_layout(&self.root, area, self.min, &mut out);
        out
    }

    pub fn layout(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        match self.maximized {
            Some(id) => vec![(id, area)],
            None => self.full_layout(area),
        }
    }

    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        if self.maximized.is_none() {
            collect_dividers(&self.root, area, self.min, &mut Vec::new(), &mut out);
        }
        out
    }

    pub fn neighbor(&self, from: PaneId, dir: Direction, area: Rect) -> Option<PaneId> {
        let rects = self.full_layout(area);
        let r = rects.iter().find(|(id, _)| *id == from)?.1;
        let overlap = |a0: i32, a1: i32, b0: i32, b1: i32| (a1.min(b1) - a0.max(b0)).max(0);
        let mut best: Option<(i32, i32, PaneId)> = None; // (overlap, tiebreak coord, id)
        for &(id, c) in &rects {
            let (adjacent, ov, tie) = match dir {
                Direction::Right => (
                    c.x == r.x + r.w + DIVIDER_PX,
                    overlap(r.y, r.y + r.h, c.y, c.y + c.h),
                    c.y,
                ),
                Direction::Left => (
                    c.x + c.w + DIVIDER_PX == r.x,
                    overlap(r.y, r.y + r.h, c.y, c.y + c.h),
                    c.y,
                ),
                Direction::Down => (
                    c.y == r.y + r.h + DIVIDER_PX,
                    overlap(r.x, r.x + r.w, c.x, c.x + c.w),
                    c.x,
                ),
                Direction::Up => (
                    c.y + c.h + DIVIDER_PX == r.y,
                    overlap(r.x, r.x + r.w, c.x, c.x + c.w),
                    c.x,
                ),
            };
            if !adjacent || ov == 0 {
                continue;
            }
            if best.is_none_or(|(bo, bt, _)| ov > bo || (ov == bo && tie < bt)) {
                best = Some((ov, tie, id));
            }
        }
        best.map(|(_, _, id)| id)
    }

    /// Move the divider `id` so its line sits at absolute pixel `pos_px`
    /// (x for Horizontal splits, y for Vertical), clamped to the minimum pane extent.
    pub fn set_divider(&mut self, id: &DividerId, pos_px: i32, area: Rect) {
        // Find the node's own rect by descending from the root.
        let mut rect = area;
        let mut n = &self.root;
        for &step in &id.0 {
            let Node::Split {
                orientation,
                ratio,
                a,
                b,
            } = n
            else {
                return;
            };
            let (ra, _, rb) = split_rects(*orientation, *ratio, (a, b), rect, self.min);
            (n, rect) = if step == 0 { (&**a, ra) } else { (&**b, rb) };
        }
        let Node::Split { orientation, .. } = n else {
            return;
        };
        let o = *orientation;
        let start = match o {
            Orientation::Horizontal => rect.x,
            Orientation::Vertical => rect.y,
        };
        let avail = (extent(o, rect) - DIVIDER_PX).max(1);
        let ratio = (pos_px - start) as f32 / avail as f32;
        if let Some(Node::Split { ratio: r, .. }) = node_at_mut(&mut self.root, &id.0) {
            *r = ratio.clamp(0.0, 1.0);
        }
    }

    /// Size the panes along divider `id`'s axis equally: those of the whole run of
    /// same-axis splits it belongs to (three columns get a third each). The other axis
    /// keeps its sizes; an unknown id does nothing.
    pub fn equalize(&mut self, id: &DividerId) {
        let Some(Node::Split { orientation, .. }) = node_at(&self.root, &id.0) else {
            return;
        };
        let o = *orientation;
        let same_axis = |path: &[u8]| matches!(node_at(&self.root, path), Some(Node::Split { orientation, .. }) if *orientation == o);
        let mut top = id.0.len();
        while top > 0 && same_axis(&id.0[..top - 1]) {
            top -= 1;
        }
        if let Some(n) = node_at_mut(&mut self.root, &id.0[..top]) {
            equalize_run(n, o);
        }
    }

    pub fn resize(&mut self, pane: PaneId, dir: Direction, step_px: i32, area: Rect) -> bool {
        let (want, sign) = match dir {
            Direction::Left => (Orientation::Horizontal, -1),
            Direction::Right => (Orientation::Horizontal, 1),
            Direction::Up => (Orientation::Vertical, -1),
            Direction::Down => (Orientation::Vertical, 1),
        };
        let mut path = Vec::new();
        if !path_to(&self.root, pane, &mut path) {
            return false;
        }
        // Nearest ancestor (longest proper prefix) with the matching orientation.
        for len in (0..path.len()).rev() {
            let id = DividerId(path[..len].to_vec());
            let Some(div) = self.dividers_all(area).into_iter().find(|d| d.id == id) else {
                continue;
            };
            if div.orientation != want {
                continue;
            }
            let cur = match want {
                Orientation::Horizontal => div.line.x,
                Orientation::Vertical => div.line.y,
            };
            self.set_divider(&id, cur + sign * step_px, area);
            return true;
        }
        false
    }

    /// Dividers ignoring maximize state.
    fn dividers_all(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        collect_dividers(&self.root, area, self.min, &mut Vec::new(), &mut out);
        out
    }

    pub fn toggle_maximize(&mut self, pane: PaneId) {
        self.maximized = if self.maximized == Some(pane) {
            None
        } else if self.panes().contains(&pane) {
            Some(pane)
        } else {
            self.maximized
        };
    }

    pub fn maximized(&self) -> Option<PaneId> {
        self.maximized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 201,
        h: 101,
    };

    fn widths(t: &SplitTree, ids: &[PaneId]) -> Vec<i32> {
        let l = t.layout(AREA);
        ids.iter()
            .map(|id| l.iter().find(|(p, _)| p == id).unwrap().1.w)
            .collect()
    }

    /// 1 | 2 | (3 over 4): three columns, the last one split into rows.
    fn three_columns() -> SplitTree {
        let mut t = SplitTree::new(1);
        assert!(t.split(1, Orientation::Horizontal, 2));
        assert!(t.split(2, Orientation::Horizontal, 3));
        assert!(t.split(3, Orientation::Vertical, 4));
        t
    }

    #[test]
    fn equalize_evens_out_the_whole_run_of_columns() {
        // Either column divider evens out all three columns: 200 px less two
        // dividers in thirds, within a pixel.
        for id in [vec![], vec![1]] {
            let mut t = three_columns();
            t.set_divider(&DividerId(vec![]), 20, AREA);
            t.set_divider(&DividerId(vec![1]), 180, AREA);
            t.equalize(&DividerId(id));
            assert_eq!(widths(&t, &[1, 2, 3]), [67, 66, 66]);
        }
    }

    #[test]
    fn equalize_leaves_the_other_axis_alone() {
        let mut t = three_columns();
        t.set_divider(&DividerId(vec![]), 40, AREA);
        let rows = DividerId(vec![1, 1]);
        t.set_divider(&rows, 40, AREA);
        t.equalize(&rows);
        // 3 and 4 share the height; the columns keep their widths.
        let l = t.layout(AREA);
        let h = |id| l.iter().find(|(p, _)| *p == id).unwrap().1.h;
        assert_eq!((h(3), h(4)), (50, 50));
        assert_eq!(widths(&t, &[1])[0], 40);
        // Evening out the columns doesn't touch the rows either.
        t.set_divider(&rows, 40, AREA);
        t.equalize(&DividerId(vec![]));
        let l = t.layout(AREA);
        assert_eq!(l.iter().find(|(p, _)| *p == 3).unwrap().1.h, 40);
    }

    #[test]
    fn equalize_ignores_unknown_ids() {
        let mut t = three_columns();
        t.set_divider(&DividerId(vec![]), 40, AREA);
        // A leaf, a path past the tree, a single pane: nothing changes.
        t.equalize(&DividerId(vec![0]));
        t.equalize(&DividerId(vec![1, 1, 1, 1]));
        assert_eq!(widths(&t, &[1])[0], 40);
        SplitTree::new(1).equalize(&DividerId(vec![]));
    }

    #[test]
    fn split_before_puts_new_pane_first() {
        let mut t = SplitTree::new(1);
        assert!(t.split_before(1, Orientation::Horizontal, 2));
        assert_eq!(t.panes(), vec![2, 1]);
        let l = t.layout(AREA);
        let r = |id| l.iter().find(|(p, _)| *p == id).unwrap().1;
        assert!(r(2).x < r(1).x);
        assert_eq!(r(2).y, r(1).y);

        let mut t = SplitTree::new(1);
        assert!(t.split_before(1, Orientation::Vertical, 2));
        assert_eq!(t.panes(), vec![2, 1]);
        let l = t.layout(AREA);
        let r = |id| l.iter().find(|(p, _)| *p == id).unwrap().1;
        assert!(r(2).y < r(1).y);
        assert_eq!(r(2).x, r(1).x);

        assert!(!t.split_before(9, Orientation::Horizontal, 3));
    }

    #[test]
    fn single_pane_fills_area() {
        assert_eq!(SplitTree::new(1).layout(AREA), vec![(1, AREA)]);
    }
    #[test]
    fn split_right_two_panes_1px_divider() {
        let mut t = SplitTree::new(1);
        assert!(t.split(1, Orientation::Horizontal, 2));
        assert_eq!(
            t.layout(AREA),
            vec![
                (
                    1,
                    Rect {
                        x: 0,
                        y: 0,
                        w: 100,
                        h: 101
                    }
                ),
                (
                    2,
                    Rect {
                        x: 101,
                        y: 0,
                        w: 100,
                        h: 101
                    }
                )
            ]
        );
        let d = t.dividers(AREA);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].line,
            Rect {
                x: 100,
                y: 0,
                w: 1,
                h: 101
            }
        );
        assert_eq!(
            d[0].hit,
            Rect {
                x: 97,
                y: 0,
                w: 7,
                h: 101
            }
        );
    }
    #[test]
    fn split_down_two_panes() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Vertical, 2);
        assert_eq!(
            t.layout(AREA),
            vec![
                (
                    1,
                    Rect {
                        x: 0,
                        y: 0,
                        w: 201,
                        h: 50
                    }
                ),
                (
                    2,
                    Rect {
                        x: 0,
                        y: 51,
                        w: 201,
                        h: 50
                    }
                )
            ]
        );
    }
    #[test]
    fn panes_depth_first_order() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.split(1, Orientation::Vertical, 3);
        assert_eq!(t.panes(), vec![1, 3, 2]);
    }
    #[test]
    fn remove_promotes_sibling() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        assert_eq!(t.remove(1), RemoveResult::Removed);
        assert_eq!(t.layout(AREA), vec![(2, AREA)]);
    }
    #[test]
    fn remove_last_is_empty() {
        assert_eq!(SplitTree::new(1).remove(1), RemoveResult::Empty);
    }
    #[test]
    fn remove_unknown_not_found() {
        assert_eq!(SplitTree::new(1).remove(9), RemoveResult::NotFound);
    }
    #[test]
    fn neighbor_directions() {
        // 1 | 2 on top row, 3 below spanning the full width
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Vertical, 3);
        t.split(1, Orientation::Horizontal, 2);
        assert_eq!(t.neighbor(1, Direction::Right, AREA), Some(2));
        assert_eq!(t.neighbor(2, Direction::Left, AREA), Some(1));
        assert_eq!(t.neighbor(1, Direction::Down, AREA), Some(3));
        assert_eq!(t.neighbor(3, Direction::Up, AREA), Some(1)); // greatest overlap, tie -> leftmost
        assert_eq!(t.neighbor(1, Direction::Up, AREA), None);
    }
    #[test]
    fn resize_moves_nearest_matching_divider() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        assert!(t.resize(2, Direction::Right, 10, AREA)); // moves the divider right even from the right pane
        assert_eq!(t.layout(AREA)[0].1.w, 110);
        assert!(!t.resize(2, Direction::Down, 10, AREA)); // no vertical ancestor
    }
    #[test]
    fn ratio_clamped_to_min_pane() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.set_divider(&DividerId(vec![]), 5, AREA);
        assert_eq!(t.layout(AREA)[0].1.w, MIN_PANE_PX);
    }
    #[test]
    fn split_refused_when_too_small() {
        let t = SplitTree::new(1);
        assert!(!t.can_split(
            1,
            Orientation::Horizontal,
            Rect {
                x: 0,
                y: 0,
                w: 2 * MIN_PANE_PX,
                h: 100
            }
        ));
        assert!(t.can_split(
            1,
            Orientation::Horizontal,
            Rect {
                x: 0,
                y: 0,
                w: 2 * MIN_PANE_PX + 1,
                h: 100
            }
        ));
    }
    #[test]
    fn maximize_shows_one_pane_no_dividers() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.toggle_maximize(2);
        assert_eq!(t.layout(AREA), vec![(2, AREA)]);
        assert!(t.dividers(AREA).is_empty());
        t.toggle_maximize(2);
        assert_eq!(t.layout(AREA).len(), 2);
    }
    #[test]
    fn removing_maximized_pane_clears_maximize() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.toggle_maximize(2);
        t.remove(2);
        assert_eq!(t.maximized(), None);
        assert_eq!(t.layout(AREA), vec![(1, AREA)]);
    }
    #[test]
    fn nested_drag_respects_subtree_min() {
        let area = Rect {
            x: 0,
            y: 0,
            w: 401,
            h: 101,
        };
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.split(1, Orientation::Horizontal, 3);
        t.set_divider(&DividerId(vec![]), 5, area);
        for (_, r) in t.layout(area) {
            assert!(r.w >= MIN_PANE_PX, "{r:?}");
        }
    }
    #[test]
    fn nested_resize_respects_subtree_min() {
        let area = Rect {
            x: 0,
            y: 0,
            w: 401,
            h: 101,
        };
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.split(1, Orientation::Horizontal, 3);
        for _ in 0..100 {
            t.resize(2, Direction::Left, 10, area);
        }
        for (_, r) in t.layout(area) {
            assert!(r.w >= MIN_PANE_PX, "{r:?}");
        }
    }
    /// A tree whose panes must hold one 28x40 cell plus 4px padding on each side.
    fn cell_tree() -> SplitTree {
        let mut t = SplitTree::new(1);
        t.set_min_extent(36, 48);
        t
    }

    fn r(w: i32, h: i32) -> Rect {
        Rect { x: 0, y: 0, w, h }
    }

    #[test]
    fn split_refused_below_per_axis_min() {
        let t = cell_tree();
        assert!(!t.can_split(1, Orientation::Horizontal, r(2 * 36, 500)));
        assert!(t.can_split(1, Orientation::Horizontal, r(2 * 36 + 1, 500)));
        assert!(!t.can_split(1, Orientation::Vertical, r(500, 2 * 48)));
        assert!(t.can_split(1, Orientation::Vertical, r(500, 2 * 48 + 1)));
    }

    #[test]
    fn min_extent_never_below_floor() {
        let mut t = SplitTree::new(1);
        t.set_min_extent(10, 5);
        assert!(!t.can_split(1, Orientation::Horizontal, r(2 * MIN_PANE_PX, 500)));
        assert!(!t.can_split(1, Orientation::Vertical, r(500, 2 * MIN_PANE_PX)));
        assert!(t.can_split(1, Orientation::Vertical, r(500, 2 * MIN_PANE_PX + 1)));
    }

    #[test]
    fn drags_and_resizes_keep_per_axis_min() {
        let area = r(401, 301);
        let mut t = cell_tree();
        // [ (1 over 3) | (2 | 4) ]
        assert!(t.split(1, Orientation::Horizontal, 2));
        assert!(t.split(1, Orientation::Vertical, 3));
        assert!(t.split(2, Orientation::Horizontal, 4));
        let check = |t: &SplitTree| {
            for (id, rect) in t.layout(area) {
                assert!(rect.w >= 36 && rect.h >= 48, "pane {id}: {rect:?}");
            }
        };
        t.set_divider(&DividerId(vec![]), 5, area);
        check(&t);
        t.set_divider(&DividerId(vec![]), 400, area);
        check(&t);
        t.set_divider(&DividerId(vec![0]), 2, area);
        check(&t);
        t.set_divider(&DividerId(vec![0]), 300, area);
        check(&t);
        for dir in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            for id in 1..=4 {
                for _ in 0..50 {
                    t.resize(id, dir, 20, area);
                }
                check(&t);
            }
        }
    }

    #[test]
    fn zero_size_area_does_not_panic() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        let z = Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
        assert_eq!(t.layout(z).len(), 2);
        let _ = t.dividers(z);
        t.set_divider(&DividerId(vec![]), 3, z);
        t.resize(1, Direction::Right, 5, z);
    }

    #[test]
    fn swap_trades_slots_and_keeps_shape() {
        let mut t = three_columns();
        t.set_divider(&DividerId(vec![]), 40, AREA);
        t.set_divider(&DividerId(vec![1]), 120, AREA);
        t.set_divider(&DividerId(vec![1, 1]), 30, AREA);
        let rect = |t: &SplitTree, id: PaneId| {
            t.layout(AREA)
                .into_iter()
                .find(|&(p, _)| p == id)
                .unwrap()
                .1
        };
        let before: Vec<Rect> = (1..=4).map(|id| rect(&t, id)).collect();
        let lines = |t: &SplitTree| {
            t.dividers(AREA)
                .into_iter()
                .map(|d| d.line)
                .collect::<Vec<_>>()
        };
        let dividers = lines(&t);
        let shape = t.shape();
        assert_ne!(before[0], before[3]);
        assert!(t.swap(1, 4));
        assert_eq!(rect(&t, 4), before[0]);
        assert_eq!(rect(&t, 1), before[3]);
        assert_eq!(rect(&t, 2), before[1]);
        assert_eq!(rect(&t, 3), before[2]);
        assert_eq!(lines(&t), dividers);
        assert!(!t.swap(1, 1));
        assert!(!t.swap(1, 99));
        assert!(!t.swap(99, 1));
        let mut back = t.clone();
        assert!(back.swap(1, 4));
        assert_eq!(back.shape(), shape);
    }

    #[test]
    fn swap_leaves_maximized_alone() {
        let mut t = three_columns();
        t.toggle_maximize(2);
        assert!(t.swap(2, 3));
        assert_eq!(t.maximized(), Some(2));
    }

    #[test]
    fn shape_round_trip() {
        let mut t = SplitTree::new(1);
        assert!(t.split(1, Orientation::Horizontal, 2));
        assert!(t.split(2, Orientation::Vertical, 3));
        t.set_divider(&DividerId(vec![]), 60, AREA);
        t.set_divider(&DividerId(vec![1]), 70, AREA);
        let back = SplitTree::from_shape(t.shape());
        assert_eq!(back.layout(AREA), t.layout(AREA));
        assert_eq!(back.shape(), t.shape());
    }

    #[test]
    fn shape_leaves_depth_first() {
        let t = three_columns();
        let s = t.shape();
        assert_eq!(s.leaves(), [&1, &2, &3, &4]);
        let m = s.map(&mut |id| id * 10);
        assert_eq!(m.leaves(), [&10, &20, &30, &40]);
    }
}
