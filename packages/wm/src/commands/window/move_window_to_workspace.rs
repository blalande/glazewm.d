use anyhow::Context;
use tracing::info;

use crate::{
  commands::{
    container::{move_container_within_tree, set_focused_descendant},
    workspace::activate_workspace,
  },
  models::{WindowContainer, WorkspaceTarget},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Moves a window to the workspace given by the target.
///
/// A tiling window is inserted at `target_index` amongst the tiling
/// windows of the target workspace (appended if the index is past the last
/// one). If no index is given, it is inserted after the target workspace's
/// focused tiling window. Non-tiling windows are always appended.
pub fn move_window_to_workspace(
  window: WindowContainer,
  target: WorkspaceTarget,
  target_index: Option<usize>,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let current_workspace = window.workspace().context("No workspace.")?;
  let current_monitor =
    current_workspace.monitor().context("No monitor.")?;

  let (target_workspace_name, target_workspace) =
    state.workspace_by_target(&current_workspace, target, config)?;

  // Retrieve or activate the target workspace by its name.
  let target_workspace = match target_workspace {
    Some(_) => anyhow::Ok(target_workspace),
    _ => match target_workspace_name {
      Some(name) => {
        activate_workspace(Some(&name), None, state, config)?;

        Ok(state.workspace_by_name(&name))
      }
      _ => Ok(None),
    },
  }?;

  if let Some(target_workspace) = target_workspace {
    if target_workspace.id() == current_workspace.id() {
      return Ok(());
    }

    info!(
      "Moving window to workspace: '{}'.",
      target_workspace.config().name
    );

    let target_monitor =
      target_workspace.monitor().context("No monitor.")?;

    // Since target workspace could be on a different monitor, adjustments
    // might need to be made because of DPI.
    if current_monitor
      .has_dpi_difference(&target_monitor.clone().into())?
    {
      window.set_has_pending_dpi_adjustment(true);
    }

    // Update floating placement if the window has to cross monitors.
    if target_monitor.id() != current_monitor.id() {
      window.set_floating_placement(
        window
          .floating_placement()
          .translate_to_center(&target_workspace.to_rect()?),
      );
    }

    // Focus target is `None` if the window is not focused.
    let focus_target = state.focus_target_after_removal(&window);

    let focus_reset_target = if target_workspace.is_displayed() {
      None
    } else {
      target_monitor.descendant_focus_order().next()
    };

    let insertion_index = if window.is_tiling_window() {
      target_index.map_or_else(
        || target_workspace.index_after_focused_tiling_window(),
        |index| target_workspace.child_index_at_tiling_index(index),
      )
    } else {
      target_workspace.child_count()
    };

    // Insert the window into the target workspace.
    move_container_within_tree(
      &window.clone().into(),
      &target_workspace.clone().into(),
      insertion_index,
      state,
    )?;

    // When moving a focused window within the tree to another workspace,
    // the target workspace will get displayed. If moving the window e.g.
    // from monitor 1 -> 2, and the target workspace is hidden on that
    // monitor, we want to reset focus to the workspace that was displayed
    // on that monitor.
    if let Some(focus_reset_target) = focus_reset_target {
      set_focused_descendant(
        &focus_reset_target,
        Some(&target_monitor.into()),
      );
    }

    // Retain focus within the workspace from where the window was moved.
    if let Some(focus_target) = focus_target {
      set_focused_descendant(&focus_target, None);
      state.pending_sync.queue_focus_change();
    }

    match window {
      WindowContainer::NonTilingWindow(_) => {
        state.pending_sync.queue_container_to_redraw(window);
      }
      WindowContainer::TilingWindow(_) => {
        // Both workspaces need to be fully redrawn, since adding or
        // removing a tiling window moves all the others.
        state
          .pending_sync
          .queue_container_to_redraw(current_workspace)
          .queue_container_to_redraw(target_workspace.clone());
      }
    }

    state
      .pending_sync
      .queue_workspace_to_reorder(target_workspace);
  }

  Ok(())
}
