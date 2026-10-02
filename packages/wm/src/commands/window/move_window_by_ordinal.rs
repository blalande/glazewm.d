use anyhow::Context;
use wm_common::Ordinal;

use super::move_window_to_workspace;
use crate::{
  commands::container::{
    ordinal_target, set_focused_descendant, swap_tiling_windows,
    OrdinalTarget,
  },
  models::{TilingWindow, WorkspaceTarget},
  traits::CommonGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

/// Moves a tiling window one step towards the end or the start of its
/// workspace by swapping it with its neighbour.
///
/// Wraps around at the ends of the workspace. With `cross_monitor` and
/// more than one monitor, stepping past an end instead moves the window to
/// the next (or previous) monitor's displayed workspace: at the start of
/// that workspace when stepping forward, and at the end when stepping
/// backward.
///
/// The moved window keeps focus, including when it crosses monitors.
pub fn move_window_by_ordinal(
  window: TilingWindow,
  ordinal: Ordinal,
  cross_monitor: bool,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  match ordinal_target(&window, ordinal, cross_monitor, state)? {
    OrdinalTarget::InWorkspace(index) => {
      let workspace = window.workspace().context("No workspace.")?;

      let neighbour = workspace
        .tiling_children()
        .nth(index)
        .context("No tiling window at the target index.")?;

      // Nothing to swap if the window is alone in the workspace.
      if neighbour.id() != window.id() {
        swap_tiling_windows(&window, &neighbour, state)?;
        state.pending_sync.queue_container_to_redraw(workspace);
      }

      Ok(())
    }
    OrdinalTarget::CrossMonitor { workspace, index } => {
      move_window_to_workspace(
        window.clone().into(),
        WorkspaceTarget::Name(workspace.config().name),
        Some(index),
        state,
        config,
      )?;

      // `move_window_to_workspace` hands focus back to the origin
      // workspace, so re-focus the moved window to follow it onto the
      // target monitor.
      set_focused_descendant(&window.into(), None);
      state.pending_sync.queue_focus_change().queue_cursor_jump();

      Ok(())
    }
  }
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::{
    commands::container::attach_container,
    models::{Monitor, Workspace},
  };

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  fn add_workspace(
    state: &WmState,
    name: &str,
    count: usize,
  ) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace = Workspace::mock()
      .name(name.to_string())
      .tiling_windows(windows.clone())
      .call();
    let monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    attach_container(
      &monitor.into(),
      &state.root_container.clone().into(),
      None,
    )
    .unwrap();

    (workspace, windows)
  }

  fn tiling_ids(workspace: &Workspace) -> Vec<uuid::Uuid> {
    workspace
      .tiling_children()
      .map(|window| window.id())
      .collect()
  }

  #[test]
  fn swaps_with_the_neighbour() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let (workspace, windows) = add_workspace(&state, "1", 3);

    move_window_by_ordinal(
      windows[0].clone(),
      Ordinal::Next,
      false,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[1].id(), windows[0].id(), windows[2].id()]
    );
    assert!(state.pending_sync.has_changes());
  }

  #[test]
  fn wraps_around_the_ends() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let (workspace, windows) = add_workspace(&state, "1", 3);

    // Previous from the first window swaps it with the last one, even if
    // `cross_monitor` is set with a single monitor.
    move_window_by_ordinal(
      windows[0].clone(),
      Ordinal::Previous,
      true,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[2].id(), windows[1].id(), windows[0].id()]
    );
  }

  #[test]
  fn moves_to_the_start_of_the_next_monitor() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let (first, first_windows) = add_workspace(&state, "1", 2);
    let (second, second_windows) = add_workspace(&state, "2", 2);

    move_window_by_ordinal(
      first_windows[1].clone(),
      Ordinal::Next,
      true,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(tiling_ids(&first), vec![first_windows[0].id()]);
    assert_eq!(
      tiling_ids(&second),
      vec![
        first_windows[1].id(),
        second_windows[0].id(),
        second_windows[1].id()
      ]
    );
    assert!(state.pending_sync.has_changes());

    // The moved window keeps focus on the target monitor.
    assert_eq!(
      state.focused_container().map(|container| container.id()),
      Some(first_windows[1].id())
    );
  }

  #[test]
  fn moves_to_the_end_of_the_previous_monitor() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let (first, first_windows) = add_workspace(&state, "1", 2);
    let (second, second_windows) = add_workspace(&state, "2", 2);

    move_window_by_ordinal(
      second_windows[0].clone(),
      Ordinal::Previous,
      true,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(
      tiling_ids(&first),
      vec![
        first_windows[0].id(),
        first_windows[1].id(),
        second_windows[0].id()
      ]
    );
    assert_eq!(tiling_ids(&second), vec![second_windows[1].id()]);
  }

  #[test]
  fn moves_into_an_empty_workspace_of_another_monitor() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let (first, first_windows) = add_workspace(&state, "1", 1);
    let (second, _) = add_workspace(&state, "2", 0);

    move_window_by_ordinal(
      first_windows[0].clone(),
      Ordinal::Next,
      true,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(tiling_ids(&first), Vec::<uuid::Uuid>::new());
    assert_eq!(tiling_ids(&second), vec![first_windows[0].id()]);
  }

  #[test]
  fn does_nothing_for_a_single_window() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let (workspace, windows) = add_workspace(&state, "1", 1);

    move_window_by_ordinal(
      windows[0].clone(),
      Ordinal::Next,
      false,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(tiling_ids(&workspace), vec![windows[0].id()]);
    assert!(!state.pending_sync.has_changes());
  }
}
