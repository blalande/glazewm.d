use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Layout algorithm used to arrange the tiling windows of a workspace.
#[derive(
  Clone,
  Copy,
  Debug,
  Default,
  Deserialize,
  Eq,
  PartialEq,
  Serialize,
  ValueEnum,
)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Layout {
  /// One master area next to a stack of the remaining windows.
  #[default]
  MasterStack,
  /// Equal-width columns.
  Columns,
  /// Equal-height rows.
  Rows,
  /// Every window fills the workspace.
  Monocle,
  /// Windows laid out on a roughly square grid.
  Grid,
  /// Each window takes half of the remaining space, spiralling inwards.
  Dwindle,
}

impl Layout {
  /// All layouts, in cycle order.
  const CYCLE: [Self; 6] = [
    Self::MasterStack,
    Self::Columns,
    Self::Rows,
    Self::Monocle,
    Self::Grid,
    Self::Dwindle,
  ];

  /// Gets the layout that follows this one in cycle order, wrapping around
  /// at the end.
  ///
  /// Example:
  /// ```
  /// # use wm_common::Layout;
  /// assert_eq!(Layout::MasterStack.next(), Layout::Columns);
  /// assert_eq!(Layout::Dwindle.next(), Layout::MasterStack);
  /// ```
  #[must_use]
  pub fn next(self) -> Self {
    let index = Self::CYCLE
      .iter()
      .position(|layout| *layout == self)
      .unwrap_or(0);

    Self::CYCLE[(index + 1) % Self::CYCLE.len()]
  }
}

/// Tunable parameters shared by the dynamic layouts.
///
/// All fields are optional in the user config.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all(serialize = "camelCase"))]
pub struct LayoutParams {
  /// Fraction of the workspace taken by the master area.
  #[serde(alias = "masterRatio")]
  pub master_ratio: f32,

  /// Number of windows in the master area.
  #[serde(alias = "masterCount")]
  pub master_count: usize,

  /// Whether to mirror the layout (e.g. master on the right instead of
  /// the left).
  pub flip: bool,
}

impl LayoutParams {
  /// Smallest allowed `master_ratio`.
  pub const MIN_MASTER_RATIO: f32 = 0.1;

  /// Largest allowed `master_ratio`.
  pub const MAX_MASTER_RATIO: f32 = 0.9;

  /// Returns a copy with `master_ratio` and `master_count` clamped to
  /// their valid ranges.
  #[must_use]
  pub fn sanitized(mut self) -> Self {
    self.master_ratio = Self::clamp_ratio(self.master_ratio);
    self.master_count = self.master_count.max(1);
    self
  }

  /// Clamps a master ratio to the valid range.
  ///
  /// Non-finite values fall back to the default ratio.
  #[must_use]
  pub fn clamp_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
      ratio.clamp(Self::MIN_MASTER_RATIO, Self::MAX_MASTER_RATIO)
    } else {
      Self::default().master_ratio
    }
  }

  /// Adjusts the master ratio by a delta, keeping it in the valid range.
  pub fn adjust_master_ratio(&mut self, delta: f32) {
    self.master_ratio = Self::clamp_ratio(self.master_ratio + delta);
  }

  /// Adjusts the master count by a delta, keeping it at 1 or above.
  pub fn adjust_master_count(&mut self, delta: i32) {
    let count = i64::try_from(self.master_count).unwrap_or(i64::MAX);

    self.master_count =
      usize::try_from(count.saturating_add(i64::from(delta)).max(1))
        .unwrap_or(usize::MAX);
  }
}

impl Default for LayoutParams {
  fn default() -> Self {
    Self {
      master_ratio: 0.55,
      master_count: 1,
      flip: false,
    }
  }
}

/// Relative position in an ordered list of windows.
#[derive(
  Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum,
)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Ordinal {
  Next,
  Previous,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn layout_cycles_and_wraps() {
    let mut layout = Layout::default();
    let mut seen = vec![layout];

    for _ in 0..Layout::CYCLE.len() - 1 {
      layout = layout.next();
      seen.push(layout);
    }

    assert_eq!(seen, Layout::CYCLE);
    assert_eq!(layout.next(), Layout::MasterStack);
  }

  #[test]
  fn params_defaults() {
    let params = LayoutParams::default();
    assert!((params.master_ratio - 0.55).abs() < f32::EPSILON);
    assert_eq!(params.master_count, 1);
    assert!(!params.flip);
  }

  #[test]
  fn params_partial_yaml_uses_defaults() {
    let params: LayoutParams =
      serde_yaml::from_str("master_count: 2").unwrap();
    assert_eq!(params.master_count, 2);
    assert!((params.master_ratio - 0.55).abs() < f32::EPSILON);
  }

  #[test]
  fn ratio_is_clamped() {
    let mut params = LayoutParams::default();
    params.adjust_master_ratio(10.);
    assert!(
      (params.master_ratio - LayoutParams::MAX_MASTER_RATIO).abs()
        < f32::EPSILON
    );
    params.adjust_master_ratio(-10.);
    assert!(
      (params.master_ratio - LayoutParams::MIN_MASTER_RATIO).abs()
        < f32::EPSILON
    );
    assert!((LayoutParams::clamp_ratio(f32::NAN) - 0.55).abs() < 1e-6);
  }

  #[test]
  fn count_has_minimum_of_one() {
    let mut params = LayoutParams::default();
    params.adjust_master_count(-5);
    assert_eq!(params.master_count, 1);
    params.adjust_master_count(2);
    assert_eq!(params.master_count, 3);
  }

  #[test]
  fn sanitized_clamps_out_of_range_values() {
    let params = LayoutParams {
      master_ratio: 5.,
      master_count: 0,
      flip: true,
    }
    .sanitized();
    assert!(
      (params.master_ratio - LayoutParams::MAX_MASTER_RATIO).abs()
        < f32::EPSILON
    );
    assert_eq!(params.master_count, 1);
  }
}
