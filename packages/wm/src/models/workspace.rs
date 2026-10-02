use std::{
  cell::{Ref, RefCell, RefMut},
  collections::VecDeque,
  rc::Rc,
};

use anyhow::Context;
use uuid::Uuid;
use wm_common::{
  ContainerDto, GapsConfig, Layout, LayoutParams, WorkspaceConfig,
  WorkspaceDto,
};
use wm_platform::{Rect, RectDelta};

use crate::{
  impl_common_getters, impl_container_debug,
  models::{Container, TilingWindow, WindowContainer},
  traits::{CommonGetters, PositionGetters},
};

#[derive(Clone)]
pub struct Workspace(Rc<RefCell<WorkspaceInner>>);

#[derive(Debug)]
struct WorkspaceInner {
  id: Uuid,
  parent: Option<Container>,
  children: VecDeque<Container>,
  child_focus_order: VecDeque<Uuid>,
  config: WorkspaceConfig,
  gaps_config: GapsConfig,
  layout: Layout,
  layout_params: LayoutParams,
}

impl Workspace {
  pub fn new(
    config: WorkspaceConfig,
    gaps_config: GapsConfig,
    layout: Layout,
    layout_params: LayoutParams,
  ) -> Self {
    let workspace = WorkspaceInner {
      id: Uuid::new_v4(),
      parent: None,
      children: VecDeque::new(),
      child_focus_order: VecDeque::new(),
      config,
      gaps_config,
      layout,
      layout_params: layout_params.sanitized(),
    };

    Self(Rc::new(RefCell::new(workspace)))
  }

  /// Underlying config for the workspace.
  pub fn config(&self) -> WorkspaceConfig {
    self.0.borrow().config.clone()
  }

  /// Update the underlying config for the workspace.
  pub fn set_config(&self, config: WorkspaceConfig) {
    self.0.borrow_mut().config = config;
  }

  /// Layout used to arrange the tiling windows of the workspace.
  pub fn layout(&self) -> Layout {
    self.0.borrow().layout
  }

  /// Sets the layout used to arrange the tiling windows of the workspace.
  pub fn set_layout(&self, layout: Layout) {
    self.0.borrow_mut().layout = layout;
  }

  /// Tunable parameters of the workspace's layout.
  pub fn layout_params(&self) -> LayoutParams {
    self.0.borrow().layout_params.clone()
  }

  /// Sets the layout parameters of the workspace.
  ///
  /// Values are clamped to their valid ranges.
  pub fn set_layout_params(&self, layout_params: LayoutParams) {
    self.0.borrow_mut().layout_params = layout_params.sanitized();
  }

  /// Whether the workspace is currently displayed by the parent monitor.
  pub fn is_displayed(&self) -> bool {
    self
      .monitor()
      .and_then(|monitor| monitor.displayed_workspace())
      .is_some_and(|workspace| workspace.id() == self.id())
  }

  pub fn set_gaps_config(&self, gaps_config: GapsConfig) {
    self.0.borrow_mut().gaps_config = gaps_config;
  }

  /// Effective outer gaps for this workspace.
  ///
  /// Uses `single_window_outer_gap` when the workspace has a single tiling
  /// window, otherwise falls back to `outer_gap`.
  pub fn outer_gaps(&self) -> RectDelta {
    let is_single_window = self.tiling_children().nth(1).is_none();

    let gaps_config = &self.0.borrow().gaps_config;
    let gaps = if is_single_window {
      gaps_config
        .single_window_outer_gap
        .as_ref()
        .unwrap_or(&gaps_config.outer_gap)
    } else {
      &gaps_config.outer_gap
    };

    // TODO: Should this be scaled by the monitor's DPI?
    gaps.clone()
  }

