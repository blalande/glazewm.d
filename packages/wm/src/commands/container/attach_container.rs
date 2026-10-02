use anyhow::bail;

use crate::{models::Container, traits::CommonGetters};

/// Inserts a child container at the specified index.
///
/// Appends the child if no index is given. Callers are responsible for
/// queueing the parent workspace for redraw when attaching a tiling
/// window, since any change to the tiling windows moves all of them.
pub fn attach_container(
  child: &Container,
  target_parent: &Container,
  target_index: Option<usize>,
) -> anyhow::Result<()> {
  if !child.is_detached() {
    bail!("Cannot attach an already attached container.");
  }

  if let Some(target_index) = target_index {
    // Ensure target index is within the bounds of the parent's children.
    let target_index = target_index.clamp(0, target_parent.child_count());

    // Insert the child at the specified index.
    target_parent
      .borrow_children_mut()
      .insert(target_index, child.clone());
  } else {
    target_parent.borrow_children_mut().push_back(child.clone());
  }

  target_parent
    .borrow_child_focus_order_mut()
    .push_back(child.id());

  *child.borrow_parent_mut() = Some(target_parent.clone());

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    commands::container::detach_container,
    models::{TilingWindow, Workspace},
  };

  #[test]
  fn attach_clamps_the_index_and_detach_removes_the_child() {
    let workspace = Workspace::mock().call();
    let workspace_container: Container = workspace.clone().into();

    let first = TilingWindow::mock().call();
    let second = TilingWindow::mock().call();
    attach_container(&first.clone().into(), &workspace_container, Some(5))
      .unwrap();
    attach_container(
      &second.clone().into(),
      &workspace_container,
      Some(0),
    )
    .unwrap();

    assert_eq!(second.index(), 0);
    assert_eq!(first.index(), 1);
    assert!(attach_container(
      &first.clone().into(),
      &workspace_container,
      None
    )
    .is_err());

    detach_container(second.clone().into()).unwrap();

    assert!(second.is_detached());
    assert_eq!(workspace.child_count(), 1);
    assert_eq!(workspace.borrow_child_focus_order().len(), 1);
  }
}
