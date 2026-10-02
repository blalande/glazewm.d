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
  /// Computes the rects of `count` tiling windows within `area`.
  ///
  /// `inner_gap` is `(horizontal, vertical)`: the gap in pixels between
  /// side-by-side and stacked windows respectively. Outer gaps are the
  /// caller's concern (`area` must already be inset).
  ///
  /// Returns exactly `count` rects in child order (index 0 is the first
  /// window, i.e. the master). Rects in a row or column tile `area`
  /// exactly, with the last one absorbing the rounding remainder. Widths
  /// and heights are clamped to at least 1 px, so rects can overflow
  /// `area` when the gaps exceed it.
  #[must_use]
  pub fn arrange(
    &self,
    params: &LayoutParams,
    area: &Rect,
    count: usize,
    inner_gap: (i32, i32),
  ) -> Vec<Rect> {
    if count == 0 {
      return Vec::new();
    }

    let (gap_x, gap_y) = inner_gap;
    let ratio = params.master_ratio;

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
      Self::Columns => split_span(area.width(), gap_x, &equal(count))
        .into_iter()
        .map(|(x, w)| {
          Rect::from_xy(area.x() + x, area.y(), w, area.height())
        })
        .collect(),
      Self::Rows => split_span(area.height(), gap_y, &equal(count))
        .into_iter()
        .map(|(y, h)| {
          Rect::from_xy(area.x(), area.y() + y, area.width(), h)
        })
        .collect(),
      Self::Monocle => return vec![area.clone(); count],
      Self::Grid => {
        let mut cols = 1;
        while cols * cols < count {
          cols += 1;
        }

        let (base, extra) = (count / cols, count % cols);

        split_span(area.width(), gap_x, &equal(cols))
          .into_iter()
          .enumerate()
          .flat_map(|(i, (x, w))| {
            let rows = base + usize::from(i < extra);
            column(area, area.x() + x, w, rows, gap_y)
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

  #[test]
  fn arrange_zero_windows_is_empty() {
    for layout in Layout::CYCLE {
      let result =
        layout.arrange(&LayoutParams::default(), &area(), 0, GAP);
      assert!(result.is_empty(), "{layout:?}");
    }
  }

  #[test]
  fn arrange_single_window_fills_area() {
    for layout in Layout::CYCLE {
      for flip in [false, true] {
        let result =
          layout.arrange(&params(0.55, 1, flip), &area(), 1, GAP);
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
        Layout::MasterStack.arrange(&params, &area(), count, GAP);
      assert_eq!(result, expected, "{name}");
    }
  }

  #[test]
  fn arrange_columns_remainder_goes_to_last() {
    let area = Rect::from_xy(0, 0, 1001, 600);
    let result =
      Layout::Columns.arrange(&LayoutParams::default(), &area, 3, (0, 0));
    assert_eq!(
      result,
      rects(&[(0, 0, 333, 600), (333, 0, 333, 600), (666, 0, 335, 600)])
    );

    let result =
      Layout::Columns.arrange(&params(0.55, 1, true), &area, 3, (0, 0));
    assert_eq!(
      result,
      rects(&[(668, 0, 333, 600), (335, 0, 333, 600), (0, 0, 335, 600)])
    );
  }

  #[test]
  fn arrange_rows_remainder_goes_to_last() {
    let area = Rect::from_xy(0, 0, 1000, 601);
    let result =
      Layout::Rows.arrange(&LayoutParams::default(), &area, 3, (0, 0));
    assert_eq!(
      result,
      rects(&[
        (0, 0, 1000, 200),
        (0, 200, 1000, 200),
        (0, 400, 1000, 201)
      ])
    );

    let result =
      Layout::Rows.arrange(&params(0.55, 1, true), &area, 3, (0, 0));
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
    let result =
      Layout::Monocle.arrange(&LayoutParams::default(), &area(), 3, GAP);
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
        count,
        GAP,
      );
      assert_eq!(result, expected, "{count} windows");
    }
  }

  #[test]
  fn arrange_dwindle() {
    let result =
      Layout::Dwindle.arrange(&params(0.5, 1, false), &area(), 3, GAP);
    assert_eq!(
      result,
      rects(&[(0, 0, 495, 600), (505, 0, 495, 295), (505, 305, 495, 295)])
    );

    let result =
      Layout::Dwindle.arrange(&params(0.5, 1, true), &area(), 3, GAP);
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
      3,
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
            count,
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
