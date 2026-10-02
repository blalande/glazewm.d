use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use wm_platform::Rect;

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

impl Layout {
  /// Computes the rects of the tiling windows within `area`, given one
  /// `SizeWeight` per window.
  ///
  /// `inner_gap` is `(horizontal, vertical)`: the gap in pixels between
  /// side-by-side and stacked windows respectively. Outer gaps are the
  /// caller's concern (`area` must already be inset).
  ///
  /// The weights size the columns of `Columns`, the rows of `Rows`, and
  /// both the columns and the rows within a column of `Grid` (a column is
  /// as wide as the mean width weight of its windows). Other layouts
  /// ignore them.
  ///
  /// Returns exactly one rect per weight, in child order (index 0 is the
  /// first window, i.e. the master). Rects in a row or column tile `area`
  /// exactly, with the last one absorbing the rounding remainder. Widths
  /// and heights are clamped to at least 1 px, so rects can overflow
  /// `area` when the gaps exceed it.
  #[must_use]
  pub fn arrange(
    &self,
    params: &LayoutParams,
    area: &Rect,
    weights: &[SizeWeight],
    inner_gap: (i32, i32),
  ) -> Vec<Rect> {
    let count = weights.len();

    if count == 0 {
      return Vec::new();
    }

    let (gap_x, gap_y) = inner_gap;
    let ratio = params.master_ratio;
    let widths = weights.iter().map(|w| w.width).collect::<Vec<_>>();
    let heights = weights.iter().map(|w| w.height).collect::<Vec<_>>();

    let mut rects = match self {
      Self::MasterStack => {
        let masters = params.master_count.max(1).min(count);
        let stack = count - masters;

        if stack == 0 {
          column(area, area.x(), area.width(), masters, gap_y)
        } else {
          let cols = split_span(area.width(), gap_x, &[ratio, 1. - ratio]);
          let mut rects =
            column(area, area.x() + cols[0].0, cols[0].1, masters, gap_y);
          rects.extend(column(
            area,
            area.x() + cols[1].0,
            cols[1].1,
            stack,
            gap_y,
          ));
          rects
        }
      }
      Self::Columns => split_span(area.width(), gap_x, &widths)
        .into_iter()
        .map(|(x, w)| {
          Rect::from_xy(area.x() + x, area.y(), w, area.height())
        })
        .collect(),
      Self::Rows => split_span(area.height(), gap_y, &heights)
        .into_iter()
        .map(|(y, h)| {
          Rect::from_xy(area.x(), area.y() + y, area.width(), h)
        })
        .collect(),
      Self::Monocle => return vec![area.clone(); count],
      Self::Grid => {
        split_span(area.width(), gap_x, &grid_column_widths(weights))
          .into_iter()
          .zip(grid_columns(count))
          .flat_map(|((x, w), range)| {
            split_span(area.height(), gap_y, &heights[range])
              .into_iter()
              .map(move |(y, h)| {
                Rect::from_xy(area.x() + x, area.y() + y, w, h)
              })
          })
          .collect()
      }
      Self::Dwindle => dwindle(area, ratio, count, inner_gap),
    };

    if params.flip {
      // Mirroring a tiled run keeps it tiled exactly.
      for rect in &mut rects {
        *rect = if *self == Self::Rows {
          Rect::from_ltrb(
            rect.left,
            area.top + area.bottom - rect.bottom,
            rect.right,
            area.top + area.bottom - rect.top,
          )
        } else {
          Rect::from_ltrb(
            area.left + area.right - rect.right,
            rect.top,
            area.left + area.right - rect.left,
            rect.bottom,
          )
        };
      }
    }

    rects
  }
}

