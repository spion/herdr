//! BSP tree layout for tiling panes within a workspace.
//!
//! [`TileLayout`] arranges panes in a binary space partition tree. Every leaf
//! is a [`Stack`]: an ordered list of panes, exactly one of them active
//! (shown). The other panes of a stack are hidden: alive and sized like the
//! active pane, but not drawn. In the UI, the panes of a stack are its tabs. A
//! stack with one pane behaves exactly like a plain pane.
//!
//! The layout tracks ids, tree shape and focus only; it knows nothing about
//! terminals.
//!
//! # Invariants
//!
//! After any public call:
//!
//! 1. Every pane id appears in exactly one stack, exactly once.
//! 2. Every stack has at least one pane.
//! 3. Every stack's active pane is one of its panes.
//! 4. The focused pane is in the layout and is the active pane of its stack.
//! 5. No split has a missing child: removing a stack's last pane collapses
//!    its parent split.
//!
//! # Visible and hidden panes
//!
//! [`TileLayout::panes`] returns visible panes only (the active pane of each
//! stack), so drawing, hit testing and directional navigation see one pane per
//! stack. [`TileLayout::stacks`] lists every stack with its panes; callers size
//! hidden panes like their stack's visible pane, so showing one needs no
//! reflow.
//!
//! # Closing
//!
//! Closing a pane in a stack with other panes activates its right neighbour,
//! or its left one when it was last, and keeps focus in that stack. Closing a
//! stack's last pane removes the stack; when it held focus, focus returns to
//! the stack of the previously focused pane, else the next stack in tree
//! order, landing on that stack's active pane. Closing never changes which pane
//! another stack shows. Only moving focus to a different stack records the
//! previously focused pane.

use std::cmp::Reverse;

