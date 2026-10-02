use anyhow::Context;
use wm_common::{WindowState, WmEvent};

use crate::{
  commands::container::{detach_container, set_focused_descendant},
  models::WindowContainer,
  traits::{CommonGetters, WindowGetters},
  wm_state::WmState,
};

#[allow(clippy::needless_pass_by_value)]
pub fn unmanage_window(
  window: WindowContainer,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;

  // Get container to switch focus to after the window has been removed.
  let focus_target = state.focus_target_after_removal(&window.clone());

  detach_container(window.clone().into())?;

  state.emit_event(WmEvent::WindowUnmanaged {
    unmanaged_id: window.id(),
    #[allow(clippy::cast_possible_wrap, clippy::unnecessary_cast)]
    unmanaged_handle: window.native().id().0 as isize,
  });

  // Reassign focus to suitable target.
  if let Some(focus_target) = focus_target {
    set_focused_descendant(&focus_target, None);
    state.pending_sync.queue_focus_change();
    state.unmanaged_or_minimized_timestamp =
      Some(std::time::Instant::now());
  }

  // The remaining tiling windows need to be redrawn if the window was
  // tiling, since removing a tiling window moves all the others.
  if window.state() == WindowState::Tiling {
    state.pending_sync.queue_container_to_redraw(workspace);
  }

  Ok(())
}
