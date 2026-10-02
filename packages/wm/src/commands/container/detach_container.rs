use anyhow::Context;

use crate::{models::Container, traits::CommonGetters};

/// Removes a container from the tree.
///
/// Callers are responsible for queueing the former parent workspace for
/// redraw when detaching a tiling window, since any change to the tiling
/// windows moves all of them.
#[allow(clippy::needless_pass_by_value)]
pub fn detach_container(child_to_remove: Container) -> anyhow::Result<()> {
  let parent = child_to_remove.parent().context("No parent.")?;

  parent
    .borrow_children_mut()
    .retain(|c| c.id() != child_to_remove.id());

  parent
    .borrow_child_focus_order_mut()
    .retain(|id| *id != child_to_remove.id());

  *child_to_remove.borrow_parent_mut() = None;

  Ok(())
}
