use anyhow::Context;
use wm_common::WindowState;
use wm_platform::{LengthValue, Rect};

use crate::{
  commands::workspace::resize_master_ratio,
  models::{NonTilingWindow, TilingWindow, WindowContainer},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  wm_state::WmState,
};

/// Arbitrary defaults for minimum floating window dimensions.
const MIN_FLOATING_WIDTH: i32 = 250;
const MIN_FLOATING_HEIGHT: i32 = 140;

pub fn set_window_size(
  window: WindowContainer,
  target_width: Option<LengthValue>,
  target_height: Option<LengthValue>,
  state: &mut WmState,
) -> anyhow::Result<()> {
  match window {
    WindowContainer::TilingWindow(window) => {
      if target_height.is_some() {
        tracing::debug!("Ignoring height resize of a tiling window.");
      }

      if let Some(target_width) = target_width {
        set_tiling_window_width(&window, &target_width, state)?;
      }
    }
    WindowContainer::NonTilingWindow(window) => {
      if matches!(window.state(), WindowState::Floating(_)) {
        set_floating_window_size(
          &window,
          target_width,
          target_height,
          state,
        )?;
      }
    }
  }

  Ok(())
}

/// Updates the width of a tiling window.
///
/// The size of a tiling window is dictated by its workspace's layout, so
/// the width can only be changed by resizing the master area (see
/// `resize_master_ratio`). Relative widths (e.g. percentages) are relative
/// to the workspace.
fn set_tiling_window_width(
  window: &TilingWindow,
  target_width: &LengthValue,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;
  let target_width_px =
    target_width.to_px(workspace.to_rect()?.width(), None);

  resize_master_ratio(window, target_width_px, state)
}

fn set_floating_window_size(
  window: &NonTilingWindow,
  target_width: Option<LengthValue>,
  target_height: Option<LengthValue>,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let monitor = window.monitor().context("No monitor")?;
  let monitor_rect = monitor.to_rect()?;
  let window_rect = window.to_rect()?;

  // Prevent resize from making the window smaller than minimum dimensions.
  // Always allow the size to be increased, even if the window would still
  // be within minimum dimension values.
  let length_with_clamp =
    |target_length: Option<i32>, current_length, min_length| {
      target_length.map_or(current_length, |target_length| {
        if target_length >= current_length {
          target_length
        } else {
          target_length.max(min_length)
        }
      })
    };

  let target_width_px = target_width
    .map(|target_width| target_width.to_px(monitor_rect.width(), None));

  let new_width = length_with_clamp(
    target_width_px,
    window_rect.width(),
    MIN_FLOATING_WIDTH,
  );

  let target_height_px = target_height
    .map(|target_height| target_height.to_px(monitor_rect.height(), None));

  let new_height = length_with_clamp(
    target_height_px,
    window_rect.height(),
    MIN_FLOATING_HEIGHT,
  );

  window.set_floating_placement(Rect::from_xy(
    window.floating_placement().x(),
    window.floating_placement().y(),
    new_width,
    new_height,
  ));

  state.pending_sync.queue_container_to_redraw(window.clone());

  Ok(())
}
