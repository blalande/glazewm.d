use anyhow::Context;
use wm_platform::LengthValue;

use super::set_window_size;
use crate::{
  models::WindowContainer,
  traits::{CommonGetters, PositionGetters},
  wm_state::WmState,
};

/// Resizes a window by the given width and height deltas.
///
/// Relative deltas (e.g. percentages) are relative to the window's
/// workspace. See `set_window_size` for how tiling windows are resized.
pub fn resize_window(
  window: &WindowContainer,
  width_delta: Option<LengthValue>,
  height_delta: Option<LengthValue>,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let window_rect = window.to_rect()?;
  let parent_rect = window.parent().context("No parent.")?.to_rect()?;

  let target_width = width_delta.map(|delta| {
    window_rect.width() + delta.to_px(parent_rect.width(), None)
  });

  let target_height = height_delta.map(|delta| {
    window_rect.height() + delta.to_px(parent_rect.height(), None)
  });

  set_window_size(
    window.clone(),
    target_width.map(LengthValue::from_px),
    target_height.map(LengthValue::from_px),
    state,
  )?;

  Ok(())
}
