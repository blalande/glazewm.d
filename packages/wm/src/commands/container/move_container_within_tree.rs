use anyhow::{bail, Context};
use wm_common::{VecDequeExt, WmEvent};

use super::{attach_container, detach_container, set_focused_descendant};
use crate::{
  models::{Container, TilingWindow},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Moves a container to a new parent in the tree, at the specified index.
///
/// Within the same parent, the container is shifted to the target index.
/// Otherwise, it is detached from its current parent and attached to the
/// target parent, which keeps the focus order of both subtrees consistent.
///
/// For a tiling window, the callers are responsible for queueing both the
/// original and the target workspace for redraw.
pub fn move_container_within_tree(
  container_to_move: &Container,
  target_parent: &Container,
  target_index: usize,
  state: &WmState,
) -> anyhow::Result<()> {
  // If the container is already a child of the target parent, then shift
  // it to the target index.
  if container_to_move.parent().context("No parent.")? == *target_parent {
    target_parent
      .borrow_children_mut()
      .shift_to_index(target_index, container_to_move.clone());

    emit_if_focused(container_to_move, state)?;

    return Ok(());
  }

  // Get lowest common ancestor (LCA) between the original parent and
  // `target_parent`. The original parent can never be an ancestor of the
  // target parent (or vice versa), so the LCA is a distinct container.
  let lowest_common_ancestor =
    lowest_common_ancestor(container_to_move, target_parent)
      .context("No common ancestor between containers.")?;

  // Get ancestor of `container_to_move` that is a direct child of the LCA.
  let container_to_move_ancestor = container_to_move
    .self_and_ancestors()
    .find(|ancestor| {
      ancestor.parent() == Some(lowest_common_ancestor.clone())
    })
    .context("Failed to get ancestor of container to move.")?;

  // Likewise, get ancestor of `target_parent` that is a direct child of
  // the LCA.
  let target_parent_ancestor = target_parent
    .self_and_ancestors()
    .find(|ancestor| {
      ancestor.parent() == Some(lowest_common_ancestor.clone())
    })
    .context("Failed to get ancestor of target parent.")?;

  // Get whether the container is the focused descendant in its original
  // subtree from the LCA.
  let is_focused_descendant = *container_to_move
    == container_to_move_ancestor
    || container_to_move
      .has_focus(Some(container_to_move_ancestor.clone()));

  // Get whether the ancestor of `container_to_move` appears before
  // `target_parent`'s ancestor in the child focus order of the LCA.
  let original_focus_index = container_to_move_ancestor.focus_index();
  let is_subtree_focused =
    original_focus_index < target_parent_ancestor.focus_index();

  detach_container(container_to_move.clone())?;
  attach_container(
    &container_to_move.clone(),
    &target_parent.clone(),
    Some(target_index),
  )?;

  // Set `container_to_move` as focused descendant within target subtree if
  // its original subtree had focus more recently (even if the container is
  // not the last focused within that subtree).
  if is_subtree_focused {
    set_focused_descendant(
      container_to_move,
      Some(&target_parent_ancestor),
    );
  }

  // If the focused descendant is moved to the targets subtree, then the
  // target's ancestor should be placed before the original ancestor in
  // LCA's child focus order.
  if is_focused_descendant && is_subtree_focused {
    lowest_common_ancestor
      .borrow_child_focus_order_mut()
      .shift_to_index(original_focus_index, target_parent_ancestor.id());
  }

  emit_if_focused(container_to_move, state)
}

/// Swaps the positions of two tiling windows of the same workspace.
///
/// The callers are responsible for queueing the workspace for redraw.
pub fn swap_tiling_windows(
  window_a: &TilingWindow,
  window_b: &TilingWindow,
  state: &WmState,
) -> anyhow::Result<()> {
  let parent = window_a.parent().context("No parent.")?;

  if window_b.parent().as_ref() != Some(&parent) {
    bail!("Cannot swap tiling windows of different workspaces.");
  }

  {
    let mut children = parent.borrow_children_mut();

    let index_of = |window: &TilingWindow| {
      children.iter().position(|child| child.id() == window.id())
    };

    if let (Some(index_a), Some(index_b)) =
      (index_of(window_a), index_of(window_b))
    {
      children.swap(index_a, index_b);
    }
  }

  emit_if_focused(&window_a.as_container(), state)?;
  emit_if_focused(&window_b.as_container(), state)
}

/// Emits a `FocusedContainerMoved` event if the container has focus.
fn emit_if_focused(
  container: &Container,
  state: &WmState,
) -> anyhow::Result<()> {
  if container.has_focus(None) {
    state.emit_event(WmEvent::FocusedContainerMoved {
      focused_container: container.to_dto()?,
    });
  }

  Ok(())
}

/// Gets the lowest container in the tree that has both `container_a` and
/// `container_b` as descendants.
pub fn lowest_common_ancestor(
  container_a: &Container,
  container_b: &Container,
) -> Option<Container> {
  let mut ancestor_a = Some(container_a.clone());

  // Traverse upwards from container A.
  while let Some(current_ancestor_a) = ancestor_a {
    let mut ancestor_b = Some(container_b.clone());

    // Traverse upwards from container B.
    while let Some(current_ancestor_b) = ancestor_b {
      if current_ancestor_a == current_ancestor_b {
        return Some(current_ancestor_a);
      }

      ancestor_b = current_ancestor_b.parent();
    }

    ancestor_a = current_ancestor_a.parent();
  }

  None
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::models::{Monitor, Workspace};

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  fn tiling_ids(workspace: &Workspace) -> Vec<uuid::Uuid> {
    workspace
      .tiling_children()
      .map(|window| window.id())
      .collect()
  }

  #[test]
  fn swaps_two_tiling_windows() {
    let state = mock_state();
    let windows = (0..3)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    swap_tiling_windows(&windows[0], &windows[2], &state).unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[2].id(), windows[1].id(), windows[0].id()]
    );

    // The focus order is unaffected by a swap.
    assert!(windows[0].has_focus(None));
    assert!(!windows[2].has_focus(None));
  }

  #[test]
  fn swap_rejects_windows_of_different_workspaces() {
    let state = mock_state();
    let window_a = TilingWindow::mock().call();
    let window_b = TilingWindow::mock().call();
    let _workspace_a = Workspace::mock()
      .tiling_windows(vec![window_a.clone()])
      .call();
    let _workspace_b = Workspace::mock()
      .tiling_windows(vec![window_b.clone()])
      .call();

    assert!(swap_tiling_windows(&window_a, &window_b, &state).is_err());
  }

  #[test]
  fn shifts_within_the_same_workspace() {
    let state = mock_state();
    let windows = (0..3)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    move_container_within_tree(
      &windows[2].clone().into(),
      &workspace.clone().into(),
      0,
      &state,
    )
    .unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[2].id(), windows[0].id(), windows[1].id()]
    );
  }

  #[test]
  fn moves_to_another_workspace_at_the_target_index() {
    let state = mock_state();
    let moved = TilingWindow::mock().call();
    let origin =
      Workspace::mock().tiling_windows(vec![moved.clone()]).call();
    let others = (0..2)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let target = Workspace::mock()
      .name("2".to_string())
      .tiling_windows(others.clone())
      .call();

    let monitor_a =
      Monitor::mock().workspaces(vec![origin.clone()]).call();
    let monitor_b =
      Monitor::mock().workspaces(vec![target.clone()]).call();
    for monitor in [monitor_a, monitor_b] {
      crate::commands::container::attach_container(
        &monitor.into(),
        &state.root_container.clone().into(),
        None,
      )
      .unwrap();
    }

    move_container_within_tree(
      &moved.clone().into(),
      &target.clone().into(),
      1,
      &state,
    )
    .unwrap();

    assert!(!origin.has_children());
    assert_eq!(
      tiling_ids(&target),
      vec![others[0].id(), moved.id(), others[1].id()]
    );
    assert_eq!(moved.workspace(), Some(target));
  }
}
