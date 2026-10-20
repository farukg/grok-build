//! Tree navigation over a flat row list: visibility, cursor steps and bulk expand/collapse.
//!
//! Rows are index based and carry only a depth and an optional expanded flag. The parent of row `i` is the nearest
//! preceding row with a smaller depth; a row is visible iff every ancestor is an expanded branch (a leaf ancestor hides
//! its followers). Functions are pure and never mutate: callers apply the returned [`TreeStep`] / [`BulkChange`] to their
//! own state. A cursor outside the list yields [`TreeStep::Stay`] or an empty result.

/// One row of the flat list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: u8,
    /// `Some(expanded)` for a branch, `None` for a leaf.
    pub branch: Option<bool>,
}

impl TreeRow {
    pub const fn branch(depth: u8, expanded: bool) -> Self {
        Self {
            depth,
            branch: Some(expanded),
        }
    }

    pub const fn leaf(depth: u8) -> Self {
        Self {
            depth,
            branch: None,
        }
    }

    fn is_expanded_branch(self) -> bool {
        self.branch == Some(true)
    }
}

/// A navigation key, already mapped from the terminal event. Page keys carry the number of visible rows to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp(usize),
    PageDown(usize),
}

/// What the caller must do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeStep {
    Stay,
    MoveTo(usize),
    Expand(usize),
    Collapse(usize),
}

/// Which branches a bulk gesture targets, relative to the cursor row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkScope {
    /// Branches sharing the cursor row's parent, including the cursor row.
    Siblings,
    /// The cursor row and every branch below it.
    Subtree,
    /// Every branch of the list.
    All,
}

/// Whether a bulk gesture includes the row it starts from.
///
/// A plain bulk key (`+`, `-`, `*`) sweeps its whole scope. A modifier activation
/// (Shift/Ctrl+Enter or +click) acts on the rows around the cursor: the clicked row
/// keeps its own state so the gesture never doubles as the row's own action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAnchor {
    /// The scope covers the cursor row like any other.
    Include,
    /// The cursor row keeps its own state; only the other rows of the scope change.
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkDirection {
    Expand,
    Collapse,
}

/// Branches whose expanded flag must be set to the one `direction` names; branches already in that state are omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkChange {
    pub direction: BulkDirection,
    pub indices: Vec<usize>,
}

/// Indices of visible rows in order, in one pass.
pub fn visible_rows(rows: &[TreeRow]) -> Vec<usize> {
    let mut visible = Vec::new();
    let mut hidden_below: Option<u8> = None;
    for (index, row) in rows.iter().enumerate() {
        if let Some(depth) = hidden_below {
            if row.depth > depth {
                continue;
            }
            hidden_below = None;
        }
        visible.push(index);
        if !row.is_expanded_branch() {
            hidden_below = Some(row.depth);
        }
    }
    visible
}

/// The row itself when visible, else its outermost non-expanded ancestor (the nearest visible row above it).
/// Use it to repair a cursor after a collapse; `None` when `index` is out of range.
pub fn visible_anchor(rows: &[TreeRow], index: usize) -> Option<usize> {
    let mut need = rows.get(index)?.depth;
    let mut anchor = index;
    for (j, row) in rows.iter().enumerate().take(index).rev() {
        if need == 0 {
            break;
        }
        if row.depth < need {
            need = row.depth;
            if !row.is_expanded_branch() {
                anchor = j;
            }
        }
    }
    Some(anchor)
}

/// The step for `key` from `cursor`. A hidden cursor acts as its [`visible_anchor`] and is moved there when nothing else happens.
pub fn navigate(rows: &[TreeRow], cursor: usize, key: TreeKey) -> TreeStep {
    let Some(at) = visible_anchor(rows, cursor) else {
        return TreeStep::Stay;
    };
    match step_from_visible(rows, at, key) {
        TreeStep::Stay if at != cursor => TreeStep::MoveTo(at),
        step => step,
    }
}

/// Expand a collapsed branch or collapse an expanded one; `Stay` on a leaf or out of range.
pub fn toggle(rows: &[TreeRow], index: usize) -> TreeStep {
    match rows.get(index).and_then(|row| row.branch) {
        Some(true) => TreeStep::Collapse(index),
        Some(false) => TreeStep::Expand(index),
        None => TreeStep::Stay,
    }
}

/// Toggle-all: expand every collapsed targeted branch if any is collapsed, else collapse all targeted branches.
pub fn bulk_toggle(
    rows: &[TreeRow],
    cursor: usize,
    scope: BulkScope,
    anchor: BulkAnchor,
) -> BulkChange {
    let targets = targeted(rows, cursor, scope, anchor);
    let any_collapsed = targets
        .iter()
        .filter_map(|&i| rows.get(i))
        .any(|row| !row.is_expanded_branch());
    let direction = if any_collapsed {
        BulkDirection::Expand
    } else {
        BulkDirection::Collapse
    };
    changes_for(rows, targets, direction)
}

/// Expand or collapse every targeted branch (explicit `*`-style gesture).
pub fn bulk_set(
    rows: &[TreeRow],
    cursor: usize,
    scope: BulkScope,
    direction: BulkDirection,
    anchor: BulkAnchor,
) -> BulkChange {
    changes_for(rows, targeted(rows, cursor, scope, anchor), direction)
}

