use anyhow::Context;
use wm_common::WmEvent;

use super::{
  activate_workspace, focus_workspace,
  move_workspace_in_direction::adjust_windows_to_new_monitor,
  sort_workspaces,
};
use crate::{
  commands::container::move_container_within_tree,
  models::{Monitor, WorkspaceTarget},
  traits::CommonGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

/// Brings a workspace by name to the given monitor and focuses it.
///
/// The workspace is moved over if it is active on another monitor, and
/// activated on the given monitor if it isn't active. The monitor it
/// leaves gets a new workspace if it has none left.
pub fn summon_workspace(
  name: &str,
  monitor: &Monitor,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  match state.workspace_by_name(name) {
    None => {
      activate_workspace(
        Some(name),
        Some(monitor.clone()),
        state,
        config,
      )?;
    }
    Some(workspace) => {
      let origin_monitor = workspace.monitor().context("No monitor.")?;

      if origin_monitor.id() != monitor.id() {
        // Hidden once the summoned workspace is focused.
        let displayed_workspace = monitor
          .displayed_workspace()
          .context("No displayed workspace.")?;

        move_container_within_tree(
          &workspace.clone().into(),
          &monitor.clone().into(),
          monitor.child_count(),
          state,
        )?;

        adjust_windows_to_new_monitor(&workspace)?;
        state
          .pending_sync
          .queue_container_to_redraw(workspace.clone())
          .queue_container_to_redraw(displayed_workspace);

        match origin_monitor.displayed_workspace() {
          // Prevent origin monitor from having no workspaces.
          None => {
            activate_workspace(None, Some(origin_monitor), state, config)?
          }
          Some(origin_displayed) => {
            state
              .pending_sync
              .queue_container_to_redraw(origin_displayed);
          }
        }

        sort_workspaces(monitor, config)?;

        state.emit_event(WmEvent::WorkspaceUpdated {
          updated_workspace: workspace.to_dto()?,
        });
      }
    }
  }

  focus_workspace(WorkspaceTarget::Name(name.to_string()), state, config)
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::{
    commands::container::{attach_container, set_focused_descendant},
    models::{TilingWindow, Workspace},
  };

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  /// Attaches a monitor with the given workspaces to the root container.
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

  fn workspace(name: &str) -> Workspace {
    Workspace::mock().name(name.to_string()).call()
  }

  fn names(monitor: &Monitor) -> Vec<String> {
    monitor
      .workspaces()
      .iter()
      .map(|workspace| workspace.config().name)
      .collect()
  }

  #[test]
  fn moves_an_active_workspace_and_refills_its_monitor() {
    let mut state = mock_state();
    let config = UserConfig::mock();
    let window = TilingWindow::mock().call();
    let summoned = Workspace::mock()
      .name("2".to_string())
      .tiling_windows(vec![window.clone()])
      .call();

    let monitor = add_monitor(&state, vec![workspace("1")]);
    let other = add_monitor(&state, vec![summoned.clone()]);
    set_focused_descendant(&monitor.workspaces()[0].clone().into(), None);

    summon_workspace("2", &monitor, &mut state, &config).unwrap();

    // Workspace "1" is destroyed once hidden, since it is empty.
    assert_eq!(names(&monitor), ["2"]);
    assert_eq!(monitor.displayed_workspace(), Some(summoned));
    assert_eq!(state.focused_container(), Some(window.into()));
    assert_eq!(names(&other), ["3"]);
  }

  #[test]
  fn activates_an_inactive_workspace_on_the_monitor() {
    let mut state = mock_state();
    let config = UserConfig::mock();

    let monitor = add_monitor(&state, vec![workspace("1")]);
    add_monitor(&state, vec![workspace("2")]);
    set_focused_descendant(&monitor.workspaces()[0].clone().into(), None);

    summon_workspace("5", &monitor, &mut state, &config).unwrap();

    let summoned = state.workspace_by_name("5").unwrap();
    assert_eq!(summoned.monitor().map(|m| m.id()), Some(monitor.id()));
    assert_eq!(monitor.displayed_workspace(), Some(summoned));
  }
}
