use wm_common::{Layout, LayoutParams, WmEvent};

use crate::{
  models::{Container, Workspace},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Sets the layout of the workspace that the container belongs to.
pub fn set_layout(
  container: &Container,
  layout: Layout,
  state: &mut WmState,
) -> anyhow::Result<()> {
  update_layout(container, state, |workspace| workspace.set_layout(layout))
}

/// Switches the workspace that the container belongs to, to the next
/// layout in cycle order.
pub fn cycle_layout(
  container: &Container,
  state: &mut WmState,
) -> anyhow::Result<()> {
  update_layout(container, state, |workspace| {
    workspace.set_layout(workspace.layout().next());
  })
}

/// Adjusts the master ratio of the workspace that the container belongs
/// to.
///
/// The resulting ratio is clamped to the valid range.
pub fn adjust_master_ratio(
  container: &Container,
  delta: f32,
  state: &mut WmState,
) -> anyhow::Result<()> {
  update_layout_params(container, state, |params| {
    params.adjust_master_ratio(delta);
  })
}

/// Adjusts the master count of the workspace that the container belongs
/// to.
///
/// The resulting count is never lower than 1.
pub fn adjust_master_count(
  container: &Container,
  delta: i32,
  state: &mut WmState,
) -> anyhow::Result<()> {
  update_layout_params(container, state, |params| {
    params.adjust_master_count(delta);
  })
}

/// Mirrors the layout of the workspace that the container belongs to.
pub fn flip_layout(
  container: &Container,
  state: &mut WmState,
) -> anyhow::Result<()> {
  update_layout_params(container, state, |params| {
    params.flip = !params.flip;
  })
}

/// Applies a change to the layout parameters of the container's workspace.
fn update_layout_params(
  container: &Container,
  state: &mut WmState,
  update: impl FnOnce(&mut LayoutParams),
) -> anyhow::Result<()> {
  update_layout(container, state, |workspace| {
    let mut params = workspace.layout_params();
    update(&mut params);
    workspace.set_layout_params(params);
  })
}

/// Applies a change to the container's workspace, then emits a
/// `LayoutChanged` event and queues the workspace for redraw.
///
/// Does nothing if the container does not belong to a workspace.
fn update_layout(
  container: &Container,
  state: &mut WmState,
  update: impl FnOnce(&Workspace),
) -> anyhow::Result<()> {
  let Some(workspace) = container.workspace() else {
    return Ok(());
  };

  update(&workspace);

  state.emit_event(WmEvent::LayoutChanged {
    workspace: workspace.to_dto()?,
    new_layout: workspace.layout(),
    new_layout_params: workspace.layout_params(),
  });

  // Any change to the layout can move every window of the workspace.
  state.pending_sync.queue_container_to_redraw(workspace);

  Ok(())
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_platform::Dispatcher;

  use super::*;
  use crate::models::Monitor;

  fn mock_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  #[test]
  fn set_and_cycle_layout() -> anyhow::Result<()> {
    let mut state = mock_state();
    let workspace = Workspace::mock().call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();
    let container: Container = workspace.clone().into();

    set_layout(&container, Layout::Rows, &mut state)?;
    assert_eq!(workspace.layout(), Layout::Rows);
    assert!(state.pending_sync.has_changes());

    cycle_layout(&container, &mut state)?;
    assert_eq!(workspace.layout(), Layout::Rows.next());

    Ok(())
  }

  #[test]
  fn adjusts_layout_params_within_limits() -> anyhow::Result<()> {
    let mut state = mock_state();
    let workspace = Workspace::mock().call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();
    let container: Container = workspace.clone().into();

    adjust_master_ratio(&container, 0.05, &mut state)?;
    assert!((workspace.layout_params().master_ratio - 0.6).abs() < 1e-6);

    adjust_master_ratio(&container, 5., &mut state)?;
    assert!(
      (workspace.layout_params().master_ratio
        - LayoutParams::MAX_MASTER_RATIO)
        .abs()
        < 1e-6
    );

    adjust_master_count(&container, -3, &mut state)?;
    assert_eq!(workspace.layout_params().master_count, 1);

    adjust_master_count(&container, 2, &mut state)?;
    assert_eq!(workspace.layout_params().master_count, 3);

    Ok(())
  }

  #[test]
  fn flips_layout() -> anyhow::Result<()> {
    let mut state = mock_state();
    let workspace = Workspace::mock().call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();
    let container: Container = workspace.clone().into();

    flip_layout(&container, &mut state)?;
    assert!(workspace.layout_params().flip);

    flip_layout(&container, &mut state)?;
    assert!(!workspace.layout_params().flip);

    Ok(())
  }
}