/// Arranges `count` windows by repeatedly splitting `area`.
///
/// Each window but the last takes the first part of the remaining area,
/// alternating between a left/right and a top/bottom split. The first
/// split uses `ratio`, later ones 0.5. The last window takes what remains.
fn dwindle(
  area: &Rect,
  ratio: f32,
  count: usize,
  (gap_x, gap_y): (i32, i32),
) -> Vec<Rect> {
  let mut rects = Vec::with_capacity(count);
  let mut rest = area.clone();

  for i in 0..count.saturating_sub(1) {
    let weights = if i == 0 {
      [ratio, 1. - ratio]
    } else {
      [0.5, 0.5]
    };

    if i % 2 == 0 {
      let parts = split_span(rest.width(), gap_x, &weights);
      rects.push(Rect::from_xy(
        rest.x(),
        rest.y(),
        parts[0].1,
        rest.height(),
      ));
      rest = Rect::from_xy(
        rest.x() + parts[1].0,
        rest.y(),
        parts[1].1,
        rest.height(),
      );
    } else {
      let parts = split_span(rest.height(), gap_y, &weights);
      rects.push(Rect::from_xy(
        rest.x(),
        rest.y(),
        rest.width(),
        parts[0].1,
      ));
      rest = Rect::from_xy(
        rest.x(),
        rest.y() + parts[1].0,
        rest.width(),
        parts[1].1,
      );
    }
  }

  rects.push(rest);
  rects
}

/// Equal weights for `count` parts.
fn equal(count: usize) -> Vec<f32> {
  vec![1.; count]
}

/// Gets the width weight of each column of the `Grid` layout (see
/// `grid_columns`): the mean width weight of the column's windows.
#[must_use]
pub fn grid_column_widths(weights: &[SizeWeight]) -> Vec<f32> {
  grid_columns(weights.len())
    .into_iter()
    .map(|range| {
      #[allow(clippy::cast_precision_loss)]
      let len = range.len() as f32;
      weights[range].iter().map(|w| w.width).sum::<f32>() / len
    })
    .collect()
}

/// Splits `count` windows into the columns of the `Grid` layout.
///
/// Returns the range of window indices in each column, from left to
/// right. The grid has `ceil(sqrt(count))` columns, and the leftmost
/// columns get one extra window when `count` doesn't divide evenly.
#[must_use]
pub fn grid_columns(count: usize) -> Vec<std::ops::Range<usize>> {
  let mut cols = 1;
  while cols * cols < count {
    cols += 1;
  }

  let (base, extra) = (count / cols, count % cols);
  let mut start = 0;

  (0..cols)
    .map(|i| {
      let end = start + base + usize::from(i < extra);
      let range = start..end;
      start = end;
      range
    })
    .filter(|range| !range.is_empty())
    .collect()
}

/// Stacks `rows` equal-height rects vertically in the column at `x` with
/// the given `width`, spanning the height of `area`.
fn column(
  area: &Rect,
  x: i32,
  width: i32,
  rows: usize,
  gap_y: i32,
) -> Vec<Rect> {
  split_span(area.height(), gap_y, &equal(rows))
    .into_iter()
    .map(|(y, h)| Rect::from_xy(x, area.y() + y, width, h))
    .collect()
}

/// Splits a span of `span` pixels into `weights.len()` parts separated by
/// `gap` pixels.
///
/// Returns `(offset, length)` pairs relative to the span start. Each part
/// is sized proportionally to its weight, rounded down; the last part
/// absorbs the rounding remainder so the parts and gaps cover the span
/// exactly. Lengths are clamped to at least 1 px, so the parts can
/// overflow the span when the gaps exceed it.
fn split_span(span: i32, gap: i32, weights: &[f32]) -> Vec<(i32, i32)> {
  let Some(last) = weights.len().checked_sub(1) else {
    return Vec::new();
  };

  let gap = i64::from(gap.max(0));
  let gaps = gap.saturating_mul(i64::try_from(last).unwrap_or(i64::MAX));
  let available = i64::from(span).saturating_sub(gaps);
  let total: f64 = weights.iter().map(|w| f64::from(*w)).sum();

  let mut parts = Vec::with_capacity(weights.len());
  let (mut offset, mut used) = (0_i64, 0_i64);

  for (i, weight) in weights.iter().enumerate() {
    let length = if i == last {
      available - used
    } else {
      // Truncation is intended: the value is floored and bounded by
      // `available`.
      #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss
      )]
      {
        (available as f64 * f64::from(*weight) / total).floor() as i64
      }
    }
    .max(1);

    parts.push((
      i32::try_from(offset).unwrap_or(i32::MAX),
      i32::try_from(length).unwrap_or(i32::MAX),
    ));
    offset = offset.saturating_add(length).saturating_add(gap);
    used = used.saturating_add(length);
  }

  parts
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

/// Relative size of a tiling window, used by the layouts that share space
/// between windows (see `Layout::arrange`).
///
/// A window with twice the weight of its neighbours gets twice their
/// width (or height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeWeight {
  pub width: f32,
  pub height: f32,
}

