use anyhow::Context;
use wm_common::{Layout, LayoutParams, WmEvent};

use crate::{
  models::{Container, TilingWindow, Workspace},
  traits::{CommonGetters, PositionGetters},
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

/// Resizes the master area of a workspace so that a tiling window ends up
/// with the given width, as far as the layout allows.
///
/// Only the master/stack and dwindle layouts have a master area: the
/// master ratio grows with the width of a master window (the first window
/// for dwindle), and shrinks with the width of a stack window. Other
/// layouts, and workspaces whose layout has a single area, are left
/// untouched.
///
/// Stack windows deeper in the dwindle spiral only follow the ratio of the
/// first split, so their resulting width is approximate.
pub fn resize_master_ratio(
  window: &TilingWindow,
  target_width: i32,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;
  let index = window.tiling_index().context("No tiling index.")?;
  let count = workspace.tiling_children().count();
  let params = workspace.layout_params();

  let is_master = match workspace.layout() {
    Layout::MasterStack if count > params.master_count => {
      index < params.master_count
    }
    Layout::Dwindle if count > 1 => index == 0,
    layout => {
      tracing::debug!(
        "Ignoring width resize of a tiling window with layout {layout:?}          and {count} window(s)."
      );
      return Ok(());
    }
  };

  // The master ratio splits the workspace width that remains after the gap
  // between the master area and the stack.
  let (gap_x, _) = workspace.inner_gaps()?;
  let available_width = workspace.to_rect()?.width() - gap_x;

  if available_width <= 0 {
    return Ok(());
  }

  let width_delta = target_width - window.to_rect()?.width();

  // A wider stack window means a narrower master area.
  let signed_delta = if is_master { width_delta } else { -width_delta };

  #[allow(clippy::cast_precision_loss)]
  let ratio_delta = signed_delta as f32 / available_width as f32;

  let new_ratio =
    LayoutParams::clamp_ratio(params.master_ratio + ratio_delta);

  if (new_ratio - params.master_ratio).abs() < f32::EPSILON {
    return Ok(());
  }

  adjust_master_ratio(&workspace.into(), ratio_delta, state)
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

  // Any change to the layout can move every window of the workspace, and
  // overlapping layouts (e.g. monocle) need the focused window on top.
  state
    .pending_sync
    .queue_container_to_redraw(workspace.clone())
    .queue_workspace_to_reorder(workspace);

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

  /// Creates a workspace with `count` windows and a 50/50 master ratio.
  fn workspace_with_windows(
    layout: Layout,
    count: usize,
  ) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace = Workspace::mock()
      .layout(layout)
      .layout_params(LayoutParams {
        master_ratio: 0.5,
        ..LayoutParams::default()
      })
      .tiling_windows(windows.clone())
      .call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    (workspace, windows)
  }

  fn ratio(workspace: &Workspace) -> f32 {
    workspace.layout_params().master_ratio
  }

  #[test]
  fn growing_the_master_grows_the_master_ratio() {
    let mut state = mock_state();
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 3);

    // The mock monitor is 1680px wide and has no gaps, so 168px is 10% of
    // the width.
    let width = windows[0].to_rect().unwrap().width();
    resize_master_ratio(&windows[0], width + 168, &mut state).unwrap();

    assert!((ratio(&workspace) - 0.6).abs() < 1e-4);
    assert!(state.pending_sync.has_changes());
  }

  #[test]
  fn growing_a_stack_window_shrinks_the_master_ratio() {
    let mut state = mock_state();
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 3);

    let width = windows[2].to_rect().unwrap().width();
    resize_master_ratio(&windows[2], width + 168, &mut state).unwrap();

    assert!((ratio(&workspace) - 0.4).abs() < 1e-4);
  }

  #[test]
  fn the_first_dwindle_window_is_the_master() {
    let mut state = mock_state();
    let (workspace, windows) = workspace_with_windows(Layout::Dwindle, 3);

    let width = windows[0].to_rect().unwrap().width();
    resize_master_ratio(&windows[0], width - 168, &mut state).unwrap();
    assert!((ratio(&workspace) - 0.4).abs() < 1e-4);

    let width = windows[1].to_rect().unwrap().width();
    resize_master_ratio(&windows[1], width - 168, &mut state).unwrap();
    assert!((ratio(&workspace) - 0.5).abs() < 1e-4);
  }

  #[test]
  fn the_master_ratio_is_clamped() {
    let mut state = mock_state();
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 2);

    resize_master_ratio(&windows[0], 100_000, &mut state).unwrap();

    assert!(
      (ratio(&workspace) - LayoutParams::MAX_MASTER_RATIO).abs() < 1e-6
    );
  }

  #[test]
  fn unchanged_widths_do_not_queue_a_redraw() {
    let mut state = mock_state();
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 2);

    let width = windows[0].to_rect().unwrap().width();
    resize_master_ratio(&windows[0], width, &mut state).unwrap();

    assert!((ratio(&workspace) - 0.5).abs() < f32::EPSILON);
    assert!(!state.pending_sync.has_changes());
  }

  #[test]
  fn layouts_without_a_master_area_are_left_untouched() {
    for layout in
      [Layout::Columns, Layout::Rows, Layout::Monocle, Layout::Grid]
    {
      let mut state = mock_state();
      let (workspace, windows) = workspace_with_windows(layout, 3);

      resize_master_ratio(&windows[0], 100, &mut state).unwrap();

      assert!(
        (ratio(&workspace) - 0.5).abs() < f32::EPSILON,
        "{layout:?}"
      );
      assert!(!state.pending_sync.has_changes(), "{layout:?}");
    }
  }

  #[test]
  fn a_lone_window_has_no_master_area() {
    let mut state = mock_state();

    for layout in [Layout::MasterStack, Layout::Dwindle] {
      let (workspace, windows) = workspace_with_windows(layout, 1);

      resize_master_ratio(&windows[0], 100, &mut state).unwrap();

      assert!(
        (ratio(&workspace) - 0.5).abs() < f32::EPSILON,
        "{layout:?}"
      );
    }

    assert!(!state.pending_sync.has_changes());
  }

  #[test]
  fn extra_masters_are_not_stack_windows() {
    let mut state = mock_state();
    let windows = (0..3)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace = Workspace::mock()
      .layout_params(LayoutParams {
        master_ratio: 0.5,
        master_count: 2,
        flip: false,
      })
      .tiling_windows(windows.clone())
      .call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    // The second window is still a master, the third one is the stack.
    let width = windows[1].to_rect().unwrap().width();
    resize_master_ratio(&windows[1], width + 168, &mut state).unwrap();
    assert!((ratio(&workspace) - 0.6).abs() < 1e-4);

    let width = windows[2].to_rect().unwrap().width();
    resize_master_ratio(&windows[2], width + 168, &mut state).unwrap();
    assert!((ratio(&workspace) - 0.5).abs() < 1e-4);
  }
}
