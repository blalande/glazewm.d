use anyhow::Context;
use wm_common::Ordinal;

use crate::{
  models::{TilingWindow, Workspace},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Where an ordinal step from a tiling window lands.
#[derive(Clone, Debug, PartialEq)]
pub enum OrdinalTarget {
  /// Index amongst the tiling children of the window's own workspace.
  ///
  /// Equals the window's own index when it is the only tiling window.
  InWorkspace(usize),

  /// Displayed workspace of another monitor, and the index amongst its
  /// tiling children at which a moved window is inserted.
  ///
  /// The index is `0` when stepping forward, and the number of tiling
  /// windows when stepping backward. Hence it is one past the last tiling
  /// window when stepping backward, which a focus step has to clamp.
  CrossMonitor { workspace: Workspace, index: usize },
}

/// Resolves the target of an ordinal step from a tiling window. Shared by
/// the `focus` and `move` commands.
///
/// Steps by one amongst the tiling windows of the window's workspace,
/// wrapping at both ends. With `cross_monitor` and more than one monitor,
/// stepping past an end instead lands on the displayed workspace of the
/// next (or previous) monitor, wrapping through the monitors in the order
/// of the root container.
pub fn ordinal_target(
  window: &TilingWindow,
  ordinal: Ordinal,
  cross_monitor: bool,
  state: &WmState,
) -> anyhow::Result<OrdinalTarget> {
  let workspace = window.workspace().context("No workspace.")?;
  let index = window.tiling_index().context("No tiling index.")?;
  let count = workspace.tiling_children().count();

  let is_past_end = match ordinal {
    Ordinal::Next => index + 1 >= count,
    Ordinal::Previous => index == 0,
  };

  let monitors = state.monitors();

  if !(cross_monitor && is_past_end && monitors.len() > 1) {
    return Ok(OrdinalTarget::InWorkspace(
      ordinal.wrapping_step(index, count),
    ));
  }

  let monitor = window.monitor().context("No monitor.")?;
  let monitor_index = monitors
    .iter()
    .position(|other| other.id() == monitor.id())
    .context("Monitor of the window is not in the root container.")?;

  let target_workspace = monitors
    [ordinal.wrapping_step(monitor_index, monitors.len())]
  .displayed_workspace()
  .context("Target monitor has no displayed workspace.")?;

  let index = match ordinal {
    Ordinal::Next => 0,
    Ordinal::Previous => target_workspace.tiling_children().count(),
  };

  Ok(OrdinalTarget::CrossMonitor {
    workspace: target_workspace,
    index,
  })
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::{
    commands::container::attach_container,
    models::{Monitor, NonTilingWindow},
  };

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  /// Creates a workspace with `count` tiling windows.
  fn workspace_with_windows(
    count: usize,
  ) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();

    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();

    (workspace, windows)
  }

  /// Attaches a monitor displaying the given workspace to the root
  /// container.
  fn add_monitor(state: &WmState, workspace: &Workspace) {
    let monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    attach_container(
      &monitor.into(),
      &state.root_container.clone().into(),
      None,
    )
    .unwrap();
  }

  fn target(
    window: &TilingWindow,
    ordinal: Ordinal,
    cross_monitor: bool,
    state: &WmState,
  ) -> OrdinalTarget {
    ordinal_target(window, ordinal, cross_monitor, state).unwrap()
  }

  #[test]
  fn steps_within_workspace() {
    let state = mock_state();
    let (workspace, windows) = workspace_with_windows(3);
    add_monitor(&state, &workspace);

    for cross_monitor in [false, true] {
      assert_eq!(
        target(&windows[0], Ordinal::Next, cross_monitor, &state),
        OrdinalTarget::InWorkspace(1)
      );
      assert_eq!(
        target(&windows[2], Ordinal::Previous, cross_monitor, &state),
        OrdinalTarget::InWorkspace(1)
      );
    }
  }

  #[test]
  fn wraps_at_both_ends_without_cross_monitor() {
    let state = mock_state();
    let (workspace, windows) = workspace_with_windows(3);
    let (other, _) = workspace_with_windows(2);
    add_monitor(&state, &workspace);
    add_monitor(&state, &other);

    assert_eq!(
      target(&windows[2], Ordinal::Next, false, &state),
      OrdinalTarget::InWorkspace(0)
    );
    assert_eq!(
      target(&windows[0], Ordinal::Previous, false, &state),
      OrdinalTarget::InWorkspace(2)
    );
  }

  #[test]
  fn wraps_on_a_single_monitor_with_cross_monitor() {
    let state = mock_state();
    let (workspace, windows) = workspace_with_windows(3);
    add_monitor(&state, &workspace);

    assert_eq!(
      target(&windows[2], Ordinal::Next, true, &state),
      OrdinalTarget::InWorkspace(0)
    );
    assert_eq!(
      target(&windows[0], Ordinal::Previous, true, &state),
      OrdinalTarget::InWorkspace(2)
    );
  }

  #[test]
  fn a_lone_window_targets_itself() {
    let state = mock_state();
    let (workspace, windows) = workspace_with_windows(1);
    add_monitor(&state, &workspace);

    assert_eq!(
      target(&windows[0], Ordinal::Next, false, &state),
      OrdinalTarget::InWorkspace(0)
    );
    assert_eq!(
      target(&windows[0], Ordinal::Previous, true, &state),
      OrdinalTarget::InWorkspace(0)
    );
  }

  #[test]
  fn crosses_to_the_next_monitor_past_the_end() {
    let state = mock_state();
    let (first, first_windows) = workspace_with_windows(2);
    let (second, _) = workspace_with_windows(3);
    add_monitor(&state, &first);
    add_monitor(&state, &second);

    // Not at the end yet: stays in the workspace.
    assert_eq!(
      target(&first_windows[0], Ordinal::Next, true, &state),
      OrdinalTarget::InWorkspace(1)
    );

    assert_eq!(
      target(&first_windows[1], Ordinal::Next, true, &state),
      OrdinalTarget::CrossMonitor {
        workspace: second,
        index: 0
      }
    );
  }

  #[test]
  fn crosses_to_the_previous_monitor_past_the_start() {
    let state = mock_state();
    let (first, _) = workspace_with_windows(3);
    let (second, second_windows) = workspace_with_windows(2);
    add_monitor(&state, &first);
    add_monitor(&state, &second);

    assert_eq!(
      target(&second_windows[0], Ordinal::Previous, true, &state),
      OrdinalTarget::CrossMonitor {
        workspace: first,
        index: 3
      }
    );
  }

  #[test]
  fn wraps_through_monitors() {
    let state = mock_state();
    let (first, first_windows) = workspace_with_windows(1);
    let (second, second_windows) = workspace_with_windows(1);
    add_monitor(&state, &first);
    add_monitor(&state, &second);

    // Past the last monitor wraps to the first one, and vice versa.
    assert_eq!(
      target(&second_windows[0], Ordinal::Next, true, &state),
      OrdinalTarget::CrossMonitor {
        workspace: first,
        index: 0
      }
    );
    assert_eq!(
      target(&first_windows[0], Ordinal::Previous, true, &state),
      OrdinalTarget::CrossMonitor {
        workspace: second,
        index: 1
      }
    );
  }

  #[test]
  fn crossing_into_an_empty_workspace_targets_index_zero() {
    let state = mock_state();
    let (first, first_windows) = workspace_with_windows(2);
    let (empty, _) = workspace_with_windows(0);
    add_monitor(&state, &first);
    add_monitor(&state, &empty);

    // With two monitors, the empty one is both the next and the previous
    // monitor of the first one.
    for (ordinal, window) in [
      (Ordinal::Next, &first_windows[1]),
      (Ordinal::Previous, &first_windows[0]),
    ] {
      assert_eq!(
        target(window, ordinal, true, &state),
        OrdinalTarget::CrossMonitor {
          workspace: empty.clone(),
          index: 0
        }
      );
    }
  }

  #[test]
  fn ignores_non_tiling_windows_when_indexing() {
    let state = mock_state();
    let windows =
      vec![TilingWindow::mock().call(), TilingWindow::mock().call()];
    let workspace = Workspace::mock()
      .tiling_windows(windows.clone())
      .non_tiling_windows(vec![NonTilingWindow::mock().call()])
      .call();
    add_monitor(&state, &workspace);

    assert_eq!(
      target(&windows[1], Ordinal::Next, false, &state),
      OrdinalTarget::InWorkspace(0)
    );
    assert_eq!(
      target(&windows[0], Ordinal::Previous, false, &state),
      OrdinalTarget::InWorkspace(1)
    );
  }
}