use ratatui::{
    layout::{Direction, Rect},
    widgets::Borders,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PaneId(u32);

/// Global atomic counter for unique PaneId generation across all workspaces.
static NEXT_PANE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl PaneId {
    /// Allocate a globally unique PaneId.
    pub fn alloc() -> Self {
        Self(NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    /// Reconstruct from a saved u32 (persistence only).
    pub fn from_raw(id: u32) -> Self {
        Self(id)
    }
}

/// Globally unique id of a layout leaf. Stable while panes move in and out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StackId(u32);

static NEXT_STACK_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl StackId {
    fn alloc() -> Self {
        Self(NEXT_STACK_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub fn raw(self) -> u32 {
        self.0
    }
}

/// A layout leaf: ordered panes, one of them active (shown).
pub struct Stack {
    id: StackId,
    panes: Vec<PaneId>,
    active: usize,
}

impl Stack {
    /// Panics when `panes` is empty. An out-of-range `active` selects the last pane.
    pub fn new(panes: Vec<PaneId>, active: usize) -> Self {
        assert!(!panes.is_empty(), "a stack holds at least one pane");
        let active = active.min(panes.len() - 1);
        Self {
            id: StackId::alloc(),
            panes,
            active,
        }
    }

    pub fn id(&self) -> StackId {
        self.id
    }

    pub fn panes(&self) -> &[PaneId] {
        &self.panes
    }

    pub fn active(&self) -> PaneId {
        self.panes[self.active]
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Keep only the panes `keep` accepts, preferring to keep the active pane
    /// shown. None when no pane is left.
    pub fn retain(mut self, keep: impl Fn(PaneId) -> bool) -> Option<Self> {
        let shown = self.active();
        self.panes.retain(|id| keep(*id));
        if self.panes.is_empty() {
            return None;
        }
        self.active = self
            .panes
            .iter()
            .position(|id| *id == shown)
            .unwrap_or(self.active.min(self.panes.len() - 1));
        Some(self)
    }

    fn contains(&self, id: PaneId) -> bool {
        self.panes.contains(&id)
    }

    fn activate(&mut self, id: PaneId) {
        if let Some(index) = self.panes.iter().position(|pane| *pane == id) {
            self.active = index;
        }
    }

    /// Remove a pane from a stack that keeps at least one other pane. A removed
    /// active pane hands over to its right neighbour, or its left one when last.
    fn remove(&mut self, id: PaneId) {
        let Some(index) = self.panes.iter().position(|pane| *pane == id) else {
            return;
        };
        self.panes.remove(index);
        if index < self.active {
            self.active -= 1;
        } else {
            self.active = self.active.min(self.panes.len() - 1);
        }
    }
}

/// A stack's panes in order and its shown pane.
#[derive(Clone)]
pub struct StackInfo {
    pub panes: Vec<PaneId>,
    pub active: PaneId,
}

/// Snapshot of a pane's position and focus state after layout.
#[derive(Clone)]
pub struct PaneInfo {
    pub id: PaneId,
    /// Outer rect (including borders if present).
    pub rect: Rect,
    /// Inner rect (content area, excluding borders). Used for selection.
    pub inner_rect: Rect,
    /// Visible scrollbar lane, when scrollback is present. `inner_rect` may still
    /// exclude a stable hidden gutter when this is `None`.
    pub scrollbar_rect: Option<Rect>,
    /// Borders drawn around this pane after UI chrome is applied.
    pub borders: Borders,
    pub is_focused: bool,
    /// Its stack holds hidden panes too, so the tile shows a tab strip.
    pub stacked: bool,
}

/// Info about a split boundary, used for mouse drag resize.
#[derive(Clone)]
pub struct SplitBorder {
    /// Position of the divider line (x for horizontal split, y for vertical).
    pub pos: u16,
    /// Direction of the split that created this border.
    pub direction: Direction,
    /// Ratio assigned to the first child of this split.
    pub ratio: f32,
    /// Total area of the split node.
    pub area: Rect,
    /// Path from root to this split node (false=first, true=second).
    pub path: Vec<bool>,
}

/// Cardinal direction for pane navigation.
#[derive(Debug, Clone, Copy)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

/// A node in the BSP tree. Public for serialization.
pub enum Node {
    Stack(Stack),
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    /// A leaf holding a single pane.
    pub fn pane(id: PaneId) -> Self {
        Node::Stack(Stack::new(vec![id], 0))
    }

    /// Stand-in while a subtree is moved out with `std::mem::replace`.
    fn placeholder() -> Self {
        Node::Stack(Stack {
            id: StackId(0),
            panes: Vec::new(),
            active: 0,
        })
    }
}

/// BSP tiling layout. Tracks a tree of splits and a focused pane.
pub struct TileLayout {
    root: Node,
    focus: PaneId,
    /// Pane focused before `focus`, used by `close_focused`. Only a real focus
    /// move to another stack writes it; tree edits go through the target-taking
    /// primitives (`split_pane`, `close_pane`, unfocused `insert_pane_near`) so
    /// internal focus excursions never corrupt it.
    prev_focus: Option<PaneId>,
}

impl TileLayout {
    /// Create a new layout with a single pane (globally unique ID).
    /// Returns (layout, root_pane_id) so the caller can create the pane.
    pub fn new() -> (Self, PaneId) {
        let root_id = PaneId::alloc();
        (
            Self {
                root: Node::pane(root_id),
                focus: root_id,
                prev_focus: None,
            },
            root_id,
        )
    }

    /// Activate `id` in its stack and focus it. Leaving another stack records
    /// the pane being left.
    fn set_focus(&mut self, id: PaneId) {
        let Some(stack) = find_stack_mut(&mut self.root, id) else {
            return;
        };
        let same_stack = stack.contains(self.focus);
        stack.activate(id);
        if id != self.focus {
            if !same_stack {
                self.prev_focus = Some(self.focus);
            }
            self.focus = id;
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        find_stack(&self.root, id).is_some()
    }

    pub fn focused(&self) -> PaneId {
        self.focus
    }

    pub fn pane_count(&self) -> usize {
        count_panes(&self.root)
    }

    /// Visible panes (the active pane of each stack), with their stack's rect.
    pub fn panes(&self, area: Rect) -> Vec<PaneInfo> {
        let mut result = Vec::new();
        collect_panes(&self.root, area, self.focus, &mut result);
        result
    }

    /// The stack holding `pane`.
    pub fn stack_of(&self, pane: PaneId) -> Option<&Stack> {
        find_stack(&self.root, pane)
    }

    /// Whether `pane` is in the layout and shown by its stack.
    pub fn is_shown(&self, pane: PaneId) -> bool {
        find_stack(&self.root, pane).is_some_and(|stack| stack.active() == pane)
    }

    /// Number of tiles (stacks) on screen.
    pub fn tile_count(&self) -> usize {
        count_stacks(&self.root)
    }

    /// Every stack in tree order.
    pub fn stacks(&self) -> Vec<StackInfo> {
        let mut result = Vec::new();
        collect_stacks(&self.root, &mut result);
        result
    }

    /// Compute pane rects as they will be after splitting `target`, without
    /// changing the layout. Returns the rects and the new pane's index; the
    /// new pane's entry carries `target`'s id.
    pub fn panes_after_split(
        &self,
        area: Rect,
        target: PaneId,
        direction: Direction,
        ratio: f32,
    ) -> Option<(Vec<PaneInfo>, usize)> {
        let shown = find_stack(&self.root, target)?.active();
        let mut panes = self.panes(area);
        let index = panes.iter().position(|info| info.id == shown)?;
        let (first, second) = split_rect(panes[index].rect, direction, valid_split_ratio(ratio));
        panes[index].rect = first;
        panes[index].inner_rect = first;
        let mut new_pane = panes[index].clone();
        new_pane.rect = second;
        new_pane.inner_rect = second;
        new_pane.is_focused = false;
        panes.insert(index + 1, new_pane);
        Some((panes, index + 1))
    }

    /// Collect all split boundaries for mouse drag resize.
    pub fn splits(&self, area: Rect) -> Vec<SplitBorder> {
        let mut result = Vec::new();
        collect_splits(&self.root, area, &mut Vec::new(), &mut result);
        result
    }

    /// Split the focused pane. Returns the new pane's id. Production splits
    /// flow through `Tab` so a failed runtime spawn can roll back; this remains
    /// as the user-split shape for tests.
    #[cfg(test)]
    pub fn split_focused(&mut self, direction: Direction) -> PaneId {
        self.split_focused_with_ratio(direction, 0.5)
    }

    /// Split the focused pane with a custom first-child ratio.
    #[cfg(test)]
    pub fn split_focused_with_ratio(&mut self, direction: Direction, ratio: f32) -> PaneId {
        let new_id = self
            .split_pane(self.focus, direction, ratio)
            .expect("focused pane is in the layout");
        self.set_focus(new_id);
        new_id
    }

    /// Split `target`'s stack without moving focus: the new pane gets its own
    /// stack. Returns the new pane's id, or None when `target` is not in the
    /// layout.
    pub fn split_pane(
        &mut self,
        target: PaneId,
        direction: Direction,
        ratio: f32,
    ) -> Option<PaneId> {
        let leaf = find_leaf_mut(&mut self.root, target)?;
        let new_id = PaneId::alloc();
        split_leaf(
            leaf,
            direction,
            Node::pane(new_id),
            valid_split_ratio(ratio),
        );
        Some(new_id)
    }

    /// Insert an existing pane id in a new stack next to `target`'s stack
    /// without allocating a new pane or spawning a terminal runtime. When
    /// `focus` is false, focus and its history are left untouched.
    pub fn insert_pane_near(
        &mut self,
        target: PaneId,
        moved: PaneId,
        direction: Direction,
        ratio: f32,
        focus: bool,
    ) -> bool {
        if target == moved || self.contains(moved) {
            return false;
        }
        let Some(leaf) = find_leaf_mut(&mut self.root, target) else {
            return false;
        };
        split_leaf(leaf, direction, Node::pane(moved), valid_split_ratio(ratio));
        if focus {
            self.set_focus(moved);
        }
        true
    }

    /// Allocate a pane in `stack`, at `index` or at the end. With `activate`,
    /// it becomes active and focused; otherwise it is hidden.
    pub fn add_to_stack(
        &mut self,
        stack: StackId,
        index: Option<usize>,
        activate: bool,
    ) -> Option<PaneId> {
        let id = PaneId::alloc();
        self.insert_tab(stack, id, index, activate).then_some(id)
    }

    /// Move `pane` into `stack`, at `index` or at the end; within its own stack
    /// this is a reorder. With `activate`, the moved pane becomes active and
    /// focused. Changes nothing and returns false when either is missing.
    pub fn move_to_stack(
        &mut self,
        pane: PaneId,
        stack: StackId,
        index: Option<usize>,
        activate: bool,
    ) -> bool {
        let Some(source) = find_stack(&self.root, pane) else {
            return false;
        };
        if source.id == stack {
            self.move_in_stack(pane, index.unwrap_or(usize::MAX));
        } else {
            if find_stack_by_id_mut(&mut self.root, stack).is_none() {
                return false;
            }
            // `pane` and `stack` are in different stacks, so this never closes
            // the last pane and never removes the target stack.
            self.close_pane(pane);
            self.insert_tab(stack, pane, index, false);
        }
        if activate {
            self.set_focus(pane);
        }
        true
    }

    /// Insert an existing pane id that is not in the layout into `stack`. To
    /// move a pane that is already in the layout, use `move_to_stack`.
    pub fn insert_tab(
        &mut self,
        stack: StackId,
        moved: PaneId,
        index: Option<usize>,
        activate: bool,
    ) -> bool {
        if self.contains(moved) {
            return false;
        }
        let Some(target) = find_stack_by_id_mut(&mut self.root, stack) else {
            return false;
        };
        let index = index.map_or(target.panes.len(), |index| index.min(target.panes.len()));
        target.panes.insert(index, moved);
        if index <= target.active {
            target.active += 1;
        }
        if activate {
            self.set_focus(moved);
        }
        true
    }

    /// Move `pane` into a new stack on `side` of `target`'s stack and focus it.
    /// Its old stack closes when it empties. Changes nothing and returns false
    /// when either is missing, or when `pane` is alone in `target`'s stack.
    pub fn split_out(&mut self, pane: PaneId, target: PaneId, side: NavDirection) -> bool {
        if !self.contains(pane) {
            return false;
        }
        let Some(anchor) = self
            .stack_of(target)
            .and_then(|stack| stack.panes.iter().copied().find(|id| *id != pane))
        else {
            return false;
        };
        // `anchor` stays in the layout, so `pane` is never its last pane.
        self.close_pane(pane);
        let Some(leaf) = find_leaf_mut(&mut self.root, anchor) else {
            return false;
        };
        let direction = match side {
            NavDirection::Left | NavDirection::Right => Direction::Horizontal,
            NavDirection::Up | NavDirection::Down => Direction::Vertical,
        };
        split_leaf(leaf, direction, Node::pane(pane), 0.5);
        if matches!(side, NavDirection::Left | NavDirection::Up) {
            if let Node::Split { first, second, .. } = leaf {
                std::mem::swap(first, second);
            }
        }
        self.set_focus(pane);
        true
    }

    /// Show `pane` in its stack. When that stack holds focus, focus moves to
    /// `pane` too, so the focused pane stays shown.
    pub fn show(&mut self, pane: PaneId) {
        let Some(stack) = find_stack_mut(&mut self.root, pane) else {
            return;
        };
        if stack.contains(self.focus) {
            self.set_focus(pane);
        } else {
            stack.activate(pane);
        }
    }

    /// The tile where a companion of `pane` opens as a tab, so it never hides
    /// `pane` itself: the tile focused before the current one when it borders
    /// `pane`'s tile, else the nearest tile toward `toward`. Returns the pane
    /// that tile shows. None means there is no such neighbour and the caller
    /// should split instead.
    pub fn companion_tile(&self, pane: PaneId, toward: NavDirection, area: Rect) -> Option<PaneId> {
        let own = self.stack_of(pane)?;
        let shown = self.panes(area);
        let tile_of = |pane: PaneId| {
            let active = self.stack_of(pane)?.active();
            shown.iter().find(|info| info.id == active)
        };
        let own_tile = tile_of(pane)?;
        let remembered = self
            .prev_focus
            .filter(|prev| !own.contains(*prev))
            .and_then(tile_of)
            .filter(|tile| rects_touch(tile.rect, own_tile.rect))
            .map(|tile| tile.id);
        remembered.or_else(|| find_in_direction(own_tile, toward, &shown))
    }

    /// Move `pane` to `index` within its stack. The shown pane and focus do not
    /// change.
    pub fn move_in_stack(&mut self, pane: PaneId, index: usize) -> bool {
        let Some(stack) = find_stack_mut(&mut self.root, pane) else {
            return false;
        };
        let shown = stack.active();
        let from = stack.panes.iter().position(|id| *id == pane).unwrap();
        let to = index.min(stack.panes.len() - 1);
        if from == to {
            return false;
        }
        let moved = stack.panes.remove(from);
        stack.panes.insert(to, moved);
        stack.activate(shown);
        true
    }

    /// Close the focused pane. Within a stack, its neighbour takes over;
    /// closing a stack's last pane returns focus to the stack it came from when
    /// that stack is still open. Returns false if it's the last pane.
    pub fn close_focused(&mut self) -> bool {
        if self.pane_count() <= 1 {
            return false;
        }
        let target = self.focus;
        let Some(stack) = find_stack_mut(&mut self.root, target) else {
            return false;
        };
        if stack.panes.len() > 1 {
            stack.remove(target);
            self.focus = stack.active();
            return true;
        }
        let visible = self.visible_pane_ids();
        let pos = visible.iter().position(|id| *id == target).unwrap();
        let ordered = if pos + 1 < visible.len() {
            visible[pos + 1]
        } else {
            visible[pos - 1]
        };
        let new_focus = self
            .prev_focus
            .filter(|prev| *prev != target)
            .and_then(|prev| find_stack(&self.root, prev))
            .map_or(ordered, Stack::active);
        let old = std::mem::replace(&mut self.root, Node::placeholder());
        if let Some(new_root) = remove_pane(old, target) {
            self.root = new_root;
            self.focus = new_focus;
            self.prev_focus = None;
            true
        } else {
            false
        }
    }

    /// Close any pane. Focus and its history are left alone unless the closed
    /// pane is the focused one.
    pub fn close_pane(&mut self, id: PaneId) -> bool {
        if self.focus == id {
            return self.close_focused();
        }
        if self.pane_count() <= 1 || !self.contains(id) {
            return false;
        }
        let old = std::mem::replace(&mut self.root, Node::placeholder());
        let Some(new_root) = remove_pane(old, id) else {
            return false;
        };
        self.root = new_root;
        if self.prev_focus == Some(id) {
            self.prev_focus = None;
        }
        true
    }

    /// Focus `id`, activating it in its stack.
    pub fn focus_pane(&mut self, id: PaneId) {
        self.set_focus(id);
    }

    /// Swap two pane ids in the layout tree while preserving split shape and
    /// ratios, across stacks too. Each position keeps its active state, except
    /// that the focused pane is re-activated wherever it lands. Returns true
    /// only when both panes exist and are different.
    pub fn swap_panes(&mut self, first: PaneId, second: PaneId) -> bool {
        if first == second {
            return false;
        }
        let ids = self.pane_ids();
        if !ids.contains(&first) || !ids.contains(&second) {
            return false;
        }
        swap_pane_ids(&mut self.root, first, second);
        if let Some(stack) = find_stack_mut(&mut self.root, self.focus) {
            stack.activate(self.focus);
        }
        true
    }

    /// Set the ratio of a split node at the given path.
    pub fn set_ratio_at(&mut self, path: &[bool], ratio: f32) -> bool {
        set_ratio_at(&mut self.root, path, ratio.clamp(0.1, 0.9))
    }

    /// Adjust the nearest split in the given direction for the focused pane.
    /// `delta` is positive to grow, negative to shrink.
    pub fn resize_focused(&mut self, nav: NavDirection, delta: f32, area: Rect) {
        let panes = self.panes(area);
        let Some(focused) = panes.iter().find(|p| p.is_focused) else {
            return;
        };
        let focused_rect = focused.rect;
        let splits = self.splits(area);

        let target_dir = match nav {
            NavDirection::Left | NavDirection::Right => Direction::Horizontal,
            NavDirection::Up | NavDirection::Down => Direction::Vertical,
        };
        let grows = matches!(nav, NavDirection::Right | NavDirection::Down);

        let best = nearest_resize_split(&splits, target_dir, focused_rect, nav).or_else(|| {
            nearest_resize_split(&splits, target_dir, focused_rect, opposite_direction(nav))
        });

        if let Some(split) = best {
            let path = split.path.clone();
            let current_ratio = get_ratio_at(&self.root, &path).unwrap_or(0.5);
            let adj = if grows { delta } else { -delta };
            self.set_ratio_at(&path, current_ratio + adj);
        }
    }

    pub fn resize_pane(
        &mut self,
        pane_id: PaneId,
        nav: NavDirection,
        delta: f32,
        area: Rect,
    ) -> bool {
        let Some(shown) = find_stack(&self.root, pane_id).map(Stack::active) else {
            return false;
        };
        let before = split_ratios(&self.root);
        let previous_focus = self.focus;
        self.focus = shown;
        self.resize_focused(nav, delta, area);
        self.focus = previous_focus;
        split_ratios(&self.root) != before
    }

    /// All panes, hidden ones included, in tree and stack order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        collect_ids(&self.root, &mut ids);
        ids
    }

    fn visible_pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        collect_visible_ids(&self.root, &mut ids);
        ids
    }

    /// Access the tree root for serialization.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Reconstruct a layout from a saved tree, activating `focus` in its stack.
    pub fn from_saved(root: Node, focus: PaneId) -> Self {
        let mut layout = Self {
            root,
            focus,
            prev_focus: None,
        };
        if let Some(stack) = find_stack_mut(&mut layout.root, focus) {
            stack.activate(focus);
        }
        layout
    }
}

// --- Directional pane navigation ---

/// Find the nearest pane in the given direction from `focused`.
pub fn find_in_direction(
    focused: &PaneInfo,
    direction: NavDirection,
    panes: &[PaneInfo],
) -> Option<PaneId> {
    let fr = focused.rect;

    panes
        .iter()
        .enumerate()
        .filter(|(_, p)| p.id != focused.id)
        .filter(|(_, p)| {
            let r = p.rect;
            match direction {
                NavDirection::Left => {
                    r.x + r.width <= fr.x && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Right => {
                    r.x >= fr.x + fr.width && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up => {
                    r.y + r.height <= fr.y && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
                NavDirection::Down => {
                    r.y >= fr.y + fr.height && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
            }
        })
        .min_by_key(|(index, p)| {
            let r = p.rect;
            let edge_distance = match direction {
                NavDirection::Left => fr.x.saturating_sub(r.x + r.width),
                NavDirection::Right => r.x.saturating_sub(fr.x + fr.width),
                NavDirection::Up => fr.y.saturating_sub(r.y + r.height),
                NavDirection::Down => r.y.saturating_sub(fr.y + fr.height),
            };
            let overlap = match direction {
                NavDirection::Left | NavDirection::Right => {
                    range_overlap_amount(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up | NavDirection::Down => {
                    range_overlap_amount(r.x, r.width, fr.x, fr.width)
                }
            };
            let center_distance = match direction {
                NavDirection::Left | NavDirection::Right => {
                    range_center_distance(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up | NavDirection::Down => {
                    range_center_distance(r.x, r.width, fr.x, fr.width)
                }
            };
            (edge_distance, Reverse(overlap), center_distance, *index)
        })
        .map(|(_, p)| p.id)
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start + b_len && a_start + a_len > b_start
}

/// Whether two non-overlapping rects share part of an edge.
fn rects_touch(a: Rect, b: Rect) -> bool {
    ((a.right() == b.x || b.right() == a.x) && ranges_overlap(a.y, a.height, b.y, b.height))
        || ((a.bottom() == b.y || b.bottom() == a.y) && ranges_overlap(a.x, a.width, b.x, b.width))
}

fn split_on_requested_edge(split: &SplitBorder, focused: Rect, nav: NavDirection) -> bool {
    split_edge_distance(split, focused, nav) <= 1
}

fn split_area_overlaps_focused_pane(split: &SplitBorder, focused: Rect, nav: NavDirection) -> bool {
    match nav {
        NavDirection::Left | NavDirection::Right => {
            ranges_overlap(split.area.y, split.area.height, focused.y, focused.height)
        }
        NavDirection::Up | NavDirection::Down => {
            ranges_overlap(split.area.x, split.area.width, focused.x, focused.width)
        }
    }
}

fn nearest_resize_split(
    splits: &[SplitBorder],
    target_dir: Direction,
    focused: Rect,
    nav: NavDirection,
) -> Option<&SplitBorder> {
    splits
        .iter()
        .filter(|s| s.direction == target_dir)
        .filter(|s| split_area_overlaps_focused_pane(s, focused, nav))
        .filter(|s| split_on_requested_edge(s, focused, nav))
        .min_by_key(|s| split_edge_distance(s, focused, nav))
}

fn opposite_direction(nav: NavDirection) -> NavDirection {
    match nav {
        NavDirection::Left => NavDirection::Right,
        NavDirection::Right => NavDirection::Left,
        NavDirection::Up => NavDirection::Down,
        NavDirection::Down => NavDirection::Up,
    }
}

fn split_edge_distance(split: &SplitBorder, focused: Rect, nav: NavDirection) -> u32 {
    match nav {
        NavDirection::Left => (split.pos as i32 - focused.x as i32).unsigned_abs(),
        NavDirection::Right => {
            (split.pos as i32 - (focused.x + focused.width) as i32).unsigned_abs()
        }
        NavDirection::Up => (split.pos as i32 - focused.y as i32).unsigned_abs(),
        NavDirection::Down => {
            (split.pos as i32 - (focused.y + focused.height) as i32).unsigned_abs()
        }
    }
}

fn range_overlap_amount(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> u16 {
    let a_end = a_start.saturating_add(a_len);
    let b_end = b_start.saturating_add(b_len);
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

fn range_center_distance(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> u16 {
    let a_center = a_start.saturating_mul(2).saturating_add(a_len);
    let b_center = b_start.saturating_mul(2).saturating_add(b_len);
    a_center.abs_diff(b_center)
}

// --- Tree operations ---

fn count_panes(node: &Node) -> usize {
    match node {
        Node::Stack(stack) => stack.panes.len(),
        Node::Split { first, second, .. } => count_panes(first) + count_panes(second),
    }
}

fn count_stacks(node: &Node) -> usize {
    match node {
        Node::Stack(_) => 1,
        Node::Split { first, second, .. } => count_stacks(first) + count_stacks(second),
    }
}

fn collect_panes(node: &Node, area: Rect, focus: PaneId, result: &mut Vec<PaneInfo>) {
    match node {
        Node::Stack(stack) => {
            let id = stack.active();
            result.push(PaneInfo {
                id,
                rect: area,
                // inner_rect is set during render when we know if borders are shown
                inner_rect: area,
                scrollbar_rect: None,
                borders: Borders::NONE,
                is_focused: id == focus,
                stacked: stack.panes.len() > 1,
            });
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (a, b) = split_rect(area, *direction, *ratio);
            collect_panes(first, a, focus, result);
            collect_panes(second, b, focus, result);
        }
    }
}

fn collect_stacks(node: &Node, result: &mut Vec<StackInfo>) {
    match node {
        Node::Stack(stack) => result.push(StackInfo {
            panes: stack.panes.clone(),
            active: stack.active(),
        }),
        Node::Split { first, second, .. } => {
            collect_stacks(first, result);
            collect_stacks(second, result);
        }
    }
}

fn collect_splits(node: &Node, area: Rect, path: &mut Vec<bool>, result: &mut Vec<SplitBorder>) {
    if let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    {
        let (a, b) = split_rect(area, *direction, *ratio);
        let pos = match direction {
            Direction::Horizontal => a.x + a.width,
            Direction::Vertical => a.y + a.height,
        };
        result.push(SplitBorder {
            pos,
            direction: *direction,
            ratio: *ratio,
            area,
            path: path.clone(),
        });
        path.push(false);
        collect_splits(first, a, path, result);
        path.pop();
        path.push(true);
        collect_splits(second, b, path, result);
        path.pop();
    }
}

fn collect_ids(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Stack(stack) => ids.extend_from_slice(&stack.panes),
        Node::Split { first, second, .. } => {
            collect_ids(first, ids);
            collect_ids(second, ids);
        }
    }
}

fn collect_visible_ids(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Stack(stack) => ids.push(stack.active()),
        Node::Split { first, second, .. } => {
            collect_visible_ids(first, ids);
            collect_visible_ids(second, ids);
        }
    }
}

fn split_ratios(node: &Node) -> Vec<(Vec<bool>, f32)> {
    fn collect(node: &Node, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, f32)>) {
        match node {
            Node::Stack(_) => {}
            Node::Split {
                ratio,
                first,
                second,
                ..
            } => {
                out.push((path.clone(), *ratio));
                path.push(false);
                collect(first, path, out);
                path.pop();
                path.push(true);
                collect(second, path, out);
                path.pop();
            }
        }
    }

    let mut out = Vec::new();
    collect(node, &mut Vec::new(), &mut out);
    out
}

fn swap_pane_ids(node: &mut Node, first: PaneId, second: PaneId) {
    match node {
        Node::Stack(stack) => {
            for id in &mut stack.panes {
                if *id == first {
                    *id = second;
                } else if *id == second {
                    *id = first;
                }
            }
        }
        Node::Split {
            first: first_child,
            second: second_child,
            ..
        } => {
            swap_pane_ids(first_child, first, second);
            swap_pane_ids(second_child, first, second);
        }
    }
}

fn find_stack(node: &Node, pane: PaneId) -> Option<&Stack> {
    match node {
        Node::Stack(stack) => stack.contains(pane).then_some(stack),
        Node::Split { first, second, .. } => {
            find_stack(first, pane).or_else(|| find_stack(second, pane))
        }
    }
}

/// The leaf node whose stack holds `pane`.
fn find_leaf_mut(node: &mut Node, pane: PaneId) -> Option<&mut Node> {
    match node {
        Node::Stack(stack) if stack.contains(pane) => Some(node),
        Node::Stack(_) => None,
        Node::Split { first, second, .. } => {
            find_leaf_mut(first, pane).or_else(|| find_leaf_mut(second, pane))
        }
    }
}

fn find_stack_mut(node: &mut Node, pane: PaneId) -> Option<&mut Stack> {
    match find_leaf_mut(node, pane)? {
        Node::Stack(stack) => Some(stack),
        Node::Split { .. } => None,
    }
}

fn find_stack_by_id_mut(node: &mut Node, id: StackId) -> Option<&mut Stack> {
    match node {
        Node::Stack(stack) => (stack.id == id).then_some(stack),
        Node::Split { first, second, .. } => {
            find_stack_by_id_mut(first, id).or_else(|| find_stack_by_id_mut(second, id))
        }
    }
}

/// Replace `leaf` with a split holding it first and `second` after it.
fn split_leaf(leaf: &mut Node, direction: Direction, second: Node, split_ratio: f32) {
    let first = std::mem::replace(leaf, Node::placeholder());
    *leaf = Node::Split {
        direction,
        ratio: split_ratio,
        first: Box::new(first),
        second: Box::new(second),
    };
}

#[cfg(test)]
fn split_node(target: PaneId, direction: Direction, new_id: PaneId, split_ratio: f32) -> Node {
    let mut node = Node::pane(target);
    split_leaf(&mut node, direction, Node::pane(new_id), split_ratio);
    node
}

pub(crate) fn valid_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0.1, 0.9)
    } else {
        0.5
    }
}

fn remove_pane(node: Node, target: PaneId) -> Option<Node> {
    match node {
        Node::Stack(stack) if !stack.contains(target) => Some(Node::Stack(stack)),
        Node::Stack(stack) if stack.panes.len() == 1 => None,
        Node::Stack(mut stack) => {
            stack.remove(target);
            Some(Node::Stack(stack))
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => match (remove_pane(*first, target), remove_pane(*second, target)) {
            (None, Some(s)) => Some(s),
            (Some(f), None) => Some(f),
            (Some(f), Some(s)) => Some(Node::Split {
                direction,
                ratio,
                first: Box::new(f),
                second: Box::new(s),
            }),
            (None, None) => None,
        },
    }
}

fn set_ratio_at(node: &mut Node, path: &[bool], new_ratio: f32) -> bool {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            *ratio = new_ratio;
            true
        } else if path[0] {
            set_ratio_at(second, &path[1..], new_ratio)
        } else {
            set_ratio_at(first, &path[1..], new_ratio)
        }
    } else {
        false
    }
}

fn get_ratio_at(node: &Node, path: &[bool]) -> Option<f32> {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            Some(*ratio)
        } else if path[0] {
            get_ratio_at(second, &path[1..])
        } else {
            get_ratio_at(first, &path[1..])
        }
    } else {
        None
    }
}

fn split_rect(area: Rect, direction: Direction, ratio: f32) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let first_w = ((area.width as f32) * ratio).round() as u16;
            let second_w = area.width.saturating_sub(first_w);
            (
                Rect::new(area.x, area.y, first_w, area.height),
                Rect::new(area.x + first_w, area.y, second_w, area.height),
            )
        }
        Direction::Vertical => {
            let first_h = ((area.height as f32) * ratio).round() as u16;
            let second_h = area.height.saturating_sub(first_h);
            (
                Rect::new(area.x, area.y, area.width, first_h),
                Rect::new(area.x, area.y + first_h, area.width, second_h),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_paths_preserve_preorder_geometry_and_resize_targets() {
        let ids = [
            PaneId::alloc(),
            PaneId::alloc(),
            PaneId::alloc(),
            PaneId::alloc(),
        ];
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.5,
                first: Box::new(split_node(ids[0], Direction::Vertical, ids[1], 0.5)),
                second: Box::new(split_node(ids[2], Direction::Vertical, ids[3], 0.25)),
            },
            ids[0],
        );
        let area = Rect::new(3, 7, 120, 80);
        let splits = layout.splits(area);
        assert_eq!(splits.len(), 3);
        for (split, (path, direction, pos, rect)) in splits.iter().zip([
            (vec![], Direction::Horizontal, 63, area),
            (
                vec![false],
                Direction::Vertical,
                47,
                Rect::new(3, 7, 60, 80),
            ),
            (
                vec![true],
                Direction::Vertical,
                27,
                Rect::new(63, 7, 60, 80),
            ),
        ]) {
            assert_eq!(
                (split.path.clone(), split.direction, split.pos, split.area),
                (path, direction, pos, rect)
            );
        }
        assert!(layout.set_ratio_at(&splits[2].path, 0.5));
        let resized = layout.splits(area);
        assert_eq!(resized[1].pos, splits[1].pos);
        assert_eq!(resized[2].pos, 47);
    }

    #[test]
    fn rejected_splits_and_insertions_preserve_layout_and_focus_history() {
        let (mut layout, root) = TileLayout::new();
        let second = layout.split_focused(Direction::Horizontal);
        let absent = PaneId::alloc();
        let before = pane_rects(&layout);
        let splits = split_snapshot(&layout);
        assert!(layout
            .split_pane(absent, Direction::Vertical, 0.5)
            .is_none());
        for (target, moved) in [(absent, PaneId::alloc()), (root, second), (root, root)] {
            assert!(!layout.insert_pane_near(target, moved, Direction::Vertical, 0.3, true));
        }
        assert_eq!(pane_rects(&layout), before);
        assert_eq!(split_snapshot(&layout), splits);
        assert_eq!(layout.focused(), second);
        assert_eq!(layout.prev_focus, Some(root));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), root);
    }

    #[test]
    fn splitting_a_deep_leaf_preserves_other_branches_and_focus() {
        let (mut layout, root) = TileLayout::new();
        let right = layout.split_pane(root, Direction::Horizontal, 0.6).unwrap();
        let bottom_left = layout.split_pane(root, Direction::Vertical, 0.4).unwrap();
        let right_rect = pane_rect(&layout, right);
        let focus = layout.focused();
        let new = layout
            .split_pane(bottom_left, Direction::Horizontal, f32::NAN)
            .unwrap();
        assert_eq!(layout.pane_ids(), [root, bottom_left, new, right]);
        assert_eq!(pane_rect(&layout, right), right_rect);
        assert_eq!(layout.focused(), focus);
        assert_eq!(
            split_snapshot(&layout),
            [
                (Direction::Horizontal, 0.6),
                (Direction::Vertical, 0.4),
                (Direction::Horizontal, 0.5)
            ]
        );
    }

    #[test]
    fn panes_after_split_predicts_the_real_split_geometry() {
        let area = Rect::new(3, 1, 121, 37);
        let (mut layout, root) = TileLayout::new();
        let right = layout.split_pane(root, Direction::Horizontal, 0.6).unwrap();
        let bottom_left = layout.split_pane(root, Direction::Vertical, 0.4).unwrap();
        for (target, direction, ratio) in [
            (bottom_left, Direction::Horizontal, 0.5),
            (right, Direction::Vertical, 0.3),
            (root, Direction::Horizontal, f32::NAN),
        ] {
            let (predicted, new_index) = layout
                .panes_after_split(area, target, direction, ratio)
                .unwrap();
            let new_id = layout.split_pane(target, direction, ratio).unwrap();
            let actual: Vec<_> = layout
                .panes(area)
                .into_iter()
                .map(|info| info.rect)
                .collect();
            assert_eq!(
                predicted.iter().map(|info| info.rect).collect::<Vec<_>>(),
                actual
            );
            assert_eq!(layout.pane_ids()[new_index], new_id);
        }
        assert!(layout
            .panes_after_split(area, PaneId::alloc(), Direction::Vertical, 0.5)
            .is_none());
    }

    #[test]
    #[ignore = "manual BSP allocation and traversal scaling profile"]
    fn bsp_layout_profile() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};
        fn build(count: usize, balanced: bool) -> TileLayout {
            let (mut layout, root) = TileLayout::new();
            let mut leaves = std::collections::VecDeque::from([root]);
            for _ in 1..count {
                let target = leaves.pop_front().unwrap();
                let new = layout
                    .split_pane(target, Direction::Horizontal, 0.5)
                    .unwrap();
                if balanced {
                    leaves.push_back(target);
                }
                leaves.push_back(new);
            }
            layout
        }
        fn measure(mut operation: impl FnMut()) -> f64 {
            for _ in 0..32 {
                operation();
            }
            let mut samples = Vec::new();
            for _ in 0..7 {
                let start = Instant::now();
                let mut iterations = 0;
                while start.elapsed() < Duration::from_millis(20) {
                    operation();
                    iterations += 1;
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations));
            }
            samples.sort_by(f64::total_cmp);
            samples[3]
        }
        for count in [1, 15, 128, 512] {
            for balanced in [true, false] {
                let layout = build(count, balanced);
                let splits = measure(|| {
                    black_box(layout.splits(Rect::new(0, 0, 120, 40)));
                });
                let build = measure(|| {
                    black_box(build(count, balanced));
                });
                println!("bsp panes={count} balanced={balanced} splits_us={splits:.3} build_us={build:.3}");
            }
        }
    }

    fn pane(id: u32) -> PaneId {
        PaneId::from_raw(id)
    }

    fn sample_layout() -> TileLayout {
        TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.3,
                first: Box::new(Node::pane(pane(1))),
                second: Box::new(Node::Split {
                    direction: Direction::Vertical,
                    ratio: 0.6,
                    first: Box::new(Node::pane(pane(2))),
                    second: Box::new(Node::Split {
                        direction: Direction::Horizontal,
                        ratio: 0.4,
                        first: Box::new(Node::pane(pane(3))),
                        second: Box::new(Node::pane(pane(4))),
                    }),
                }),
            },
            pane(2),
        )
    }

    fn pane_rects(layout: &TileLayout) -> Vec<(PaneId, Rect)> {
        layout
            .panes(Rect::new(0, 0, 100, 40))
            .into_iter()
            .map(|info| (info.id, info.rect))
            .collect()
    }

    fn pane_rect(layout: &TileLayout, pane_id: PaneId) -> Rect {
        pane_rects(layout)
            .into_iter()
            .find_map(|(id, rect)| (id == pane_id).then_some(rect))
            .expect("pane should exist")
    }

    fn split_snapshot(layout: &TileLayout) -> Vec<(Direction, f32)> {
        fn collect(node: &Node, out: &mut Vec<(Direction, f32)>) {
            match node {
                Node::Stack(_) => {}
                Node::Split {
                    direction,
                    ratio,
                    first,
                    second,
                } => {
                    out.push((*direction, *ratio));
                    collect(first, out);
                    collect(second, out);
                }
            }
        }

        let mut out = Vec::new();
        collect(layout.root(), &mut out);
        out
    }

    #[test]
    fn swap_panes_exchanges_leaf_ids_without_changing_cells() {
        let mut layout = sample_layout();
        let before_rects = pane_rects(&layout);
        let before_splits = split_snapshot(&layout);

        assert!(layout.swap_panes(pane(2), pane(4)));

        assert_eq!(layout.pane_count(), 4);
        assert_eq!(split_snapshot(&layout), before_splits);
        assert_eq!(layout.focused(), pane(2));

        let after_rects = pane_rects(&layout);
        assert_eq!(after_rects[0], before_rects[0]);
        assert_eq!(after_rects[1], (pane(4), before_rects[1].1));
        assert_eq!(after_rects[2], before_rects[2]);
        assert_eq!(after_rects[3], (pane(2), before_rects[3].1));
    }

    #[test]
    fn swap_panes_is_noop_for_same_or_missing_pane() {
        let mut layout = sample_layout();
        let before_rects = pane_rects(&layout);
        let before_splits = split_snapshot(&layout);
        let before_focus = layout.focused();

        assert!(!layout.swap_panes(pane(2), pane(2)));
        assert!(!layout.swap_panes(pane(2), pane(99)));
        assert!(!layout.swap_panes(pane(99), pane(2)));

        assert_eq!(pane_rects(&layout), before_rects);
        assert_eq!(split_snapshot(&layout), before_splits);
        assert_eq!(layout.focused(), before_focus);
    }

    #[test]
    fn insert_existing_pane_near_target_preserves_existing_ids_and_focuses_moved_pane() {
        let (mut layout, root) = TileLayout::new();
        let moved = pane(99);

        assert!(layout.insert_pane_near(root, moved, Direction::Horizontal, 0.25, true));

        assert_eq!(layout.pane_count(), 2);
        assert_eq!(layout.pane_ids(), vec![root, moved]);
        assert_eq!(layout.focused(), moved);
        let splits = split_snapshot(&layout);
        assert_eq!(splits, vec![(Direction::Horizontal, 0.25)]);
        assert_eq!(pane_rect(&layout, root), Rect::new(0, 0, 25, 40));
        assert_eq!(pane_rect(&layout, moved), Rect::new(25, 0, 75, 40));
    }

    #[test]
    fn split_focused_with_ratio_sets_new_split_ratio() {
        let (mut layout, root) = TileLayout::new();
        layout.focus_pane(root);

        layout.split_focused_with_ratio(Direction::Horizontal, 0.333);

        let splits = split_snapshot(&layout);
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].0, Direction::Horizontal);
        assert!((splits[0].1 - 0.333).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_pane_preserves_focus_and_reports_change() {
        let mut layout = sample_layout();
        let original_focus = layout.focused();

        assert!(layout.resize_pane(pane(1), NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40),));

        assert_eq!(layout.focused(), original_focus);
        let split = split_snapshot(&layout)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.35).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_second_child_toward_split_decreases_ratio() {
        let (mut layout, root) = TileLayout::new();
        let right = layout.split_focused(Direction::Horizontal);
        layout.focus_pane(root);

        assert!(layout.resize_pane(right, NavDirection::Left, 0.05, Rect::new(0, 0, 100, 40),));

        let split = split_snapshot(&layout)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.45).abs() < f32::EPSILON);
        assert_eq!(layout.focused(), root);
    }

    #[test]
    fn resize_outer_edges_shrink_focused_pane() {
        let (mut horizontal, left) = TileLayout::new();
        horizontal.split_focused(Direction::Horizontal);

        assert!(horizontal.resize_pane(left, NavDirection::Left, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&horizontal)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.45).abs() < f32::EPSILON);

        let (mut horizontal, _left) = TileLayout::new();
        let right = horizontal.split_focused(Direction::Horizontal);

        assert!(horizontal.resize_pane(right, NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&horizontal)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.55).abs() < f32::EPSILON);

        let (mut vertical, top) = TileLayout::new();
        vertical.split_focused(Direction::Vertical);

        assert!(vertical.resize_pane(top, NavDirection::Up, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&vertical)[0];
        assert_eq!(split.0, Direction::Vertical);
        assert!((split.1 - 0.45).abs() < f32::EPSILON);

        let (mut vertical, _top) = TileLayout::new();
        let bottom = vertical.split_focused(Direction::Vertical);

        assert!(vertical.resize_pane(bottom, NavDirection::Down, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&vertical)[0];
        assert_eq!(split.0, Direction::Vertical);
        assert!((split.1 - 0.55).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_outer_edge_falls_back_to_horizontal_ancestor_split() {
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.6,
                first: Box::new(Node::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    first: Box::new(Node::pane(pane(1))),
                    second: Box::new(Node::pane(pane(2))),
                }),
                second: Box::new(Node::pane(pane(3))),
            },
            pane(1),
        );
        let before = pane_rect(&layout, pane(1));

        assert!(layout.resize_pane(pane(1), NavDirection::Left, 0.05, Rect::new(0, 0, 100, 40),));

        let after = pane_rect(&layout, pane(1));
        assert_eq!(after.height, before.height);
        assert!(after.width < before.width);
        let splits = split_snapshot(&layout);
        assert_eq!(splits[0].0, Direction::Horizontal);
        assert!((splits[0].1 - 0.55).abs() < f32::EPSILON);
        assert_eq!(splits[1], (Direction::Vertical, 0.5));
    }

    #[test]
    fn resize_outer_edge_falls_back_to_vertical_ancestor_split() {
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Vertical,
                ratio: 0.6,
                first: Box::new(Node::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    first: Box::new(Node::pane(pane(1))),
                    second: Box::new(Node::pane(pane(2))),
                }),
                second: Box::new(Node::pane(pane(3))),
            },
            pane(1),
        );
        let before = pane_rect(&layout, pane(1));

        assert!(layout.resize_pane(pane(1), NavDirection::Up, 0.05, Rect::new(0, 0, 100, 40),));

        let after = pane_rect(&layout, pane(1));
        assert_eq!(after.width, before.width);
        assert!(after.height < before.height);
        let splits = split_snapshot(&layout);
        assert_eq!(splits[0].0, Direction::Vertical);
        assert!((splits[0].1 - 0.55).abs() < f32::EPSILON);
        assert_eq!(splits[1], (Direction::Horizontal, 0.5));
    }

    #[test]
    fn resize_uses_split_in_same_branch_when_borders_share_coordinate() {
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    first: Box::new(Node::pane(pane(1))),
                    second: Box::new(Node::pane(pane(2))),
                }),
                second: Box::new(Node::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    first: Box::new(Node::pane(pane(3))),
                    second: Box::new(Node::pane(pane(4))),
                }),
            },
            pane(3),
        );

        assert!(layout.resize_pane(pane(3), NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40),));

        let splits = split_snapshot(&layout);
        assert_eq!(splits[0], (Direction::Vertical, 0.5));
        assert_eq!(splits[1], (Direction::Horizontal, 0.5));
        assert_eq!(splits[2].0, Direction::Horizontal);
        assert!((splits[2].1 - 0.55).abs() < f32::EPSILON);
    }

    #[test]
    fn find_in_direction_tiebreaks_by_larger_overlap_before_layout_order() {
        let focused = PaneInfo {
            id: pane(1),
            rect: Rect::new(10, 10, 10, 10),
            inner_rect: Rect::new(10, 10, 10, 10),
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: true,
            stacked: false,
        };
        let small_overlap_first = PaneInfo {
            id: pane(2),
            rect: Rect::new(0, 10, 10, 2),
            inner_rect: Rect::new(0, 10, 10, 2),
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: false,
            stacked: false,
        };
        let larger_overlap_second = PaneInfo {
            id: pane(3),
            rect: Rect::new(0, 10, 10, 8),
            inner_rect: Rect::new(0, 10, 10, 8),
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: false,
            stacked: false,
        };
        let panes = vec![focused.clone(), small_overlap_first, larger_overlap_second];

        assert_eq!(
            find_in_direction(&focused, NavDirection::Left, &panes),
            Some(pane(3))
        );
    }

    #[test]
    fn close_focused_returns_to_the_pane_focus_came_from() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn close_focused_returns_to_the_pane_that_opened_a_split() {
        // Allocated ids only: sample_layout() uses from_raw and shares the id
        // space with the allocator.
        let (mut layout, first) = TileLayout::new();
        let second = layout.split_focused(Direction::Horizontal);
        let third = layout.split_focused(Direction::Vertical);
        assert_eq!(layout.pane_ids().len(), 3);

        layout.focus_pane(first);
        let opened = layout.split_focused(Direction::Horizontal);
        assert_eq!(layout.focused(), opened);

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), first);
        assert!(layout.pane_ids().contains(&second));
        assert!(layout.pane_ids().contains(&third));
    }

    #[test]
    fn closing_a_background_pane_keeps_the_focused_pane_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_pane(pane(1)));
        assert_eq!(layout.focused(), pane(4));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn closing_the_remembered_pane_drops_the_focus_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_pane(pane(2)));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(3));
    }

    #[test]
    fn close_focused_uses_tree_order_without_focus_history() {
        let mut layout = sample_layout();

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), pane(3));
    }

    #[test]
    fn close_focused_does_not_reuse_history_after_it_is_consumed() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(3));
    }

    #[test]
    fn resize_does_not_disturb_the_close_focus_target() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));
        layout.resize_pane(pane(1), NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40));

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn split_pane_leaves_focus_and_history_untouched() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        let new_id = layout
            .split_pane(pane(1), Direction::Horizontal, 0.5)
            .expect("target exists");

        assert!(layout.pane_ids().contains(&new_id));
        assert_eq!(layout.focused(), pane(4));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn split_pane_missing_target_changes_nothing() {
        let mut layout = sample_layout();
        let ids = layout.pane_ids();

        assert_eq!(
            layout.split_pane(pane(99), Direction::Horizontal, 0.5),
            None
        );

        assert_eq!(layout.pane_ids(), ids);
    }

    #[test]
    fn insert_pane_near_unfocused_keeps_focus_and_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.insert_pane_near(pane(1), pane(9), Direction::Horizontal, 0.5, false));

        assert_eq!(layout.focused(), pane(4));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn failed_split_rollback_preserves_focus_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        let new_id = layout
            .split_pane(layout.focused(), Direction::Horizontal, 0.5)
            .expect("target exists");
        assert!(layout.close_pane(new_id));

        assert_eq!(layout.focused(), pane(4));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }
}