  /// Gets the bounds of a workspace with the given outer gap config.
  fn workspace_rect_with_gap_config(
    &self,
    outer_gaps: &RectDelta,
  ) -> anyhow::Result<Rect> {
    let monitor =
      self.monitor().context("Workspace has no parent monitor.")?;

    let gaps_config = &self.0.borrow().gaps_config;
    let scale_factor = if gaps_config.scale_with_dpi {
      monitor.native_properties().scale_factor
    } else {
      1.
    };

    // Get the delta between the monitor's bounds and its working area.
    let monitor_bounds = monitor.native_properties().bounds;
    let working_area_delta = monitor
      .native_properties()
      .working_area
      .delta(&monitor_bounds);

    Ok(
      monitor_bounds
        // Scale the gaps if `scale_with_dpi` is enabled. Outer gap config
        // values can be a percentage (relative to the monitor bounds), so
        // the outer gap delta needs to be applied prior to the working
        // area delta.
        .apply_delta(&outer_gaps.inverse(), Some(scale_factor))
        .apply_delta(&working_area_delta, None),
    )
  }

  /// Gets the horizontal and vertical gaps between tiling windows in
  /// pixels.
  ///
  /// The gaps are scaled by the monitor's DPI if `scale_with_dpi` is
  /// enabled. Percentage gaps are relative to the monitor bounds.
  pub fn inner_gaps(&self) -> anyhow::Result<(i32, i32)> {
    let monitor =
      self.monitor().context("Workspace has no parent monitor.")?;

    let monitor_rect = monitor.native_properties().bounds;
    let gaps_config = &self.0.borrow().gaps_config;

    let scale_factor = if gaps_config.scale_with_dpi {
      monitor.native_properties().scale_factor
    } else {
      1.
    };

    Ok((
      gaps_config
        .inner_gap
        .to_px(monitor_rect.width(), Some(scale_factor)),
      gaps_config
        .inner_gap
        .to_px(monitor_rect.height(), Some(scale_factor)),
    ))
  }

  /// Computes the rects of the tiling windows of the workspace.
  ///
  /// Returns one rect per tiling child, in child order, as arranged by the
  /// workspace's layout within its bounds (i.e. excluding outer gaps).
  pub fn layout_rects(&self) -> anyhow::Result<Vec<Rect>> {
    let area = self.to_rect()?;
    let inner_gap = self.inner_gaps()?;
    let count = self.tiling_children().count();

    Ok(self.layout().arrange(
      &self.layout_params(),
      &area,
      count,
      inner_gap,
    ))
  }

  /// Gets the most recently focused tiling window of the workspace.
  pub fn focused_tiling_window(&self) -> Option<TilingWindow> {
    self
      .descendant_focus_order()
      .find_map(|descendant| descendant.as_tiling_window().cloned())
  }

  /// Gets the child index to insert a new tiling window at, such that it
  /// ends up directly after the focused tiling window.
  ///
  /// Appends to the end if the workspace has no tiling windows.
  pub fn index_after_focused_tiling_window(&self) -> usize {
    self
      .focused_tiling_window()
      .map_or_else(|| self.child_count(), |window| window.index() + 1)
  }

  /// Gets the child index to insert a tiling window at, such that it ends
  /// up at `tiling_index` amongst the tiling children.
  ///
  /// Indices past the last tiling window append to the end of the
  /// children.
  pub fn child_index_at_tiling_index(&self, tiling_index: usize) -> usize {
    self
      .tiling_children()
      .nth(tiling_index)
      .map_or_else(|| self.child_count(), |window| window.index())
  }

  /// Gets the maximum bounds of a workspace considering both `outer_gap`
  /// and `single_window_outer_gap` config values.
  pub fn max_workspace_rect(&self) -> anyhow::Result<Rect> {
    let gaps_config = &self.0.borrow().gaps_config;

    // Get the workspace rect using `outer_gap`.
    let multi_window_rect =
      self.workspace_rect_with_gap_config(&gaps_config.outer_gap)?;

    let Some(single_gap) = &gaps_config.single_window_outer_gap else {
      return Ok(multi_window_rect);
    };

    // Get the workspace rect using `single_window_outer_gap`.
    let single_window_rect =
      self.workspace_rect_with_gap_config(single_gap)?;

    Ok(multi_window_rect.union(&single_window_rect))
  }

