use anyhow::{bail, Context};
use wm_common::VecDequeExt;

use super::{attach_container, detach_container};
use crate::{models::Container, traits::CommonGetters};

/// Replaces a container at the specified index.
///
/// The replaced container will be detached from the tree. The replacement
/// takes over the focus position of the replaced container.
pub fn replace_container(
  replacement_container: &Container,
  target_parent: &Container,
  target_index: usize,
) -> anyhow::Result<()> {
  if !replacement_container.is_detached() {
    bail!(
      "Cannot use an already attached container as replacement container."
    );
  }

  let container_to_replace = target_parent
    .children()
    .get(target_index)
    .cloned()
    .with_context(|| format!("No container at index {target_index}."))?;

  let focus_index = container_to_replace.focus_index();

  // We can't attach first before detaching, because detaching removes
  // child based on ID and both containers might have the same ID.
  detach_container(container_to_replace)?;

  attach_container(
    replacement_container,
    target_parent,
    Some(target_index),
  )?;

  // Shift to the correct focus index.
  target_parent
    .borrow_child_focus_order_mut()
    .shift_to_index(focus_index, replacement_container.id());

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    commands::container::set_focused_descendant,
    models::{TilingWindow, Workspace},
  };

  #[test]
  fn replaces_a_container_and_keeps_its_focus_position() {
    let windows = (0..3)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();
    let workspace_container: Container = workspace.clone().into();

    set_focused_descendant(&windows[1].clone().into(), None);

    let replacement = TilingWindow::mock().call();
    replace_container(
      &replacement.clone().into(),
      &workspace_container,
      1,
    )
    .unwrap();

    assert!(windows[1].is_detached());
    assert_eq!(replacement.index(), 1);
    assert_eq!(replacement.focus_index(), 0);
    assert_eq!(workspace.child_count(), 3);
  }

  #[test]
  fn replace_rejects_an_attached_replacement() {
    let windows = (0..2)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();

    assert!(replace_container(
      &windows[1].clone().into(),
      &workspace.into(),
      0
    )
    .is_err());
  }
}