#[cfg(test)]
mod stack_properties {
    use std::collections::{HashMap, HashSet};

    use proptest::prelude::*;
    use proptest::sample::Index;

    use super::*;

    const AREA: Rect = Rect::new(0, 0, 200, 60);
    /// Upper bound for generated insert positions; larger values exercise clamping.
    const MAX_SLOT: usize = 8;

    #[derive(Debug, Clone)]
    enum Op {
        Split {
            pane: Index,
            horizontal: bool,
        },
        AddToStack {
            stack: Index,
            index: Option<Index>,
            activate: bool,
        },
        MoveToStack {
            pane: Index,
            stack: Index,
            index: Option<Index>,
            activate: bool,
        },
        Close {
            pane: Index,
        },
        CloseFocused,
        Focus {
            pane: Index,
        },
        Show {
            pane: Index,
        },
        SplitOut {
            pane: Index,
            target: Index,
            side: u8,
        },
        Reorder {
            pane: Index,
            index: Index,
        },
        Swap {
            first: Index,
            second: Index,
        },
    }

    fn op() -> impl Strategy<Value = Op> {
        let slot = || proptest::option::of(any::<Index>());
        prop_oneof![
            3 => (any::<Index>(), any::<bool>())
                .prop_map(|(pane, horizontal)| Op::Split { pane, horizontal }),
            3 => (any::<Index>(), slot(), any::<bool>())
                .prop_map(|(stack, index, activate)| Op::AddToStack { stack, index, activate }),
            3 => (any::<Index>(), any::<Index>(), slot(), any::<bool>()).prop_map(
                |(pane, stack, index, activate)| Op::MoveToStack { pane, stack, index, activate }
            ),
            2 => any::<Index>().prop_map(|pane| Op::Close { pane }),
            1 => Just(Op::CloseFocused),
            2 => any::<Index>().prop_map(|pane| Op::Focus { pane }),
            1 => any::<Index>().prop_map(|pane| Op::Show { pane }),
            2 => (any::<Index>(), any::<Index>(), any::<u8>())
                .prop_map(|(pane, target, side)| Op::SplitOut { pane, target, side }),
            1 => (any::<Index>(), any::<Index>()).prop_map(|(pane, index)| Op::Reorder { pane, index }),
            1 => (any::<Index>(), any::<Index>()).prop_map(|(first, second)| Op::Swap { first, second }),
        ]
    }