/// The scope's branches, minus the cursor row when the gesture leaves it alone.
fn targeted(rows: &[TreeRow], cursor: usize, scope: BulkScope, anchor: BulkAnchor) -> Vec<usize> {
    let mut targets = scope_targets(rows, cursor, scope);
    if anchor == BulkAnchor::Exclude {
        targets.retain(|&i| i != cursor);
    }
    targets
}

fn step_from_visible(rows: &[TreeRow], at: usize, key: TreeKey) -> TreeStep {
    match key {
        TreeKey::Up => move_to(at, previous_visible(rows, at)),
        TreeKey::Down => move_to(at, next_visible(rows, at)),
        TreeKey::Home => move_to(at, Some(0)),
        TreeKey::End => move_to(
            at,
            rows.len()
                .checked_sub(1)
                .and_then(|last| visible_anchor(rows, last)),
        ),
        TreeKey::PageUp(count) => page(rows, at, count, false),
        TreeKey::PageDown(count) => page(rows, at, count, true),
        TreeKey::Right => step_right(rows, at),
        TreeKey::Left => step_left(rows, at),
    }
}

fn move_to(from: usize, to: Option<usize>) -> TreeStep {
    match to {
        Some(to) if to != from => TreeStep::MoveTo(to),
        Some(_) | None => TreeStep::Stay,
    }
}

fn previous_visible(rows: &[TreeRow], at: usize) -> Option<usize> {
    visible_anchor(rows, at.checked_sub(1)?)
}

fn next_visible(rows: &[TreeRow], at: usize) -> Option<usize> {
    let row = *rows.get(at)?;
    let depth = row.depth;
    let mut next = at + 1;
    if !row.is_expanded_branch() {
        while rows.get(next).is_some_and(|row| row.depth > depth) {
            next += 1;
        }
    }
    (next < rows.len()).then_some(next)
}

fn page(rows: &[TreeRow], at: usize, count: usize, forward: bool) -> TreeStep {
    let visible = visible_rows(rows);
    let Ok(position) = visible.binary_search(&at) else {
        return TreeStep::Stay;
    };
    let target = if forward {
        position.saturating_add(count).min(visible.len() - 1)
    } else {
        position.saturating_sub(count)
    };
    move_to(at, visible.get(target).copied())
}

fn step_right(rows: &[TreeRow], at: usize) -> TreeStep {
    let Some(&row) = rows.get(at) else {
        return TreeStep::Stay;
    };
    match row.branch {
        Some(false) => TreeStep::Expand(at),
        Some(true) => match rows.get(at + 1) {
            Some(child) if child.depth > row.depth => TreeStep::MoveTo(at + 1),
            Some(_) | None => TreeStep::Stay,
        },
        None => TreeStep::Stay,
    }
}

fn step_left(rows: &[TreeRow], at: usize) -> TreeStep {
    let Some(&row) = rows.get(at) else {
        return TreeStep::Stay;
    };
    match row.branch {
        Some(true) => TreeStep::Collapse(at),
        Some(false) | None => {
            let parent = rows.iter().take(at).rposition(|r| r.depth < row.depth);
            move_to(at, parent)
        }
    }
}

/// Branch indices the scope covers; the cursor-relative scopes are empty for an out-of-range cursor.
fn scope_targets(rows: &[TreeRow], cursor: usize, scope: BulkScope) -> Vec<usize> {
    let branches = |range: std::ops::Range<usize>| -> Vec<usize> {
        rows.iter()
            .enumerate()
            .skip(range.start)
            .take(range.len())
            .filter(|(_, row)| row.branch.is_some())
            .map(|(i, _)| i)
            .collect()
    };
    match (scope, rows.get(cursor)) {
        (BulkScope::All, _) => branches(0..rows.len()),
        (BulkScope::Subtree, Some(row)) => {
            let end = rows
                .iter()
                .skip(cursor + 1)
                .position(|r| r.depth <= row.depth)
                .map_or(rows.len(), |offset| cursor + 1 + offset);
            branches(cursor..end)
        }
        (BulkScope::Siblings, Some(row)) => {
            // Rows between the parent and the next shallower row hold the cursor's siblings plus their descendants.
            let start = rows
                .iter()
                .take(cursor)
                .rposition(|r| r.depth < row.depth)
                .map_or(0, |parent| parent + 1);
            let end = rows
                .iter()
                .skip(cursor + 1)
                .position(|r| r.depth < row.depth)
                .map_or(rows.len(), |offset| cursor + 1 + offset);
            let mut siblings = branches(start..end);
            siblings.retain(|&i| rows.get(i).is_some_and(|r| r.depth == row.depth));
            siblings
        }
        (BulkScope::Subtree | BulkScope::Siblings, None) => Vec::new(),
    }
}

fn changes_for(rows: &[TreeRow], targets: Vec<usize>, direction: BulkDirection) -> BulkChange {
    let affected = match direction {
        BulkDirection::Expand => Some(false),
        BulkDirection::Collapse => Some(true),
    };
    let indices = targets
        .into_iter()
        .filter(|&i| rows.get(i).is_some_and(|r| r.branch == affected))
        .collect();
    BulkChange { direction, indices }
}

#[cfg(test)]
#[path = "tree_nav_tests.rs"]
mod tests;
