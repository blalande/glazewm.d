use std::{
  cell::{Ref, RefCell, RefMut},
  collections::VecDeque,
  rc::Rc,
};

use anyhow::Context;
use uuid::Uuid;
use wm_common::{
  ActiveDrag, ContainerDto, DisplayState, WindowDto, WindowRuleConfig,
  WindowState,
};
use wm_platform::{NativeWindow, Rect, RectDelta};

use crate::{
  impl_common_getters, impl_container_debug, impl_window_getters,
  models::{
    Container, NativeWindowProperties, NonTilingWindow, WindowContainer,
  },
  traits::{CommonGetters, PositionGetters, WindowGetters},
};

#[derive(Clone)]
pub struct TilingWindow(Rc<RefCell<TilingWindowInner>>);

struct TilingWindowInner {
  id: Uuid,
  parent: Option<Container>,
  children: VecDeque<Container>,
  child_focus_order: VecDeque<Uuid>,
  native: NativeWindow,
  native_properties: NativeWindowProperties,
  state: WindowState,
  prev_state: Option<WindowState>,
  display_state: DisplayState,
  border_delta: RectDelta,
  has_pending_dpi_adjustment: bool,
  floating_placement: Rect,
  has_custom_floating_placement: bool,
  done_window_rules: Vec<WindowRuleConfig>,
  active_drag: Option<ActiveDrag>,
}

impl TilingWindow {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    id: Option<Uuid>,
    native: NativeWindow,
    properties: NativeWindowProperties,
    prev_state: Option<WindowState>,
    border_delta: RectDelta,
    floating_placement: Rect,
    has_custom_floating_placement: bool,
    done_window_rules: Vec<WindowRuleConfig>,
    active_drag: Option<ActiveDrag>,
  ) -> Self {
    let window = TilingWindowInner {
      id: id.unwrap_or_else(Uuid::new_v4),
      parent: None,
      children: VecDeque::new(),
      child_focus_order: VecDeque::new(),
      native,
      native_properties: properties,
      state: WindowState::Tiling,
      prev_state,
      display_state: DisplayState::Shown,
      border_delta,
      has_pending_dpi_adjustment: false,
      floating_placement,
      has_custom_floating_placement,
      done_window_rules,
      active_drag,
    };

    Self(Rc::new(RefCell::new(window)))
  }

  pub fn to_non_tiling(&self, state: WindowState) -> NonTilingWindow {
    NonTilingWindow::new(
      Some(self.id()),
      self.native().clone(),
      self.native_properties().clone(),
      state,
      Some(WindowState::Tiling),
      self.border_delta(),
      self.floating_placement(),
      self.has_custom_floating_placement(),
      self.done_window_rules(),
      self.active_drag(),
    )
  }

  pub fn to_dto(&self) -> anyhow::Result<ContainerDto> {
    let rect = self.to_rect()?;

    Ok(ContainerDto::Window(WindowDto {
      id: self.id(),
      parent_id: self.parent().map(|parent| parent.id()),
      has_focus: self.has_focus(None),
      width: rect.width(),
      height: rect.height(),
      x: rect.x(),
      y: rect.y(),
      state: self.state(),
      prev_state: self.prev_state(),
      display_state: self.display_state(),
      border_delta: self.border_delta(),
      floating_placement: self.floating_placement(),
      #[allow(clippy::cast_possible_wrap, clippy::unnecessary_cast)]
      handle: self.native().id().0 as isize,
      title: self.native_properties().title,
      #[cfg(target_os = "windows")]
      class_name: self.native_properties().class_name,
      process_name: self.native_properties().process_name,
      active_drag: self.active_drag(),
    }))
  }
}

impl_container_debug!(TilingWindow);
impl_common_getters!(TilingWindow);
impl_window_getters!(TilingWindow);

impl TilingWindow {
  /// Index of this window amongst the tiling children of its workspace.
  ///
  /// This is the index of the window's rect in
  /// [`Workspace::layout_rects`]. Returns `None` if the window is
  /// detached.
  ///
  /// [`Workspace::layout_rects`]: crate::models::Workspace::layout_rects
  pub fn tiling_index(&self) -> Option<usize> {
    self
      .parent()?
      .tiling_children()
      .position(|sibling| sibling.id() == self.id())
  }
}

impl PositionGetters for TilingWindow {
  /// Gets the rect that the layout of the workspace assigns to the window.
  fn to_rect(&self) -> anyhow::Result<Rect> {
    let workspace = self
      .workspace()
      .context("Tiling window has no workspace.")?;

    let index = self
      .tiling_index()
      .context("Tiling window is not attached to a workspace.")?;

    // ponytail: Every window computes the rects of the whole workspace, so
    // a redraw is O(n^2) in the number of tiling windows. This is fine for
    // the ~20 windows of a workspace; if it shows up in a profile, cache
    // the rects per workspace for the duration of a redraw in
    // `pending_sync`/`platform_sync`.
    workspace
      .layout_rects()?
      .into_iter()
      .nth(index)
      .context("Layout produced no rect for the tiling window.")
  }
}