    fn ops() -> impl Strategy<Value = Vec<Op>> {
        proptest::collection::vec(op(), 1..80)
    }

    fn stack_id(layout: &TileLayout, stack: &StackInfo) -> StackId {
        layout.stack_of(stack.active).unwrap().id()
    }

    fn apply(layout: &mut TileLayout, op: &Op) {
        let panes = layout.pane_ids();
        let stacks: Vec<StackId> = layout
            .stacks()
            .iter()
            .map(|stack| stack_id(layout, stack))
            .collect();
        let pane = |index: &Index| *index.get(&panes);
        let stack = |index: &Index| *index.get(&stacks);
        let slot = |index: &Option<Index>| index.map(|index| index.index(MAX_SLOT));
        match op {
            Op::Split {
                pane: target,
                horizontal,
            } => {
                let direction = if *horizontal {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                };
                layout.split_pane(pane(target), direction, 0.5);
            }
            Op::AddToStack {
                stack: target,
                index,
                activate,
            } => {
                layout.add_to_stack(stack(target), slot(index), *activate);
            }
            Op::MoveToStack {
                pane: moved,
                stack: target,
                index,
                activate,
            } => {
                layout.move_to_stack(pane(moved), stack(target), slot(index), *activate);
            }
            Op::Close { pane: target } => {
                layout.close_pane(pane(target));
            }
            Op::CloseFocused => {
                layout.close_focused();
            }
            Op::Focus { pane: target } => layout.focus_pane(pane(target)),
            Op::Show { pane: target } => layout.show(pane(target)),
            Op::SplitOut {
                pane: moved,
                target,
                side,
            } => {
                let side = [
                    NavDirection::Left,
                    NavDirection::Right,
                    NavDirection::Up,
                    NavDirection::Down,
                ][usize::from(*side % 4)];
                layout.split_out(pane(moved), pane(target), side);
            }
            Op::Reorder { pane: moved, index } => {
                layout.move_in_stack(pane(moved), index.index(MAX_SLOT));
            }
            Op::Swap { first, second } => {
                layout.swap_panes(pane(first), pane(second));
            }
        }
    }

