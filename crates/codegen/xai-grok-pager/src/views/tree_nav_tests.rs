use super::*;

/// Index: row
///  0 turn (expanded)
///  1   group (expanded)
///  2     entry
///  3     entry
///  4   group (collapsed)
///  5     entry            hidden
///  6     entry            hidden
///  7   entry
///  8 turn (collapsed)
///  9   group (expanded)   hidden
/// 10     entry            hidden
/// 11 turn (expanded, no children)
///
/// Visible: 0 1 2 3 4 7 8 11.
fn sample() -> Vec<TreeRow> {
    vec![
        TreeRow::branch(0, true),
        TreeRow::branch(1, true),
        TreeRow::leaf(2),
        TreeRow::leaf(2),
        TreeRow::branch(1, false),
        TreeRow::leaf(2),
        TreeRow::leaf(2),
        TreeRow::leaf(1),
        TreeRow::branch(0, false),
        TreeRow::branch(1, true),
        TreeRow::leaf(2),
        TreeRow::branch(0, true),
    ]
}

fn sample_prefix(len: usize) -> Vec<TreeRow> {
    sample().into_iter().take(len).collect()
}

fn set_expanded(rows: &mut [TreeRow], index: usize, expanded: bool) {
    if let Some(row) = rows.get_mut(index) {
        row.branch = Some(expanded);
    }
}

fn walk(rows: &[TreeRow], start: usize, key: TreeKey) -> Vec<usize> {
    let mut cursor = start;
    let mut seen = Vec::new();
    while let TreeStep::MoveTo(next) = navigate(rows, cursor, key) {
        seen.push(next);
        cursor = next;
    }
    seen
}

#[test]
fn visible_rows_hide_descendants_of_collapsed_branches() {
    assert_eq!(visible_rows(&sample()), [0, 1, 2, 3, 4, 7, 8, 11]);
}

#[test]
fn visible_rows_of_fully_expanded_tree_is_everything() {
    let rows = [
        TreeRow::branch(0, true),
        TreeRow::branch(1, true),
        TreeRow::leaf(2),
        TreeRow::leaf(1),
    ];
    assert_eq!(visible_rows(&rows), [0, 1, 2, 3]);
}

#[test]
fn visible_rows_hide_followers_of_a_leaf_ancestor() {
    let rows = [TreeRow::leaf(0), TreeRow::leaf(1), TreeRow::leaf(0)];
    assert_eq!(visible_rows(&rows), [0, 2]);
}

#[test]
fn down_skips_hidden_rows() {
    assert_eq!(walk(&sample(), 0, TreeKey::Down), [1, 2, 3, 4, 7, 8, 11]);
}

#[test]
fn up_skips_hidden_rows() {
    assert_eq!(walk(&sample(), 11, TreeKey::Up), [8, 7, 4, 3, 2, 1, 0]);
}

#[test]
fn up_and_down_clamp_at_the_ends() {
    let rows = sample();
    assert_eq!(navigate(&rows, 0, TreeKey::Up), TreeStep::Stay);
    assert_eq!(navigate(&rows, 11, TreeKey::Down), TreeStep::Stay);
}

#[test]
fn down_from_a_collapsed_branch_skips_its_subtree() {
    let rows = sample();
    assert_eq!(navigate(&rows, 4, TreeKey::Down), TreeStep::MoveTo(7));
    assert_eq!(navigate(&rows, 8, TreeKey::Down), TreeStep::MoveTo(11));
}

#[test]
fn home_and_end_jump_to_first_and_last_visible_row() {
    let rows = sample();
    assert_eq!(navigate(&rows, 7, TreeKey::Home), TreeStep::MoveTo(0));
    assert_eq!(navigate(&rows, 0, TreeKey::Home), TreeStep::Stay);
    assert_eq!(navigate(&rows, 0, TreeKey::End), TreeStep::MoveTo(11));
    assert_eq!(navigate(&rows, 11, TreeKey::End), TreeStep::Stay);
}