  pub fn to_dto(&self) -> anyhow::Result<ContainerDto> {
    let rect = self.to_rect()?;
    let config = self.config();

    let children = self
      .children()
      .iter()
      .map(CommonGetters::to_dto)
      .try_collect()?;

    Ok(ContainerDto::Workspace(WorkspaceDto {
      id: self.id(),
      name: config.name,
      display_name: config.display_name,
      parent_id: self.parent().map(|parent| parent.id()),
      children,
      child_focus_order: self.0.borrow().child_focus_order.clone().into(),
      has_focus: self.has_focus(None),
      is_displayed: self.is_displayed(),
      width: rect.width(),
      height: rect.height(),
      x: rect.x(),
      y: rect.y(),
      layout: self.layout(),
      layout_params: self.layout_params(),
    }))
  }
}

impl PartialEq for Workspace {
  fn eq(&self, other: &Self) -> bool {
    self.id() == other.id()
  }
}

impl Eq for Workspace {}

impl_container_debug!(Workspace);
impl_common_getters!(Workspace);

impl PositionGetters for Workspace {
  fn to_rect(&self) -> anyhow::Result<Rect> {
    self.workspace_rect_with_gap_config(&self.outer_gaps())
  }
}

impl std::fmt::Display for Workspace {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Workspace(name={}, layout={:?})",
      self.config().name,
      self.layout(),
    )
  }
}

#[cfg(test)]
mod tests {
  use wm_platform::LengthValue;

  use super::*;
  use crate::{
    commands::container::{attach_container, set_focused_descendant},
    models::{Monitor, NonTilingWindow},
  };

  fn px(amount: i32) -> LengthValue {
    LengthValue::from_px(amount)
  }

  /// Gaps config with a 10px inner gap and 20px outer gaps.
  fn gaps_config() -> GapsConfig {
    GapsConfig {
      scale_with_dpi: false,
      inner_gap: px(10),
      outer_gap: RectDelta::new(px(20), px(20), px(20), px(20)),
      single_window_outer_gap: Some(RectDelta::new(
        px(40),
        px(40),
        px(40),
        px(40),
      )),
    }
  }

  /// Creates a workspace with the given tiling windows on a monitor.
  fn workspace_with_windows(
    layout: Layout,
    count: usize,
  ) -> (Workspace, Vec<TilingWindow>) {
    let windows = (0..count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();

    let workspace = Workspace::mock()
      .layout(layout)
      .gaps_config(gaps_config())
      .tiling_windows(windows.clone())
      .call();

    let _monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();

    (workspace, windows)
  }

  #[test]
  fn layout_rects_arranges_the_tiling_children_within_the_gaps() {
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 3);

    let rects = workspace.layout_rects().unwrap();
    assert_eq!(rects.len(), 3);

    // The area excludes the outer gaps, and the rects use the inner gap.
    let area = workspace.to_rect().unwrap();
    assert_eq!(area.x(), 20);
    assert_eq!(area.y(), 20);
    assert_eq!(
      rects,
      Layout::MasterStack.arrange(
        &workspace.layout_params(),
        &area,
        3,
        (10, 10)
      )
    );

    // Each window gets the rect at its own tiling index.
    for (window, rect) in windows.iter().zip(&rects) {
      assert_eq!(&window.to_rect().unwrap(), rect);
    }
  }

  #[test]
  fn layout_rects_follow_the_layout_and_the_window_count() {
    let (workspace, windows) = workspace_with_windows(Layout::Monocle, 1);

    // A single window uses the single window outer gap.
    let area = workspace.to_rect().unwrap();
    assert_eq!(area.x(), 40);
    assert_eq!(workspace.layout_rects().unwrap(), vec![area.clone()]);

    let second = TilingWindow::mock().call();
    attach_container(
      &second.clone().into(),
      &workspace.clone().into(),
      None,
    )
    .unwrap();

    // Both windows fill the (smaller) multi-window area.
    let area = workspace.to_rect().unwrap();
    assert_eq!(area.x(), 20);
    assert_eq!(windows[0].to_rect().unwrap(), area);
    assert_eq!(second.to_rect().unwrap(), area);

    workspace.set_layout(Layout::Columns);
    assert_ne!(windows[0].to_rect().unwrap(), second.to_rect().unwrap());
  }