    proptest! {
        /// Business rule: every terminal is in exactly one place and every tile
        /// shows something. A duplicated or lost pane leaks a terminal or draws
        /// it twice; an empty stack draws nothing and breaks closing.
        #[test]
        fn every_pane_is_in_exactly_one_nonempty_stack(ops in ops()) {
            let (mut layout, _) = TileLayout::new();
            for op in &ops {
                apply(&mut layout, op);
                let ids = layout.pane_ids();
                let unique: HashSet<PaneId> = ids.iter().copied().collect();
                prop_assert_eq!(unique.len(), ids.len(), "a pane appears twice");
                let stacks = layout.stacks();
                prop_assert!(stacks.iter().all(|stack| !stack.panes.is_empty()), "empty stack");
                let stacked: Vec<PaneId> =
                    stacks.iter().flat_map(|stack| stack.panes.iter().copied()).collect();
                prop_assert_eq!(stacked, ids);
            }
        }

        /// Business rule: keystrokes go to the pane on screen. Each stack shows
        /// one of its own panes, and the focused pane is the one its stack shows.
        #[test]
        fn focused_pane_is_shown_in_its_stack(ops in ops()) {
            let (mut layout, _) = TileLayout::new();
            for op in &ops {
                apply(&mut layout, op);
                let stacks = layout.stacks();
                prop_assert!(stacks.iter().all(|stack| stack.panes.contains(&stack.active)));
                let focused: Vec<&StackInfo> = stacks
                    .iter()
                    .filter(|stack| stack.panes.contains(&layout.focused()))
                    .collect();
                prop_assert_eq!(focused.len(), 1, "exactly one focused stack");
                prop_assert_eq!(focused[0].active, layout.focused());
                let visible: Vec<PaneId> = layout.panes(AREA).iter().map(|info| info.id).collect();
                let shown: Vec<PaneId> = stacks.iter().map(|stack| stack.active).collect();
                prop_assert_eq!(visible, shown);
            }
        }

        /// Business rule: moving a tab, by drag or by keyboard, never drops or
        /// duplicates a terminal.
        #[test]
        fn moving_tabs_preserves_the_set_of_panes(ops in ops()) {
            let (mut layout, _) = TileLayout::new();
            for op in &ops {
                let before: HashSet<PaneId> = layout.pane_ids().into_iter().collect();
                apply(&mut layout, op);
                if matches!(
                    op,
                    Op::MoveToStack { .. } | Op::Reorder { .. } | Op::Swap { .. } | Op::SplitOut { .. }
                ) {
                    let after: HashSet<PaneId> = layout.pane_ids().into_iter().collect();
                    prop_assert_eq!(after, before);
                }
            }
        }

        /// Business rule: a companion opened for a pane never covers that pane.
        /// An agent's browser or shell must not hide the agent's own
        /// conversation, so the chosen stack is another tile next to it.
        #[test]
        fn companions_open_in_a_neighbouring_tile(
            ops in ops(),
            caller in any::<Index>(),
            down in any::<bool>(),
        ) {
            let (mut layout, _) = TileLayout::new();
            for op in &ops {
                apply(&mut layout, op);
            }
            let caller = *caller.get(&layout.pane_ids());
            let toward = if down { NavDirection::Down } else { NavDirection::Right };
            let Some(target) = layout.companion_tile(caller, toward, AREA) else {
                return Ok(());
            };
            let target = layout.stack_of(target).unwrap().id();
            let own = layout.stack_of(caller).unwrap();
            prop_assert!(target != own.id(), "companion opens in the caller's tile");
            let shown = layout.panes(AREA);
            let rect = |stack: StackId| {
                shown
                    .iter()
                    .find(|info| layout.stack_of(info.id).unwrap().id() == stack)
                    .unwrap()
                    .rect
            };
            prop_assert!(rects_touch(rect(target), rect(own.id())), "companion tile is not adjacent");
        }

        /// Business rule: closing a tab in one tile never changes what another
        /// tile shows, and never leaves a broken tree behind.
        #[test]
        fn closing_keeps_the_tree_valid_and_other_stacks_unchanged(ops in ops()) {
            let (mut layout, _) = TileLayout::new();
            for op in &ops {
                let closed = match op {
                    Op::Close { pane } => Some(*pane.get(&layout.pane_ids())),
                    Op::CloseFocused => Some(layout.focused()),
                    _ => None,
                };
                let others: HashMap<StackId, PaneId> = layout
                    .stacks()
                    .into_iter()
                    .filter(|stack| closed.is_some_and(|pane| !stack.panes.contains(&pane)))
                    .map(|stack| (stack_id(&layout, &stack), stack.active))
                    .collect();
                apply(&mut layout, op);
                let Some(closed) = closed else { continue };
                let stacks = layout.stacks();
                prop_assert_eq!(layout.splits(AREA).len(), stacks.len() - 1, "orphan split");
                if layout.pane_count() > 1 || stacks[0].panes != [closed] {
                    prop_assert!(!layout.pane_ids().contains(&closed), "closed pane remains");
                }
                for stack in &stacks {
                    if let Some(active) = others.get(&stack_id(&layout, stack)) {
                        prop_assert_eq!(stack.active, *active, "another stack changed its tab");
                    }
                }
            }
        }
    }
}
