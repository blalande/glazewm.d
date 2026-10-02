use anyhow::Context;
use wm_common::WindowState;

use crate::{
  commands::container::detach_container,
  models::WindowContainer,
  traits::{CommonGetters, WindowGetters},
  wm_state::WmState,
};

#[allow(clippy::needless_pass_by_value)]
pub fn ignore_window(
  window: WindowContainer,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;

  state.ignored_windows.push(window.native().clone());
  detach_container(window.clone().into())?;

  // The remaining tiling windows need to be redrawn if the window was
  // tiling, since removing a tiling window moves all the others.
  if window.state() == WindowState::Tiling {
    state.pending_sync.queue_container_to_redraw(workspace);
  }

  Ok(())
}
