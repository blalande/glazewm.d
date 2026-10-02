use anyhow::Context;
use uuid::Uuid;
use wm_common::{
  try_warn, ActiveDragOperation, FullscreenStateConfig, WindowState,
};
use wm_platform::Point;

use crate::{
  commands::{
    container::{move_container_within_tree, swap_tiling_windows},
    window::update_window_state,
    workspace::resize_tiling_window,
  },
  events::update_floating_window_position,
  models::{NonTilingWindow, TilingWindow, WindowContainer, Workspace},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Handles the event for when a window is finished being moved or resized
/// by the user (e.g. via the window's drag handles).
///
/// This drops a dragged tiling window (by swapping it with the window
/// under the cursor, or by resizing the master area) and attaches a
/// dragged floating window.
///
/// TODO: Move this to a better location - maybe a new `active_drag_ext`
/// mod.
pub fn handle_window_moved_or_resized_end(
  window: &WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let Some(active_drag) = window.active_drag() else {
    return Ok(());
  };

  let is_maximized = try_warn!(window.native().is_maximized());

  window.update_native_properties(|properties| {
    properties.is_maximized = is_maximized;
  });

  let nearest_monitor = state
    .nearest_monitor(&window.native())
    .context("Failed to get workspace of nearest monitor.")?;

  // A tiling window that is being resized keeps its place in the layout,
  // whereas a window that is moved can be maximized or made fullscreen.
  let is_tiling_resize = window.is_tiling_window()
    && active_drag.operation != Some(ActiveDragOperation::Move);

  let should_fullscreen = !is_tiling_resize
    && window.should_fullscreen(
      &nearest_monitor
        .displayed_workspace()
        .context("No workspace.")?,
    )?;

  if !is_tiling_resize && (is_maximized || should_fullscreen) {
    let fullscreen_state =
      if let WindowState::Fullscreen(fullscreen_state) = window.state() {
        fullscreen_state
      } else {
        config
          .value
          .window_behavior
          .state_defaults
          .fullscreen
          .clone()
      };

    let window = update_window_state(
      window.clone(),
      WindowState::Fullscreen(FullscreenStateConfig {
        maximized: is_maximized,
        ..fullscreen_state
      }),
      state,
    )?;

    window.set_active_drag(None);

    if is_maximized {
      // Dequeue the window from redraw if it's maximized, since the
      // window is already in the correct state.
      state
        .pending_sync
        .dequeue_container_from_redraw(window.clone());
    } else {
      // Force a redraw to snap the window to the monitor edges.
      // TODO: Skip redraw if it's already matches fullscreen frame.
      state.pending_sync.queue_container_to_redraw(window.clone());
    }

    return Ok(());
  }

  match &window {
    WindowContainer::NonTilingWindow(window) => {
      if active_drag.is_from_floating {
        update_floating_window_position(
          window,
          window.native_properties().frame,
          &nearest_monitor,
          state,
        )?;
        window.set_active_drag(None);
      } else {
        // Window is a temporary floating window (i.e. a fullscreen window
        // that was dragged) that should be reverted back to tiling.
        let window = drop_as_tiling_window(window, state)?;
        window.set_active_drag(None);
      }
    }
    WindowContainer::TilingWindow(window) => {
      tracing::info!(
        "Tiling window move/resize ended: {}",
        window.as_window_container()?
      );

      let origin_workspace =
        window.workspace().context("No workspace.")?;

      match active_drag.operation {
        Some(ActiveDragOperation::Move) => {
          drop_tiling_window(window, state)?;
        }
        Some(ActiveDragOperation::Resize) => {
          // The window's frame includes its borders, whereas its layout
          // rect does not.
          let target_rect = window
            .native_properties()
            .frame
            .apply_delta(&window.total_border_delta()?.inverse(), None);

          resize_tiling_window(
            window,
            Some(target_rect.width()),
            Some(target_rect.height()),
            state,
          )?;
        }
        None => {}
      }

      window.set_active_drag(None);

      // Force a redraw of the workspaces to snap the window back to its
      // layout position. This is necessary since the window was moved by
      // the user, and is not necessarily changed by the above (e.g. when
      // it is dropped on itself or the resize has no effect).
      state
        .pending_sync
        .queue_container_to_redraw(origin_workspace);

      if let Some(workspace) = window.workspace() {
        state.pending_sync.queue_container_to_redraw(workspace);
      }
    }
  }

  Ok(())
}

/// Handles a tiling window being dropped after a move.
///
/// Drops over another tiling window of the same workspace swap the two
/// windows. Drops over another monitor's workspace move the window there,
/// at the position of the nearest window (or at the end if the workspace
/// has no tiling windows). Elsewhere, the window returns to its position.
///
/// Unlike `move_window_to_workspace`, the dropped window keeps focus.
fn drop_tiling_window(
  window: &TilingWindow,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let mouse_pos = state.dispatcher.cursor_position()?;
  let origin_workspace = window.workspace().context("No workspace.")?;

  let target_workspace = state
    .monitor_at_point(&mouse_pos)
    .and_then(|monitor| monitor.displayed_workspace())
    .unwrap_or_else(|| origin_workspace.clone());

  if target_workspace.id() == origin_workspace.id() {
    if let Some(target) = tiling_window_at_point(
      &target_workspace,
      &mouse_pos,
      window.id(),
      false,
    )? {
      swap_tiling_windows(window, &target, state)?;
    }

    return Ok(());
  }

  let origin_monitor =
    origin_workspace.monitor().context("No monitor.")?;
  let target_monitor =
    target_workspace.monitor().context("No monitor.")?;

  // Since the window is crossing monitors, adjustments might need to be
  // made because of DPI.
  if origin_monitor.has_dpi_difference(&target_monitor.into())? {
    window.set_has_pending_dpi_adjustment(true);
  }

  // Update floating placement since the window has to cross monitors.
  window.set_floating_placement(
    window
      .floating_placement()
      .translate_to_center(&target_workspace.to_rect()?),
  );

  let target_tiling_index = tiling_window_at_point(
    &target_workspace,
    &mouse_pos,
    window.id(),
    true,
  )?
  .and_then(|target| target.tiling_index())
  .unwrap_or_else(|| target_workspace.tiling_children().count());

  move_container_within_tree(
    &window.as_container(),
    &target_workspace.clone().into(),
    target_workspace.child_index_at_tiling_index(target_tiling_index),
    state,
  )?;

  state
    .pending_sync
    .queue_container_to_redraw(target_workspace.clone())
    .queue_workspace_to_reorder(target_workspace);

  Ok(())
}

/// Gets the tiling window of a workspace that is under the given point,
/// other than the window with the given ID.
///
/// For overlapping layouts, the most recently focused window is preferred
/// since it is on top. If no window is under the point, the window with
/// the nearest center is returned when `or_nearest` is set.
fn tiling_window_at_point(
  workspace: &Workspace,
  point: &Point,
  excluded_id: Uuid,
  or_nearest: bool,
) -> anyhow::Result<Option<TilingWindow>> {
  let candidates = workspace
    .descendant_focus_order()
    .filter_map(|descendant| descendant.as_tiling_window().cloned())
    .filter(|candidate| candidate.id() != excluded_id)
    .map(|candidate| {
      let rect = candidate.to_rect()?;
      anyhow::Ok((candidate, rect))
    })
    .try_collect::<Vec<_>>()?;

  let window_under_point = candidates
    .iter()
    .find(|(_, rect)| rect.contains_point(point));

  if let Some((window, _)) = window_under_point {
    return Ok(Some(window.clone()));
  }

  if !or_nearest {
    return Ok(None);
  }

  Ok(
    candidates
      .into_iter()
      .min_by(|(_, rect_a), (_, rect_b)| {
        let distance_a = rect_a.center_point().distance_between(point);
        let distance_b = rect_b.center_point().distance_between(point);

        distance_a.total_cmp(&distance_b)
      })
      .map(|(window, _)| window),
  )
}

/// Handles transition from temporary floating window to tiling window on
/// drag end.
///
/// The window is inserted at the position of the tiling window under the
/// cursor (or the nearest one), on the workspace of the monitor under the
/// cursor.
fn drop_as_tiling_window(
  moved_window: &NonTilingWindow,
  state: &mut WmState,
) -> anyhow::Result<WindowContainer> {
  tracing::info!(
    "Tiling window drag ended: {}",
    moved_window.as_window_container()?
  );

  let mouse_pos = state.dispatcher.cursor_position()?;
  let mouse_workspace = state
    .monitor_at_point(&mouse_pos)
    .and_then(|monitor| monitor.displayed_workspace())
    .or_else(|| moved_window.workspace())
    .context("Couldn't find workspace for window drop.")?;

  let target_tiling_index = tiling_window_at_point(
    &mouse_workspace,
    &mouse_pos,
    moved_window.id(),
    true,
  )?
  .and_then(|target| target.tiling_index());

  let moved_window = update_window_state(
    moved_window.clone().into(),
    WindowState::Tiling,
    state,
  )?;

  let moved_workspace =
    moved_window.workspace().context("No workspace.")?;

  if moved_workspace.id() != mouse_workspace.id() {
    move_container_within_tree(
      &moved_window.clone().into(),
      &mouse_workspace.clone().into(),
      mouse_workspace.child_count(),
      state,
    )?;
  }

  // Take the position of the window that the cursor is over. If there is
  // none (e.g. in an empty workspace), the window stays at the end.
  if let Some(target_tiling_index) = target_tiling_index {
    move_container_within_tree(
      &moved_window.clone().into(),
      &mouse_workspace.clone().into(),
      mouse_workspace.child_index_at_tiling_index(target_tiling_index),
      state,
    )?;
  }

  state
    .pending_sync
    .queue_container_to_redraw(moved_workspace)
    .queue_container_to_redraw(mouse_workspace.clone())
    .queue_workspace_to_reorder(mouse_workspace);

  Ok(moved_window)
}

#[cfg(test)]
mod tests {
  use wm_common::Layout;

  use super::*;
  use crate::models::Monitor;

  /// Creates a workspace with `count` tiling windows laid out in columns,
  /// on the 1680x1050 mock monitor.
  fn workspace_with_columns(
    count: usize,
  ) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace = Workspace::mock()
      .layout(Layout::Columns)
      .tiling_windows(windows.clone())
      .call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    (workspace, windows)
  }

  #[test]
  fn finds_the_window_under_the_point() {
    let (workspace, windows) = workspace_with_columns(2);
    let point = Point { x: 1500, y: 100 };

    let found =
      tiling_window_at_point(&workspace, &point, Uuid::new_v4(), false)
        .unwrap();

    assert_eq!(found.map(|window| window.id()), Some(windows[1].id()));
  }

  #[test]
  fn ignores_the_excluded_window() {
    let (workspace, windows) = workspace_with_columns(2);
    let point = Point { x: 1500, y: 100 };

    let found =
      tiling_window_at_point(&workspace, &point, windows[1].id(), false)
        .unwrap();

    assert!(found.is_none());
  }

  #[test]
  fn falls_back_to_the_nearest_window_when_requested() {
    let (workspace, windows) = workspace_with_columns(2);

    // Below the workspace, but closest to the first window's center.
    let point = Point { x: 300, y: 5000 };

    assert!(tiling_window_at_point(
      &workspace,
      &point,
      Uuid::new_v4(),
      false
    )
    .unwrap()
    .is_none());

    let found =
      tiling_window_at_point(&workspace, &point, Uuid::new_v4(), true)
        .unwrap();
    assert_eq!(found.map(|window| window.id()), Some(windows[0].id()));
  }

  #[test]
  fn prefers_the_most_recently_focused_window_when_overlapping() {
    let windows = (0..3)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();
    let workspace = Workspace::mock()
      .layout(Layout::Monocle)
      .tiling_windows(windows.clone())
      .call();
    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    // The first attached window is the focused one, so it is on top.
    let point = Point { x: 100, y: 100 };
    let found =
      tiling_window_at_point(&workspace, &point, windows[0].id(), false)
        .unwrap();
    assert_eq!(found.map(|window| window.id()), Some(windows[1].id()));

    let found =
      tiling_window_at_point(&workspace, &point, Uuid::new_v4(), false)
        .unwrap();
    assert_eq!(found.map(|window| window.id()), Some(windows[0].id()));
  }

  #[test]
  fn finds_nothing_in_an_empty_workspace() {
    let (workspace, _) = workspace_with_columns(0);
    let point = Point { x: 100, y: 100 };

    assert!(tiling_window_at_point(
      &workspace,
      &point,
      Uuid::new_v4(),
      true
    )
    .unwrap()
    .is_none());
  }
}
