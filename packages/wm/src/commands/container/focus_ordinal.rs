use anyhow::Context;
use wm_common::Ordinal;

use super::{ordinal_target, set_focused_descendant, OrdinalTarget};
use crate::{
  models::{Container, TilingWindow},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Focuses the next or previous tiling window of the origin window's
/// workspace.
///
/// Wraps around at the ends of the workspace. With `cross_monitor` and
/// more than one monitor, instead continues onto the first (or last)
/// tiling window of the next (or previous) monitor's displayed workspace.
/// If that workspace has no tiling windows, the workspace itself is
/// focused.
pub fn focus_ordinal(
  origin_window: &TilingWindow,
  ordinal: Ordinal,
  cross_monitor: bool,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let focus_target: Container =
    match ordinal_target(origin_window, ordinal, cross_monitor, state)? {
      OrdinalTarget::InWorkspace(index) => origin_window
        .workspace()
        .context("No workspace.")?
        .tiling_children()
        .nth(index)
        .context("No tiling window at the target index.")?
        .into(),
      OrdinalTarget::CrossMonitor { workspace, index } => {
        // The target index is the insertion index of a moved window, so it
        // is one past the last window when stepping backward.
        let last_index =
          workspace.tiling_children().count().checked_sub(1);

        match last_index {
          Some(last_index) => workspace
            .tiling_children()
            .nth(index.min(last_index))
            .context("No tiling window at the target index.")?
            .into(),
          None => workspace.into(),
        }
      }
    };

  if focus_target.id() != origin_window.id() {
    set_focused_descendant(&focus_target, None);
    state.pending_sync.queue_focus_change().queue_cursor_jump();
  }

  Ok(())
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
    count: usize,
  ) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();
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

  #[test]
  fn focuses_the_neighbouring_window() {
    let mut state = mock_state();
    let (_, windows) = add_workspace(&state, 3);

    focus_ordinal(&windows[0], Ordinal::Next, false, &mut state).unwrap();
    assert_eq!(state.focused_container(), Some(windows[1].as_container()));

    focus_ordinal(&windows[1], Ordinal::Previous, false, &mut state)
      .unwrap();
    assert_eq!(state.focused_container(), Some(windows[0].as_container()));
  }

  #[test]
  fn wraps_around_the_ends() {
    let mut state = mock_state();
    let (_, windows) = add_workspace(&state, 3);

    focus_ordinal(&windows[0], Ordinal::Previous, false, &mut state)
      .unwrap();
    assert_eq!(state.focused_container(), Some(windows[2].as_container()));

    focus_ordinal(&windows[2], Ordinal::Next, true, &mut state).unwrap();
    assert_eq!(state.focused_container(), Some(windows[0].as_container()));
  }

  #[test]
  fn does_not_queue_a_focus_change_for_a_single_window() {
    let mut state = mock_state();
    let (_, windows) = add_workspace(&state, 1);

    focus_ordinal(&windows[0], Ordinal::Next, false, &mut state).unwrap();

    assert!(!state.pending_sync.has_changes());
  }

  #[test]
  fn crosses_to_the_first_window_of_the_next_monitor() {
    let mut state = mock_state();
    let (_, first_windows) = add_workspace(&state, 2);
    let (_, second_windows) = add_workspace(&state, 2);

    focus_ordinal(&first_windows[1], Ordinal::Next, true, &mut state)
      .unwrap();

    assert_eq!(
      state.focused_container(),
      Some(second_windows[0].as_container())
    );
    assert!(state.pending_sync.needs_focus_update());
  }

  #[test]
  fn crosses_to_the_last_window_of_the_previous_monitor() {
    let mut state = mock_state();
    let (_, first_windows) = add_workspace(&state, 2);
    let (_, second_windows) = add_workspace(&state, 2);

    focus_ordinal(&second_windows[0], Ordinal::Previous, true, &mut state)
      .unwrap();

    assert_eq!(
      state.focused_container(),
      Some(first_windows[1].as_container())
    );
  }

  #[test]
  fn focuses_the_workspace_when_crossing_into_an_empty_one() {
    let mut state = mock_state();
    let (_, first_windows) = add_workspace(&state, 1);
    let (empty, _) = add_workspace(&state, 0);

    focus_ordinal(&first_windows[0], Ordinal::Next, true, &mut state)
      .unwrap();

    assert_eq!(state.focused_container(), Some(empty.as_container()));
  }
}
