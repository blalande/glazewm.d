use anyhow::Context;
use tracing::{info, warn};
use wm_common::WindowState;

use crate::{
  commands::container::{move_container_within_tree, replace_container},
  models::WindowContainer,
  traits::{CommonGetters, WindowGetters},
  wm_state::WmState,
};

/// Updates the state of a window.
///
/// Adds the window for redraw if there is a state change.
///
/// Returns the window after the state change.
pub fn update_window_state(
  window: WindowContainer,
  target_state: WindowState,
  state: &mut WmState,
) -> anyhow::Result<WindowContainer> {
  if window.state() == target_state {
    return Ok(window);
  }

  info!("Updating window state: {:?}.", target_state);

  match target_state {
    WindowState::Tiling => set_tiling(&window, state),
    _ => set_non_tiling(window, target_state, state),
  }
}

/// Updates the state of a window to be `WindowState::Tiling`.
fn set_tiling(
  window: &WindowContainer,
  state: &mut WmState,
) -> anyhow::Result<WindowContainer> {
  let window = window
    .as_non_tiling_window()
    .context("Invalid window state.")?
    .clone();

  let workspace =
    window.workspace().context("Window has no workspace.")?;

  // Insert the window after the last focused tiling window of the
  // workspace, or at the end if there is none.
  let target_index = workspace.index_after_focused_tiling_window();
  let tiling_window = window.to_tiling();

  // Replace the original window with the created tiling window.
  replace_container(
    &tiling_window.clone().into(),
    &window.parent().context("No parent.")?,
    window.index(),
  )?;

  move_container_within_tree(
    &tiling_window.clone().into(),
    &workspace.clone().into(),
    target_index,
    state,
  )?;

  // The whole workspace needs to be redrawn, since adding a tiling window
  // moves all the others.
  state
    .pending_sync
    .queue_container_to_redraw(workspace.clone())
    .queue_workspace_to_reorder(workspace);

  Ok(tiling_window.into())
}

/// Updates the state of a window to be either `WindowState::Floating`,
/// `WindowState::Fullscreen`, or `WindowState::Minimized`.
fn set_non_tiling(
  window: WindowContainer,
  target_state: WindowState,
  state: &mut WmState,
) -> anyhow::Result<WindowContainer> {
  // A window can only be updated to a minimized state if it is
  // natively minimized.
  // TODO: Consider doing the same for maximized and fullscreen states.
  if target_state == WindowState::Minimized
    && !window.native_properties().is_minimized
  {
    info!("No window state update. Minimizing window.");

    // TODO: Instead of doing the platform call directly here, instead add
    // a `queue_state_change` method to `PendingSync`.
    if let Err(err) = window.native().minimize() {
      warn!("Failed to minimize window: {}", err);
    }

    return Ok(window);
  }

  let workspace = window.workspace().context("No workspace.")?;

  match window {
    WindowContainer::NonTilingWindow(window) => {
      let current_state = window.state();

      // Update the window's previous state if the discriminant changes.
      // TODO: Move out handling of active drag. Can then simplify calls to
      // `set_active_drag` in `handle_window_moved_or_resized_end`.
      if !current_state.is_same_state(&target_state)
        && window.active_drag().is_none()
      {
        window.set_prev_state(current_state);
        state.pending_sync.queue_workspace_to_reorder(workspace);
      }

      window.set_state(target_state);
      state.pending_sync.queue_container_to_redraw(window.clone());

      Ok(window.into())
    }
    WindowContainer::TilingWindow(window) => {
      let non_tiling_window = window.to_non_tiling(target_state.clone());

      replace_container(
        &non_tiling_window.clone().into(),
        &workspace.clone().into(),
        window.index(),
      )?;

      state
        .pending_sync
        .queue_container_to_redraw(non_tiling_window.clone())
        .queue_container_to_redraw(workspace.clone())
        .queue_workspace_to_reorder(workspace);

      Ok(non_tiling_window.into())
    }
  }
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_common::FloatingStateConfig;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::{
    commands::container::set_focused_descendant,
    models::{Monitor, NonTilingWindow, TilingWindow, Workspace},
  };

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  fn floating() -> WindowState {
    WindowState::Floating(FloatingStateConfig::default())
  }

  fn setup(
    tiling_count: usize,
  ) -> (Workspace, Vec<TilingWindow>, NonTilingWindow) {
    let windows = (0..tiling_count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let floating_window = NonTilingWindow::mock().call();
    let workspace = Workspace::mock()
      .tiling_windows(windows.clone())
      .non_tiling_windows(vec![floating_window.clone()])
      .call();
    // The workspace keeps its monitor alive through its parent.
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    (workspace, windows, floating_window)
  }

  /// IDs of the distinct windows queued for redraw.
  fn redrawn_ids(
    state: &WmState,
  ) -> std::collections::HashSet<uuid::Uuid> {
    state
      .windows_to_redraw()
      .iter()
      .map(CommonGetters::id)
      .collect()
  }

  fn tiling_ids(workspace: &Workspace) -> Vec<uuid::Uuid> {
    workspace
      .tiling_children()
      .map(|window| window.id())
      .collect()
  }

  #[test]
  fn a_window_becoming_tiling_is_inserted_after_the_focused_one() {
    let mut state = mock_state();
    let (workspace, windows, floating_window) = setup(3);
    set_focused_descendant(&windows[1].clone().into(), None);

    let tiling = update_window_state(
      floating_window.into(),
      WindowState::Tiling,
      &mut state,
    )
    .unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![
        windows[0].id(),
        windows[1].id(),
        tiling.id(),
        windows[2].id()
      ]
    );
    assert!(tiling.is_tiling_window());

    // The whole workspace is redrawn, not only the new window.
    assert_eq!(redrawn_ids(&state).len(), 4);
  }

  #[test]
  fn a_window_becoming_tiling_is_appended_without_tiling_windows() {
    let mut state = mock_state();
    let (workspace, _, floating_window) = setup(0);

    let tiling = update_window_state(
      floating_window.into(),
      WindowState::Tiling,
      &mut state,
    )
    .unwrap();

    assert_eq!(tiling_ids(&workspace), vec![tiling.id()]);
  }

  #[test]
  fn a_tiling_window_becoming_floating_leaves_the_tiling_order() {
    let mut state = mock_state();
    let (workspace, windows, _) = setup(3);

    let window = update_window_state(
      windows[1].clone().into(),
      floating(),
      &mut state,
    )
    .unwrap();

    assert!(!window.is_tiling_window());
    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[0].id(), windows[2].id()]
    );

    // The remaining tiling windows are redrawn along with the window.
    assert_eq!(redrawn_ids(&state).len(), 4);
  }
}