impl SizeWeight {
  /// Smallest allowed weight.
  pub const MIN: f32 = 0.1;

  /// Largest allowed weight.
  pub const MAX: f32 = 10.;

  /// Clamps a weight to the valid range.
  ///
  /// Non-finite values fall back to the default weight of 1.
  #[must_use]
  pub fn clamp(weight: f32) -> f32 {
    if weight.is_finite() {
      weight.clamp(Self::MIN, Self::MAX)
    } else {
      1.
    }
  }

  /// Computes the weight that sizes a part to `target` pixels, out of
  /// `available` pixels (gaps excluded) shared with other parts whose
  /// weights sum to `others`.
  ///
  /// The target is kept within the span, and the result is clamped to the
  /// valid range. Returns `None` if the part has no other part to share
  /// the span with, or if the span is too small to resize.
  #[must_use]
  pub fn for_length(
    target: i32,
    available: i32,
    others: f32,
  ) -> Option<f32> {
    if others <= 0. || available <= 1 {
      return None;
    }

    #[allow(clippy::cast_precision_loss)]
    let (target, available) =
      (target.clamp(1, available - 1) as f32, available as f32);

    Some(Self::clamp(target * others / (available - target)))
  }
}

impl Default for SizeWeight {
  fn default() -> Self {
    Self {
      width: 1.,
      height: 1.,
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

impl Ordinal {
  /// Gets the ordinal selected by a pair of mutually exclusive `next` and
  /// `previous` flags.
  #[must_use]
  pub fn from_flags(next: bool, previous: bool) -> Option<Self> {
    match (next, previous) {
      (true, false) => Some(Self::Next),
      (false, true) => Some(Self::Previous),
      _ => None,
    }
  }

  /// Steps from `index` in a list of `len` items, wrapping around at both
  /// ends.
  ///
  /// Returns 0 for an empty list.
  ///
  /// Example:
  /// ```
  /// # use wm_common::Ordinal;
  /// assert_eq!(Ordinal::Next.wrapping_step(2, 3), 0);
  /// assert_eq!(Ordinal::Previous.wrapping_step(0, 3), 2);
  /// ```
  #[must_use]
  pub fn wrapping_step(self, index: usize, len: usize) -> usize {
    match self {
      Self::Next => (index + 1).checked_rem(len).unwrap_or(0),
      Self::Previous if index == 0 => len.saturating_sub(1),
      Self::Previous => index - 1,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const GAP: (i32, i32) = (10, 10);

  fn area() -> Rect {
    Rect::from_xy(0, 0, 1000, 600)
  }

  fn params(ratio: f32, master_count: usize, flip: bool) -> LayoutParams {
    LayoutParams {
      master_ratio: ratio,
      master_count,
      flip,
    }
  }

  fn rects(list: &[(i32, i32, i32, i32)]) -> Vec<Rect> {
    list
      .iter()
      .map(|&(x, y, w, h)| Rect::from_xy(x, y, w, h))
      .collect()
  }

  fn ones(count: usize) -> Vec<SizeWeight> {
    vec![SizeWeight::default(); count]
  }

  fn weights(list: &[(f32, f32)]) -> Vec<SizeWeight> {
    list
      .iter()
      .map(|&(width, height)| SizeWeight { width, height })
      .collect()
  }

  #[test]
  fn arrange_columns_and_rows_follow_the_weights() {
    let weights = weights(&[(1., 3.), (2., 1.), (1., 1.)]);

    let result = Layout::Columns.arrange(
      &LayoutParams::default(),
      &area(),
      &weights,
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[(0, 0, 250, 600), (250, 0, 500, 600), (750, 0, 250, 600)])
    );

    let result = Layout::Rows.arrange(
      &LayoutParams::default(),
      &area(),
      &weights,
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[
        (0, 0, 1000, 360),
        (0, 360, 1000, 120),
        (0, 480, 1000, 120)
      ])
    );
  }

  #[test]
  fn arrange_grid_follows_the_weights() {
    // Columns hold windows [0, 1] and [2]. The first column is as wide as
    // the mean width weight of its windows.
    let weights = weights(&[(1., 3.), (3., 1.), (2., 1.)]);

    let result = Layout::Grid.arrange(
      &LayoutParams::default(),
      &area(),
      &weights,
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[(0, 0, 500, 450), (0, 450, 500, 150), (500, 0, 500, 600)])
    );
  }

  #[test]
  fn grid_columns_split_the_windows() {
    assert!(grid_columns(0).is_empty());
    assert_eq!(grid_columns(1), vec![0..1]);
    assert_eq!(grid_columns(3), vec![0..2, 2..3]);
    assert_eq!(grid_columns(5), vec![0..2, 2..4, 4..5]);
  }

  #[test]
  fn weight_for_length() {
    // Two parts of weight 1 over 1000px: 750px needs 3 times the other.
    let weight = SizeWeight::for_length(750, 1000, 1.).unwrap();
    assert!((weight - 3.).abs() < 1e-6);

    // The target and the result are clamped.
    assert!(
      (SizeWeight::for_length(5000, 1000, 1.).unwrap() - SizeWeight::MAX)
        .abs()
        < 1e-6
    );
    assert!(
      (SizeWeight::for_length(-10, 1000, 1.).unwrap() - SizeWeight::MIN)
        .abs()
        < 1e-6
    );

    // A part alone in its span can't be resized.
    assert_eq!(SizeWeight::for_length(500, 1000, 0.), None);
    assert_eq!(SizeWeight::for_length(500, 1, 1.), None);
  }

  #[test]
  fn ordinal_wraps_in_both_directions() {
    assert_eq!(Ordinal::Next.wrapping_step(0, 3), 1);
    assert_eq!(Ordinal::Next.wrapping_step(2, 3), 0);
    assert_eq!(Ordinal::Previous.wrapping_step(2, 3), 1);
    assert_eq!(Ordinal::Previous.wrapping_step(0, 3), 2);
    assert_eq!(Ordinal::Next.wrapping_step(0, 1), 0);
    assert_eq!(Ordinal::Previous.wrapping_step(0, 1), 0);
    assert_eq!(Ordinal::Next.wrapping_step(0, 0), 0);
    assert_eq!(Ordinal::Previous.wrapping_step(0, 0), 0);
  }

  #[test]
  fn ordinal_from_flags() {
    assert_eq!(Ordinal::from_flags(true, false), Some(Ordinal::Next));
    assert_eq!(Ordinal::from_flags(false, true), Some(Ordinal::Previous));
    assert_eq!(Ordinal::from_flags(false, false), None);
    assert_eq!(Ordinal::from_flags(true, true), None);
  }

  #[test]
  fn arrange_zero_windows_is_empty() {
    for layout in Layout::CYCLE {
      let result =
        layout.arrange(&LayoutParams::default(), &area(), &ones(0), GAP);
      assert!(result.is_empty(), "{layout:?}");
    }
  }

  #[test]
  fn arrange_single_window_fills_area() {
    for layout in Layout::CYCLE {
      for flip in [false, true] {
        let result =
          layout.arrange(&params(0.55, 1, flip), &area(), &ones(1), GAP);
        assert_eq!(result, vec![area()], "{layout:?} flip={flip}");
      }
    }
  }

  #[test]
  fn arrange_master_stack() {
    let cases = [
      (
        "2 windows",
        params(0.5, 1, false),
        2,
        rects(&[(0, 0, 495, 600), (505, 0, 495, 600)]),
      ),
      (
        "3 windows",
        params(0.5, 1, false),
        3,
        rects(&[
          (0, 0, 495, 600),
          (505, 0, 495, 295),
          (505, 305, 495, 295),
        ]),
      ),
      (
        "2 masters, 2 windows",
        params(0.5, 2, false),
        2,
        rects(&[(0, 0, 1000, 295), (0, 305, 1000, 295)]),
      ),
      (
        "2 masters, 3 windows",
        params(0.5, 2, false),
        3,
        rects(&[(0, 0, 495, 295), (0, 305, 495, 295), (505, 0, 495, 600)]),
      ),
      (
        "flip, 3 windows",
        params(0.5, 1, true),
        3,
        rects(&[(505, 0, 495, 600), (0, 0, 495, 295), (0, 305, 495, 295)]),
      ),
    ];

    for (name, params, count, expected) in cases {
      let result =
        Layout::MasterStack.arrange(&params, &area(), &ones(count), GAP);
      assert_eq!(result, expected, "{name}");
    }
  }

  #[test]
  fn arrange_columns_remainder_goes_to_last() {
    let area = Rect::from_xy(0, 0, 1001, 600);
    let result = Layout::Columns.arrange(
      &LayoutParams::default(),
      &area,
      &ones(3),
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[(0, 0, 333, 600), (333, 0, 333, 600), (666, 0, 335, 600)])
    );

    let result = Layout::Columns.arrange(
      &params(0.55, 1, true),
      &area,
      &ones(3),
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[(668, 0, 333, 600), (335, 0, 333, 600), (0, 0, 335, 600)])
    );
  }