  #[test]
  fn tiling_index_ignores_non_tiling_windows() {
    let (workspace, windows) = workspace_with_windows(Layout::Rows, 2);

    let floating = NonTilingWindow::mock().call();
    attach_container(&floating.into(), &workspace.clone().into(), Some(1))
      .unwrap();

    assert_eq!(windows[0].tiling_index(), Some(0));
    assert_eq!(windows[1].tiling_index(), Some(1));
    assert_eq!(windows[1].index(), 2);

    let detached = TilingWindow::mock().call();
    assert_eq!(detached.tiling_index(), None);
    assert!(detached.to_rect().is_err());
  }

  #[test]
  fn index_after_focused_tiling_window() {
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 3);

    // The first attached window is the focused one.
    assert_eq!(workspace.index_after_focused_tiling_window(), 1);

    set_focused_descendant(&windows[2].clone().into(), None);
    assert_eq!(workspace.index_after_focused_tiling_window(), 3);

    set_focused_descendant(&windows[1].clone().into(), None);
    assert_eq!(workspace.index_after_focused_tiling_window(), 2);
  }

  #[test]
  fn index_after_focused_tiling_window_skips_a_focused_floating_window() {
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 2);

    let floating = NonTilingWindow::mock().call();
    attach_container(
      &floating.clone().into(),
      &workspace.clone().into(),
      None,
    )
    .unwrap();
    set_focused_descendant(&windows[0].clone().into(), None);
    set_focused_descendant(&floating.into(), None);

    // The floating window is focused, so the focused tiling window is the
    // most recently focused one.
    assert_eq!(workspace.index_after_focused_tiling_window(), 1);
  }

  #[test]
  fn index_after_focused_tiling_window_appends_without_tiling_windows() {
    let (workspace, _) = workspace_with_windows(Layout::MasterStack, 0);
    assert_eq!(workspace.index_after_focused_tiling_window(), 0);

    let floating = NonTilingWindow::mock().call();
    attach_container(&floating.into(), &workspace.clone().into(), None)
      .unwrap();

    assert_eq!(workspace.index_after_focused_tiling_window(), 1);
  }

  #[test]
  fn child_index_at_tiling_index_maps_over_non_tiling_windows() {
    let (workspace, windows) =
      workspace_with_windows(Layout::MasterStack, 2);

    let floating = NonTilingWindow::mock().call();
    attach_container(&floating.into(), &workspace.clone().into(), Some(1))
      .unwrap();

    // Children: [tiling 0, floating, tiling 1].
    assert_eq!(workspace.child_index_at_tiling_index(0), 0);
    assert_eq!(workspace.child_index_at_tiling_index(1), 2);
    assert_eq!(workspace.child_index_at_tiling_index(2), 3);
    assert_eq!(workspace.child_index_at_tiling_index(9), 3);
    assert_eq!(windows[1].index(), 2);
  }

  #[test]
  fn inner_gaps_use_the_gap_config() {
    let (workspace, _) = workspace_with_windows(Layout::MasterStack, 2);
    assert_eq!(workspace.inner_gaps().unwrap(), (10, 10));

    let percent = GapsConfig {
      inner_gap: "1%".parse().unwrap(),
      ..gaps_config()
    };
    workspace.set_gaps_config(percent);

    // Percentages are relative to the width for the horizontal gap, and
    // to the height for the vertical gap (1680x1050 mock monitor).
    assert_eq!(workspace.inner_gaps().unwrap(), (16, 10));
  }
}
