use anyhow::Context;

use crate::{
  commands::container::swap_tiling_windows, models::TilingWindow,
  traits::CommonGetters, wm_state::WmState,
};

/// Swaps a tiling window with the first tiling window of its workspace,
/// which makes it the master.
///
/// If the window already is the first one, it is swapped with the second
/// one instead. Does nothing if the workspace has a single tiling window.
pub fn promote_window(
  window: &TilingWindow,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;
  let index = window.tiling_index().context("No tiling index.")?;

  let target_index = usize::from(index == 0);

  let target = workspace.tiling_children().nth(target_index);

  if let Some(target) = target {
    swap_tiling_windows(window, &target, state)?;
    state.pending_sync.queue_container_to_redraw(workspace);
  }

  Ok(())
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

  fn setup(count: usize) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace =
      Workspace::mock().tiling_windows(windows.clone()).call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    (workspace, windows)
  }

  fn tiling_ids(workspace: &Workspace) -> Vec<uuid::Uuid> {
    workspace
      .tiling_children()
      .map(|window| window.id())
      .collect()
  }

  #[test]
  fn swaps_with_the_first_window() {
    let mut state = mock_state();
    let (workspace, windows) = setup(3);

    promote_window(&windows[2], &mut state).unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[2].id(), windows[1].id(), windows[0].id()]
    );
    assert!(state.pending_sync.has_changes());
  }

  #[test]
  fn swaps_the_first_window_with_the_second() {
    let mut state = mock_state();
    let (workspace, windows) = setup(3);

    promote_window(&windows[0], &mut state).unwrap();

    assert_eq!(
      tiling_ids(&workspace),
      vec![windows[1].id(), windows[0].id(), windows[2].id()]
    );
  }

  #[test]
  fn does_nothing_for_a_single_window() {
    let mut state = mock_state();
    let (workspace, windows) = setup(1);

    promote_window(&windows[0], &mut state).unwrap();

    assert_eq!(tiling_ids(&workspace), vec![windows[0].id()]);
    assert!(!state.pending_sync.has_changes());
  }

  #[test]
  fn leaves_non_tiling_windows_in_place() {
    let mut state = mock_state();
    let (workspace, windows) = setup(2);

    let floating = crate::models::NonTilingWindow::mock().call();
    crate::commands::container::attach_container(
      &floating.clone().into(),
      &workspace.clone().into(),
      Some(1),
    )
    .unwrap();

    // Children: [tiling 0, floating, tiling 1].
    promote_window(&windows[1], &mut state).unwrap();

    let children = workspace
      .children()
      .iter()
      .map(CommonGetters::id)
      .collect::<Vec<_>>();
    assert_eq!(
      children,
      vec![windows[1].id(), floating.id(), windows[0].id()]
    );
  }
}