#[test]
fn end_lands_on_the_collapsed_branch_hiding_the_last_row() {
    let rows = sample_prefix(11);
    assert_eq!(navigate(&rows, 0, TreeKey::End), TreeStep::MoveTo(8));
}

#[test]
fn page_keys_move_over_visible_rows_only() {
    let rows = sample();
    assert_eq!(
        navigate(&rows, 0, TreeKey::PageDown(3)),
        TreeStep::MoveTo(3)
    );
    assert_eq!(
        navigate(&rows, 3, TreeKey::PageDown(3)),
        TreeStep::MoveTo(8)
    );
    assert_eq!(navigate(&rows, 7, TreeKey::PageUp(2)), TreeStep::MoveTo(3));
}

#[test]
fn page_keys_clamp_at_the_ends() {
    let rows = sample();
    assert_eq!(
        navigate(&rows, 4, TreeKey::PageDown(100)),
        TreeStep::MoveTo(11)
    );
    assert_eq!(
        navigate(&rows, 7, TreeKey::PageUp(100)),
        TreeStep::MoveTo(0)
    );
    assert_eq!(navigate(&rows, 11, TreeKey::PageDown(5)), TreeStep::Stay);
    assert_eq!(navigate(&rows, 0, TreeKey::PageUp(5)), TreeStep::Stay);
}

#[test]
fn page_by_zero_stays() {
    assert_eq!(navigate(&sample(), 3, TreeKey::PageDown(0)), TreeStep::Stay);
}

#[test]
fn right_on_collapsed_branch_expands_it() {
    let rows = sample();
    assert_eq!(navigate(&rows, 4, TreeKey::Right), TreeStep::Expand(4));
    assert_eq!(navigate(&rows, 8, TreeKey::Right), TreeStep::Expand(8));
}

#[test]
fn right_on_expanded_branch_moves_to_first_child() {
    let rows = sample();
    assert_eq!(navigate(&rows, 0, TreeKey::Right), TreeStep::MoveTo(1));
    assert_eq!(navigate(&rows, 1, TreeKey::Right), TreeStep::MoveTo(2));
}

#[test]
fn right_on_expanded_branch_without_children_stays() {
    let rows = sample();
    assert_eq!(navigate(&rows, 11, TreeKey::Right), TreeStep::Stay);
    let sibling_follows = [TreeRow::branch(0, true), TreeRow::branch(0, false)];
    assert_eq!(
        navigate(&sibling_follows, 0, TreeKey::Right),
        TreeStep::Stay
    );
}

#[test]
fn right_on_leaf_stays() {
    assert_eq!(navigate(&sample(), 2, TreeKey::Right), TreeStep::Stay);
}

#[test]
fn left_on_expanded_branch_collapses_it() {
    let rows = sample();
    assert_eq!(navigate(&rows, 1, TreeKey::Left), TreeStep::Collapse(1));
    assert_eq!(navigate(&rows, 0, TreeKey::Left), TreeStep::Collapse(0));
}

#[test]
fn left_on_leaf_moves_to_parent() {
    let rows = sample();
    assert_eq!(navigate(&rows, 2, TreeKey::Left), TreeStep::MoveTo(1));
    assert_eq!(navigate(&rows, 3, TreeKey::Left), TreeStep::MoveTo(1));
    assert_eq!(navigate(&rows, 7, TreeKey::Left), TreeStep::MoveTo(0));
}

#[test]
fn left_on_collapsed_branch_moves_to_parent() {
    assert_eq!(navigate(&sample(), 4, TreeKey::Left), TreeStep::MoveTo(0));
}

#[test]
fn left_without_parent_stays() {
    let rows = sample();
    assert_eq!(navigate(&rows, 8, TreeKey::Left), TreeStep::Stay);
    assert_eq!(
        navigate(&[TreeRow::leaf(0)], 0, TreeKey::Left),
        TreeStep::Stay
    );
}