  #[test]
  fn arrange_rows_remainder_goes_to_last() {
    let area = Rect::from_xy(0, 0, 1000, 601);
    let result = Layout::Rows.arrange(
      &LayoutParams::default(),
      &area,
      &ones(3),
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[
        (0, 0, 1000, 200),
        (0, 200, 1000, 200),
        (0, 400, 1000, 201)
      ])
    );

    let result = Layout::Rows.arrange(
      &params(0.55, 1, true),
      &area,
      &ones(3),
      (0, 0),
    );
    assert_eq!(
      result,
      rects(&[
        (0, 401, 1000, 200),
        (0, 201, 1000, 200),
        (0, 0, 1000, 201)
      ])
    );
  }

  #[test]
  fn arrange_monocle_fills_area() {
    let result = Layout::Monocle.arrange(
      &LayoutParams::default(),
      &area(),
      &ones(3),
      GAP,
    );
    assert_eq!(result, vec![area(); 3]);
  }

  #[test]
  fn arrange_grid() {
    let cases = [
      (
        3,
        rects(&[(0, 0, 495, 295), (0, 305, 495, 295), (505, 0, 495, 600)]),
      ),
      (
        4,
        rects(&[
          (0, 0, 495, 295),
          (0, 305, 495, 295),
          (505, 0, 495, 295),
          (505, 305, 495, 295),
        ]),
      ),
      (
        5,
        rects(&[
          (0, 0, 326, 295),
          (0, 305, 326, 295),
          (336, 0, 326, 295),
          (336, 305, 326, 295),
          (672, 0, 328, 600),
        ]),
      ),
    ];

    for (count, expected) in cases {
      let result = Layout::Grid.arrange(
        &LayoutParams::default(),
        &area(),
        &ones(count),
        GAP,
      );
      assert_eq!(result, expected, "{count} windows");
    }
  }

  #[test]
  fn arrange_dwindle() {
    let result = Layout::Dwindle.arrange(
      &params(0.5, 1, false),
      &area(),
      &ones(3),
      GAP,
    );
    assert_eq!(
      result,
      rects(&[(0, 0, 495, 600), (505, 0, 495, 295), (505, 305, 495, 295)])
    );

    let result = Layout::Dwindle.arrange(
      &params(0.5, 1, true),
      &area(),
      &ones(3),
      GAP,
    );
    assert_eq!(
      result,
      rects(&[(505, 0, 495, 600), (0, 0, 495, 295), (0, 305, 495, 295)])
    );
  }

  #[test]
  fn arrange_clamps_to_one_pixel() {
    let area = Rect::from_xy(0, 0, 10, 10);
    let result = Layout::Columns.arrange(
      &LayoutParams::default(),
      &area,
      &ones(3),
      (20, 20),
    );
    assert!(result.iter().all(|r| r.width() >= 1 && r.height() >= 1));
  }

  #[test]
  fn arrange_rects_stay_inside_area_without_overlap() {
    let area = area();

    for layout in Layout::CYCLE {
      if layout == Layout::Monocle {
        continue;
      }

      for (master_count, flip) in
        [(1, false), (1, true), (2, false), (3, true)]
      {
        for count in 1..=8 {
          let result = layout.arrange(
            &params(0.55, master_count, flip),
            &area,
            &ones(count),
            GAP,
          );
          let ctx = format!("{layout:?} count={count} flip={flip}");

          assert_eq!(result.len(), count, "{ctx}");

          for (i, rect) in result.iter().enumerate() {
            assert!(area.contains_rect(rect), "{ctx}: {rect:?}");
            assert!(rect.width() >= 1 && rect.height() >= 1, "{ctx}");

            for other in &result[i + 1..] {
              assert_eq!(rect.intersection_area(other), 0, "{ctx}");
            }
          }
        }
      }
    }
  }

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
