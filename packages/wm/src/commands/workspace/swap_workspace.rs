use anyhow::Context;
use wm_common::{InvokeSwapWorkspaceCommand, WmEvent};

use super::{
  move_workspace_in_direction::adjust_windows_to_new_monitor,
  sort_workspaces,
};
use crate::{
  commands::container::{
    move_container_within_tree, set_focused_descendant,
  },
  models::Workspace,
  traits::CommonGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

/// Swaps a workspace with the target of a `swap-workspace` command, so
/// that each takes the other's place on its monitor.
///
/// Does nothing if there is no other monitor to swap with. Fails if the
/// named workspace isn't active, or is on the same monitor.
pub fn swap_workspace(
  workspace: &Workspace,
  args: &InvokeSwapWorkspaceCommand,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let monitor = workspace.monitor().context("No monitor.")?;

  let target = match (args.monitor_ordinal(), &args.workspace) {
    (Some(ordinal), _) => {
      let Some(target_monitor) =
        state.monitor_at_ordinal(&monitor, ordinal)
      else {
        return Ok(());
      };

      target_monitor
        .displayed_workspace()
        .context("No displayed workspace.")?
    }
    (None, Some(name)) => state
      .workspace_by_name(name)
      .with_context(|| format!("Workspace '{name}' is not active."))?,
    (None, None) => anyhow::bail!("No workspace to swap with."),
  };

  let target_monitor = target.monitor().context("No monitor.")?;

  if target_monitor.id() == monitor.id() {
    anyhow::bail!(
      "Workspace '{}' is on the same monitor.",
      target.config().name
    );
  }

  swap_workspaces(workspace, &target, state)?;

  sort_workspaces(&monitor, config)?;
  sort_workspaces(&target_monitor, config)
}

/// Swaps two workspaces on different monitors.
///
/// `workspace` takes the place of `target` on its monitor and is
/// displayed there, while `target` is displayed on the monitor of
/// `workspace`. Focus follows `workspace` if it had focus.
fn swap_workspaces(
  workspace: &Workspace,
  target: &Workspace,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let monitor = workspace.monitor().context("No monitor.")?;
  let target_monitor = target.monitor().context("No monitor.")?;

  // Workspace hidden by the swap if `target` wasn't displayed.
  let target_displayed = target_monitor
    .displayed_workspace()
    .context("No displayed workspace.")?;

  let (index, target_index) = (workspace.index(), target.index());

  move_container_within_tree(
    &target.clone().into(),
    &monitor.clone().into(),
    index,
    state,
  )?;

  move_container_within_tree(
    &workspace.clone().into(),
    &target_monitor.clone().into(),
    target_index,
    state,
  )?;

  // Display each workspace on its new monitor, without changing which
  // monitor has focus.
  for swapped in [workspace, target] {
    set_focused_descendant(
      &swapped.clone().into(),
      Some(&swapped.clone().into()),
    );
    adjust_windows_to_new_monitor(swapped)?;

    state.emit_event(WmEvent::WorkspaceUpdated {
      updated_workspace: swapped.to_dto()?,
    });
  }

  state
    .pending_sync
    .queue_cursor_jump()
    .queue_container_to_redraw(workspace.clone())
    .queue_container_to_redraw(target.clone())
    .queue_container_to_redraw(target_displayed);

  Ok(())
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::{
    commands::container::attach_container,
    models::{Monitor, TilingWindow},
  };

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  /// Attaches a monitor with the given workspaces to the root container.
  /// The first workspace is displayed.
  fn add_monitor(state: &WmState, workspaces: Vec<Workspace>) -> Monitor {
    let monitor = Monitor::mock().workspaces(workspaces).call();

    attach_container(
      &monitor.clone().into(),
      &state.root_container.clone().into(),
      None,
    )
    .unwrap();

    monitor
  }

  fn displayed(monitor: &Monitor) -> Workspace {
    monitor.displayed_workspace().unwrap()
  }

  #[test]
  fn swaps_the_displayed_workspaces_and_focus_follows() {
    let mut state = mock_state();
    let window = TilingWindow::mock().call();
    let workspace = Workspace::mock()
      .tiling_windows(vec![window.clone()])
      .call();
    let target = Workspace::mock().call();

    let monitor = add_monitor(&state, vec![workspace.clone()]);
    let target_monitor = add_monitor(&state, vec![target.clone()]);
    set_focused_descendant(&window.clone().into(), None);

    swap_workspaces(&workspace, &target, &mut state).unwrap();

    assert_eq!(displayed(&monitor), target);
    assert_eq!(displayed(&target_monitor), workspace);
    assert_eq!(monitor.workspaces(), vec![target]);
    assert_eq!(target_monitor.workspaces(), vec![workspace]);
    assert_eq!(state.focused_container(), Some(window.into()));
    assert!(state.pending_sync.has_changes());
  }

  #[test]
  fn a_hidden_target_is_displayed_and_its_place_taken() {
    let mut state = mock_state();
    let window = TilingWindow::mock().call();
    let workspace = Workspace::mock()
      .tiling_windows(vec![window.clone()])
      .call();
    let shown = Workspace::mock().call();
    let target = Workspace::mock().call();

    let monitor = add_monitor(&state, vec![workspace.clone()]);
    let target_monitor =
      add_monitor(&state, vec![shown.clone(), target.clone()]);
    set_focused_descendant(&window.clone().into(), None);
    assert_eq!(displayed(&target_monitor), shown);

    swap_workspaces(&workspace, &target, &mut state).unwrap();

    assert_eq!(displayed(&monitor), target);
    assert_eq!(displayed(&target_monitor), workspace);
    assert_eq!(target_monitor.workspaces(), vec![shown, workspace]);
  }
}