#[test]
fn hidden_cursor_navigates_from_its_visible_ancestor() {
    let rows = sample();
    assert_eq!(navigate(&rows, 5, TreeKey::Down), TreeStep::MoveTo(7));
    assert_eq!(navigate(&rows, 5, TreeKey::Right), TreeStep::Expand(4));
    assert_eq!(navigate(&rows, 5, TreeKey::Left), TreeStep::MoveTo(0));
    assert_eq!(navigate(&rows, 10, TreeKey::End), TreeStep::MoveTo(11));
}

#[test]
fn hidden_cursor_without_other_movement_snaps_to_its_anchor() {
    let rows = sample_prefix(11);
    assert_eq!(navigate(&rows, 10, TreeKey::End), TreeStep::MoveTo(8));
}

#[test]
fn toggle_flips_branches_and_ignores_leaves() {
    let rows = sample();
    assert_eq!(toggle(&rows, 1), TreeStep::Collapse(1));
    assert_eq!(toggle(&rows, 4), TreeStep::Expand(4));
    assert_eq!(toggle(&rows, 2), TreeStep::Stay);
    assert_eq!(toggle(&rows, 99), TreeStep::Stay);
}

#[test]
fn siblings_target_branches_sharing_the_parent() {
    let rows = sample();
    let from_group = bulk_set(
        &rows,
        1,
        BulkScope::Siblings,
        BulkDirection::Expand,
        BulkAnchor::Include,
    );
    assert_eq!(from_group.indices, [4]);
    let from_other_group = bulk_set(
        &rows,
        4,
        BulkScope::Siblings,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(from_other_group.indices, [1]);
    let turns = bulk_set(
        &rows,
        0,
        BulkScope::Siblings,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(turns.indices, [0, 11]);
    let nested = bulk_set(
        &rows,
        9,
        BulkScope::Siblings,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(nested.indices, [9]);
}

#[test]
fn subtree_targets_cursor_and_branch_descendants() {
    let rows = sample();
    let turn = bulk_set(
        &rows,
        0,
        BulkScope::Subtree,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(turn.indices, [0, 1]);
    let turn_expand = bulk_set(
        &rows,
        0,
        BulkScope::Subtree,
        BulkDirection::Expand,
        BulkAnchor::Include,
    );
    assert_eq!(turn_expand.indices, [4]);
    let collapsed_turn = bulk_set(
        &rows,
        8,
        BulkScope::Subtree,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(collapsed_turn.indices, [9]);
    let leaf = bulk_set(
        &rows,
        2,
        BulkScope::Subtree,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert!(leaf.indices.is_empty());
}

#[test]
fn all_targets_every_branch() {
    let rows = sample();
    let collapse = bulk_set(
        &rows,
        2,
        BulkScope::All,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(collapse.indices, [0, 1, 9, 11]);
    let expand = bulk_set(
        &rows,
        2,
        BulkScope::All,
        BulkDirection::Expand,
        BulkAnchor::Include,
    );
    assert_eq!(expand.indices, [4, 8]);
}

#[test]
fn excluded_anchor_keeps_the_cursor_row_out_of_every_scope() {
    let rows = sample();
    let turns = bulk_set(
        &rows,
        0,
        BulkScope::Siblings,
        BulkDirection::Collapse,
        BulkAnchor::Exclude,
    );
    assert_eq!(turns.indices, [11], "the cursor turn keeps its state");
    let below = bulk_set(
        &rows,
        0,
        BulkScope::Subtree,
        BulkDirection::Collapse,
        BulkAnchor::Exclude,
    );
    assert_eq!(
        below.indices,
        [1],
        "the subtree reaches only what is below the cursor row"
    );
    let all = bulk_set(
        &rows,
        8,
        BulkScope::All,
        BulkDirection::Collapse,
        BulkAnchor::Exclude,
    );
    assert_eq!(
        all.indices,
        [0, 1, 9, 11],
        "cursor 8 is the only branch left out"
    );
    // The direction heuristic ignores the anchor too: 8 is collapsed, but 9 under it is open.
    let subtree = bulk_toggle(&rows, 8, BulkScope::Subtree, BulkAnchor::Exclude);
    assert_eq!(subtree.direction, BulkDirection::Collapse);
    assert_eq!(subtree.indices, [9]);
    let only_sibling = bulk_set(
        &rows,
        9,
        BulkScope::Siblings,
        BulkDirection::Collapse,
        BulkAnchor::Exclude,
    );
    assert!(
        only_sibling.indices.is_empty(),
        "a lone branch stays untouched"
    );
}

#[test]
fn toggle_all_expands_when_any_target_is_collapsed() {
    let rows = sample();
    let all = bulk_toggle(&rows, 0, BulkScope::All, BulkAnchor::Include);
    assert_eq!(all.direction, BulkDirection::Expand);
    assert_eq!(all.indices, [4, 8]);
    let siblings = bulk_toggle(&rows, 1, BulkScope::Siblings, BulkAnchor::Include);
    assert_eq!(siblings.direction, BulkDirection::Expand);
    assert_eq!(siblings.indices, [4]);
    let subtree = bulk_toggle(&rows, 8, BulkScope::Subtree, BulkAnchor::Include);
    assert_eq!(subtree.direction, BulkDirection::Expand);
    assert_eq!(subtree.indices, [8]);
}

#[test]
fn toggle_all_collapses_when_every_target_is_expanded() {
    let rows = [
        TreeRow::branch(0, true),
        TreeRow::branch(1, true),
        TreeRow::leaf(2),
        TreeRow::branch(1, true),
        TreeRow::leaf(2),
    ];
    let all = bulk_toggle(&rows, 0, BulkScope::All, BulkAnchor::Include);
    assert_eq!(all.direction, BulkDirection::Collapse);
    assert_eq!(all.indices, [0, 1, 3]);
    let siblings = bulk_toggle(&rows, 1, BulkScope::Siblings, BulkAnchor::Include);
    assert_eq!(siblings.direction, BulkDirection::Collapse);
    assert_eq!(siblings.indices, [1, 3]);
    let subtree = bulk_toggle(&rows, 1, BulkScope::Subtree, BulkAnchor::Include);
    assert_eq!(subtree.direction, BulkDirection::Collapse);
    assert_eq!(subtree.indices, [1]);
}

#[test]
fn toggle_all_flips_back_after_applying_the_change() {
    let mut rows = sample();
    let first = bulk_toggle(&rows, 0, BulkScope::All, BulkAnchor::Include);
    assert_eq!(first.direction, BulkDirection::Expand);
    for index in first.indices {
        set_expanded(&mut rows, index, true);
    }
    let second = bulk_toggle(&rows, 0, BulkScope::All, BulkAnchor::Include);
    assert_eq!(second.direction, BulkDirection::Collapse);
    assert_eq!(second.indices, [0, 1, 4, 8, 9, 11]);
}

#[test]
fn visible_anchor_of_a_visible_row_is_itself() {
    let rows = sample();
    for index in [0, 1, 2, 3, 4, 7, 8, 11] {
        assert_eq!(visible_anchor(&rows, index), Some(index));
    }
}

#[test]
fn visible_anchor_of_a_hidden_row_is_its_collapsed_ancestor() {
    let rows = sample();
    assert_eq!(visible_anchor(&rows, 5), Some(4));
    assert_eq!(visible_anchor(&rows, 6), Some(4));
    assert_eq!(visible_anchor(&rows, 9), Some(8));
    assert_eq!(visible_anchor(&rows, 10), Some(8));
}

#[test]
fn visible_anchor_prefers_the_outermost_collapsed_ancestor() {
    let mut rows = sample();
    set_expanded(&mut rows, 0, false);
    set_expanded(&mut rows, 1, false);
    assert_eq!(visible_anchor(&rows, 3), Some(0));
    assert_eq!(visible_anchor(&rows, 2), Some(0));
}

#[test]
fn cursor_repair_after_bulk_collapse() {
    let mut rows = sample();
    let change = bulk_set(
        &rows,
        0,
        BulkScope::Subtree,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(change.indices, [0, 1]);
    for index in change.indices {
        set_expanded(&mut rows, index, false);
    }
    assert_eq!(visible_rows(&rows), [0, 8, 11]);
    assert_eq!(visible_anchor(&rows, 3), Some(0));
    assert_eq!(visible_anchor(&rows, 7), Some(0));
}

#[test]
fn empty_list_is_inert() {
    let rows: [TreeRow; 0] = [];
    assert!(visible_rows(&rows).is_empty());
    assert_eq!(visible_anchor(&rows, 0), None);
    assert_eq!(navigate(&rows, 0, TreeKey::Down), TreeStep::Stay);
    assert_eq!(navigate(&rows, 0, TreeKey::End), TreeStep::Stay);
    assert_eq!(toggle(&rows, 0), TreeStep::Stay);
    let change = bulk_toggle(&rows, 0, BulkScope::All, BulkAnchor::Include);
    assert!(change.indices.is_empty());
}

#[test]
fn cursor_beyond_the_end_is_handled() {
    let rows = sample();
    assert_eq!(visible_anchor(&rows, 12), None);
    assert_eq!(navigate(&rows, 12, TreeKey::Up), TreeStep::Stay);
    assert_eq!(navigate(&rows, 99, TreeKey::Home), TreeStep::Stay);
    assert!(
        bulk_toggle(&rows, 12, BulkScope::Siblings, BulkAnchor::Include)
            .indices
            .is_empty()
    );
    assert!(
        bulk_toggle(&rows, 12, BulkScope::Subtree, BulkAnchor::Include)
            .indices
            .is_empty()
    );
    assert_eq!(
        bulk_set(
            &rows,
            12,
            BulkScope::All,
            BulkDirection::Expand,
            BulkAnchor::Include
        )
        .indices,
        [4, 8]
    );
}

/// 1000 turns of 100 rows (turn + 99 leaves); even turns expanded, odd turns collapsed.
fn large() -> Vec<TreeRow> {
    let mut rows = Vec::with_capacity(100_000);
    for turn in 0..1000 {
        rows.push(TreeRow::branch(0, turn % 2 == 0));
        rows.extend((0..99).map(|_| TreeRow::leaf(1)));
    }
    rows
}

#[test]
fn large_list_is_handled_in_single_calls() {
    let rows = large();
    assert_eq!(rows.len(), 100_000);

    let visible = visible_rows(&rows);
    assert_eq!(visible.len(), 50_500);
    assert_eq!(visible.get(..3), Some(&[0, 1, 2][..]));
    assert_eq!(visible.get(99..102), Some(&[99, 100, 200][..]));

    assert_eq!(navigate(&rows, 0, TreeKey::End), TreeStep::MoveTo(99_900));
    assert_eq!(
        navigate(&rows, 99_999, TreeKey::Up),
        TreeStep::MoveTo(99_899)
    );
    assert_eq!(navigate(&rows, 100, TreeKey::Down), TreeStep::MoveTo(200));
    assert_eq!(navigate(&rows, 99, TreeKey::Down), TreeStep::MoveTo(100));
    assert_eq!(
        navigate(&rows, 0, TreeKey::PageDown(50_000)),
        TreeStep::MoveTo(99_005)
    );
    assert_eq!(visible_anchor(&rows, 99_999), Some(99_900));

    let expand = bulk_toggle(&rows, 0, BulkScope::All, BulkAnchor::Include);
    assert_eq!(expand.direction, BulkDirection::Expand);
    assert_eq!(expand.indices.len(), 500);
    assert_eq!(expand.indices.get(..2), Some(&[100, 300][..]));
    assert_eq!(expand.indices.get(499), Some(&99_900));

    let siblings = bulk_set(
        &rows,
        200,
        BulkScope::Siblings,
        BulkDirection::Collapse,
        BulkAnchor::Include,
    );
    assert_eq!(siblings.indices.len(), 500);
    assert_eq!(siblings.indices.get(..2), Some(&[0, 200][..]));
}
