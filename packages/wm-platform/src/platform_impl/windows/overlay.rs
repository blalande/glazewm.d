use std::{cell::RefCell, ops::Range};

use windows::{
  core::{w, PCWSTR},
  Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
    Graphics::{
      Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
      },
      Gdi::{
        BeginPaint, CreateCompatibleDC, CreateFontIndirectW,
        CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint,
        FillRect, GetDC, GetStockObject, GetTextExtentPoint32W,
        InvalidateRect, ReleaseDC, RoundRect, SelectObject, SetBkMode,
        SetTextColor, DRAW_TEXT_FORMAT, DT_CALCRECT, DT_EDITCONTROL,
        DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
        DT_WORDBREAK, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT,
        LOGFONTW, NULL_PEN, PAINTSTRUCT, TRANSPARENT,
      },
    },
    UI::{
      Input::KeyboardAndMouse::{
        TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
      },
      WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
        RegisterClassW, ShowWindow, SW_SHOWNOACTIVATE, WM_DESTROY,
        WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WNDCLASSW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
      },
    },
  },
};

use super::keyboard_layout::{
  physical_keys, PhysicalKey, KEYBOARD_HEIGHT, KEYBOARD_WIDTH,
};
use crate::{
  keybinding_listener::MODIFIER_GROUPS, Key, OverlaySection,
  OverlaySectionStyle, Rect,
};

const BACKGROUND_COLOR: COLORREF = COLORREF(0x002E_1E1E);
const TEXT_COLOR: COLORREF = COLORREF(0x00F4_D6CD);
const ACCENT_COLOR: COLORREF = COLORREF(0x00FA_B489);
const DIM_TEXT_COLOR: COLORREF = COLORREF(0x0086_706C);
const IDLE_KEY_COLOR: COLORREF = COLORREF(0x0044_3231);
const BOUND_KEY_COLOR: COLORREF = COLORREF(0x005A_4745);
const HINT_COLOR: COLORREF = COLORREF(0x0070_5B58);

/// Font size, padding and gaps in logical pixels (i.e. at 100% scale).
const FONT_SIZE: f32 = 15.;
const LINE_HEIGHT: f32 = 24.;
const TITLE_HEIGHT: f32 = 32.;
const PADDING: f32 = 24.;
const SECTION_GAP: f32 = 32.;
const CELL_GAP: f32 = 16.;
const HINT_PADDING: f32 = 8.;
const HINT_MAX_WIDTH: f32 = 360.;

/// Widest action of a list in logical pixels, beyond which it is
/// truncated (and shown in full on hover).
const LIST_ACTION_MAX_WIDTH: f32 = 360.;

/// Largest key size in logical pixels, used when the keyboards would
/// otherwise grow larger.
const MAX_KEY_SIZE: f32 = 72.;

/// Sizes relative to the key size.
const KEY_MARGIN: f32 = 0.05;
const KEY_PADDING: f32 = 0.08;
const LEGEND_FONT_SIZE: f32 = 0.24;
const ACTION_FONT_SIZE: f32 = 0.18;

/// Message sent when the mouse leaves a window that requested it with
/// `TrackMouseEvent` (not exposed by the `windows` crate's enabled
/// features).
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Percentage of the working area that the overlay may cover at most.
const MAX_AREA_PERCENT: i32 = 95;

thread_local! {
  /// State of the currently shown overlay, if any.
  ///
  /// Only accessed on the event loop thread.
  static OVERLAY: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
}

/// State of a shown overlay window.
struct OverlayState {
  hwnd: HWND,
  fonts: Fonts,
  brushes: Brushes,
  metrics: Metrics,
  items: Vec<DrawItem>,
  hints: Vec<Hint>,

  /// Index in `hints` of the hovered hint, and the rect of its box.
  hovered: Option<(usize, RECT)>,
}

struct Fonts {
  title: HFONT,
  text: HFONT,
  legend: HFONT,
  action: HFONT,
}

struct Brushes {
  background: HBRUSH,
  idle_key: HBRUSH,
  bound_key: HBRUSH,
  held_key: HBRUSH,
  hint: HBRUSH,
}

/// Something to draw at a fixed position in the overlay.
struct DrawItem {
  kind: ItemKind,
  rect: RECT,
  text: Vec<u16>,
}

#[derive(Clone, Copy)]
enum ItemKind {
  Title,
  KeyCap(KeyState),
  Legend(KeyState),
  Action,
  ListKey,
  ListAction,
}

/// How a key of the keyboard picture is highlighted.
#[derive(Clone, Copy)]
enum KeyState {
  Idle,
  Bound,
  Held,
}

/// Full text shown in a box when hovering `target`.
struct Hint {
  target: RECT,
  text: Vec<u16>,
}

/// Overlay sizes in physical pixels.
struct Metrics {
  padding: i32,
  section_gap: i32,
  title_height: i32,
  line_height: i32,
  cell_gap: i32,
  hint_padding: i32,
  hint_max_width: i32,
  list_action_max_width: i32,
}

/// Keybindings listed as a column of trigger keys and a column of
/// actions, measured with the overlay's fonts.
struct List<'a> {
  title: Option<&'a str>,
  lines: Vec<ListLine<'a>>,
  title_width: i32,

  /// Width of the widest key label.
  key_width: i32,

  /// Width of the widest action.
  action_width: i32,
}

/// A trigger key and its action in a `List`.
struct ListLine<'a> {
  key: String,
  action: &'a str,
  action_width: i32,
}

impl List<'_> {
  /// Width of the list, with actions truncated to
  /// `list_action_max_width`.
  fn width(&self, metrics: &Metrics) -> i32 {
    let action_width =
      self.action_width.min(metrics.list_action_max_width);

    self
      .title_width
      .max(self.key_width + metrics.cell_gap + action_width)
  }

  /// Height of the list, including its title.
  fn height(&self, metrics: &Metrics) -> i32 {
    let title_height = if self.title.is_some() {
      metrics.title_height
    } else {
      0
    };

    title_height
      + metrics.line_height * i32::try_from(self.lines.len()).unwrap_or(0)
  }
}

/// Shows an overlay with the keybindings of `sections`, centered within
/// `working_area`.
///
/// Sections are shown as keyboard pictures in a grid, followed by those
/// shown as lists. Hovering a bound key or a truncated action shows its
/// full text.
///
/// Replaces the overlay if one is already shown. The overlay is topmost,
/// never takes focus, and closes when escape is pressed (see
/// `KeyboardHook`) or when it is clicked.
///
/// Must be called on the event loop thread.
pub(crate) fn show_overlay(
  sections: &[OverlaySection],
  working_area: &Rect,
  scale_factor: f32,
) -> crate::Result<()> {
  close_overlay();

  let scale = |value: f32| to_px(value * scale_factor);

  let metrics = Metrics {
    padding: scale(PADDING),
    section_gap: scale(SECTION_GAP),
    title_height: scale(TITLE_HEIGHT),
    line_height: scale(LINE_HEIGHT),
    cell_gap: scale(CELL_GAP),
    hint_padding: scale(HINT_PADDING),
    hint_max_width: scale(HINT_MAX_WIDTH),
    list_action_max_width: scale(LIST_ACTION_MAX_WIDTH),
  };

  let max_width = working_area.width() * MAX_AREA_PERCENT / 100;
  let max_height = working_area.height() * MAX_AREA_PERCENT / 100;
  let content_width = max_width - 2 * metrics.padding;

  let title_font = create_font(scale(FONT_SIZE), FW_SEMIBOLD.0)?;
  let text_font = create_font(scale(FONT_SIZE), FW_NORMAL.0)?;

  let physical_keys = physical_keys();
  let (keyboards, lists) =
    measure_sections(sections, &physical_keys, title_font, text_font)?;

  let list_rows = flow_rows(
    &lists
      .iter()
      .map(|list| list.width(&metrics))
      .collect::<Vec<_>>(),
    content_width,
    metrics.section_gap,
  );

  let lists_height = list_rows
    .iter()
    .map(|row| row_height(&lists[row.clone()], &metrics))
    .sum::<i32>()
    + gaps(list_rows.len()) * metrics.section_gap;

  let keyboards_max_height = if lists.is_empty() {
    max_height
  } else {
    max_height - lists_height - metrics.section_gap
  };

  let (column_count, key_size) = fit_grid(
    &keyboards
      .iter()
      .map(|(_, extras)| extras.lines.len())
      .collect::<Vec<_>>(),
    &metrics,
    max_width,
    keyboards_max_height,
    MAX_KEY_SIZE * scale_factor,
  );

  let fonts = Fonts {
    title: title_font,
    text: text_font,
    legend: create_font(
      to_px(key_size * LEGEND_FONT_SIZE).max(1),
      FW_SEMIBOLD.0,
    )?,
    action: create_font(
      to_px(key_size * ACTION_FONT_SIZE).max(1),
      FW_NORMAL.0,
    )?,
  };

  let mut layout = Layout {
    items: Vec::new(),
    hints: Vec::new(),
    metrics: &metrics,
  };

  let (keyboards_width, keyboards_height) = layout.push_keyboards(
    &keyboards,
    &physical_keys,
    column_count,
    key_size,
  );

  let lists_top = if keyboards.is_empty() {
    metrics.padding
  } else {
    metrics.padding + keyboards_height + metrics.section_gap
  };

  let lists_width = layout.push_lists(&lists, &list_rows, lists_top);

  let width = 2 * metrics.padding + keyboards_width.max(lists_width);
  let height = if lists.is_empty() {
    2 * metrics.padding + keyboards_height
  } else {
    lists_top + lists_height + metrics.padding
  };

  let Layout { items, hints, .. } = layout;

  let rect =
    Rect::from_xy(0, 0, width, height).translate_to_center(working_area);

  open_window(&rect, fonts, metrics, items, hints)
}

/// Creates and shows the overlay window at `rect`, with the given
/// content.
fn open_window(
  rect: &Rect,
  fonts: Fonts,
  metrics: Metrics,
  items: Vec<DrawItem>,
  hints: Vec<Hint>,
) -> crate::Result<()> {
  let hwnd = create_window(rect)?;

  OVERLAY.with(|overlay| {
    *overlay.borrow_mut() = Some(OverlayState {
      hwnd,
      fonts,
      brushes: Brushes {
        background: unsafe { CreateSolidBrush(BACKGROUND_COLOR) },
        idle_key: unsafe { CreateSolidBrush(IDLE_KEY_COLOR) },
        bound_key: unsafe { CreateSolidBrush(BOUND_KEY_COLOR) },
        held_key: unsafe { CreateSolidBrush(ACCENT_COLOR) },
        hint: unsafe { CreateSolidBrush(HINT_COLOR) },
      },
      metrics,
      items,
      hints,
      hovered: None,
    });
  });

  unsafe {
    // Rounded corners are a cosmetic Windows 11 feature, so failure is
    // ignored.
    let _ = DwmSetWindowAttribute(
      hwnd,
      DWMWA_WINDOW_CORNER_PREFERENCE,
      std::ptr::from_ref(&DWMWCP_ROUND.0).cast(),
      // LINT: Size of an `i32` fits in a `u32`.
      #[allow(clippy::cast_possible_truncation)]
      {
        std::mem::size_of::<i32>() as u32
      },
    );

    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
  }

  Ok(())
}

/// Closes the overlay if one is shown.
///
/// Returns whether an overlay was shown.
///
/// Must be called on the event loop thread.
pub(crate) fn close_overlay() -> bool {
  let hwnd = OVERLAY
    .with(|overlay| overlay.borrow().as_ref().map(|state| state.hwnd));

  // State is released in `WM_DESTROY`, so the borrow must be dropped
  // before destroying the window.
  let Some(hwnd) = hwnd else {
    return false;
  };

  if let Err(err) = unsafe { DestroyWindow(hwnd) } {
    tracing::warn!("Failed to destroy overlay window: {err}");
  }

  true
}

/// Whether pressing `key` is done with `physical_key`.
///
/// Generic modifiers (e.g. `Key::Alt`) match both their left and right
/// keys.
fn physical_key_matches(physical_key: &PhysicalKey, key: Key) -> bool {
  let Some(physical) = physical_key.key else {
    return false;
  };

  physical == key
    || MODIFIER_GROUPS.iter().any(|(generic, variants)| {
      *generic == key && variants.contains(&physical)
    })
}

/// A keyboard section, with the list of its bindings whose keys aren't
/// on the keyboard picture.
type KeyboardSection<'a> = (&'a OverlaySection, List<'a>);

/// Measures the sections' lists.
///
/// Returns the keyboard sections with their extra lines, and the list
/// sections, each in their original order.
fn measure_sections<'a>(
  sections: &'a [OverlaySection],
  physical_keys: &[PhysicalKey],
  title_font: HFONT,
  text_font: HFONT,
) -> crate::Result<(Vec<KeyboardSection<'a>>, Vec<List<'a>>)> {
  let hdc = unsafe { CreateCompatibleDC(None) };

  let keyboards = sections
    .iter()
    .filter(|section| section.style == OverlaySectionStyle::Keyboard)
    .map(|section| {
      let extras = section.bindings.iter().filter(|(key, _)| {
        !physical_keys
          .iter()
          .any(|physical| physical_key_matches(physical, *key))
      });

      let extras = measure_list(
        hdc,
        title_font,
        text_font,
        None,
        extras,
        physical_keys,
      );

      (section, extras)
    })
    .collect();

  let lists = sections
    .iter()
    .filter(|section| section.style == OverlaySectionStyle::List)
    .map(|section| {
      measure_list(
        hdc,
        title_font,
        text_font,
        Some(&section.title),
        &section.bindings,
        physical_keys,
      )
    })
    .collect();

  unsafe { DeleteDC(hdc) }.ok()?;

  Ok((keyboards, lists))
}

/// Measures a list of `bindings`, labelling each key with its legend on
/// the keyboard picture if it has one.
fn measure_list<'a>(
  hdc: HDC,
  title_font: HFONT,
  text_font: HFONT,
  title: Option<&'a str>,
  bindings: impl IntoIterator<Item = &'a (Key, String)>,
  physical_keys: &[PhysicalKey],
) -> List<'a> {
  let lines = bindings
    .into_iter()
    .map(|(key, action)| {
      let label = physical_keys
        .iter()
        .find(|physical| {
          !physical.legend.is_empty()
            && physical_key_matches(physical, *key)
        })
        .map_or_else(
          || key.to_string(),
          |physical| physical.legend.clone(),
        );

      ListLine {
        key: label,
        action,
        action_width: text_width(hdc, text_font, action),
      }
    })
    .collect::<Vec<_>>();

  List {
    title,
    title_width: title
      .map_or(0, |title| text_width(hdc, title_font, title)),
    key_width: lines
      .iter()
      .map(|line| text_width(hdc, text_font, &line.key))
      .max()
      .unwrap_or(0),
    action_width: lines
      .iter()
      .map(|line| line.action_width)
      .max()
      .unwrap_or(0),
    lines,
  }
}

/// Splits items of the given widths into rows of at most `max_width`,
/// separated by `gap`.
///
/// Returns the index ranges of each row. An item wider than `max_width`
/// gets a row of its own.
fn flow_rows(
  widths: &[i32],
  max_width: i32,
  gap: i32,
) -> Vec<Range<usize>> {
  let mut rows = Vec::new();
  let mut start = 0;
  let mut row_width = 0;

  for (index, width) in widths.iter().enumerate() {
    if index > start && row_width + gap + width > max_width {
      rows.push(start..index);
      start = index;
    }

    row_width = if index == start {
      *width
    } else {
      row_width + gap + width
    };
  }

  if start < widths.len() {
    rows.push(start..widths.len());
  }

  rows
}

/// Height of a row of lists.
fn row_height(lists: &[List], metrics: &Metrics) -> i32 {
  lists
    .iter()
    .map(|list| list.height(metrics))
    .max()
    .unwrap_or(0)
}

/// Picks the number of grid columns that gives the largest keyboards
/// within `max_width` and `max_height`.
///
/// `extra_counts` holds the number of extra lines below each section's
/// keyboard.
///
/// Returns the column count and the key size in physical pixels, at most
/// `max_key_size`.
fn fit_grid(
  extra_counts: &[usize],
  metrics: &Metrics,
  max_width: i32,
  max_height: i32,
  max_key_size: f32,
) -> (usize, f32) {
  let mut best = (1, 0.);

  for column_count in 1..=extra_counts.len() {
    let rows = extra_counts.chunks(column_count).collect::<Vec<_>>();

    let fixed_width =
      2 * metrics.padding + gaps(column_count) * metrics.section_gap;
    let fixed_height = 2 * metrics.padding
      + gaps(rows.len()) * metrics.section_gap
      + rows
        .iter()
        .map(|row| row_fixed_height(row, metrics))
        .sum::<i32>();

    #[allow(clippy::cast_precision_loss)]
    let key_size = f32::min(
      (max_width - fixed_width) as f32
        / (column_count as f32 * KEYBOARD_WIDTH),
      (max_height - fixed_height) as f32
        / (rows.len() as f32 * KEYBOARD_HEIGHT),
    )
    .min(max_key_size);

    if key_size > best.1 {
      best = (column_count, key_size);
    }
  }

  best
}

/// Number of gaps between `count` items.
fn gaps(count: usize) -> i32 {
  i32::try_from(count.saturating_sub(1)).unwrap_or(0)
}

/// Height of a grid row, excluding its keyboards.
fn row_fixed_height(row_extra_counts: &[usize], metrics: &Metrics) -> i32 {
  let extra_lines = row_extra_counts.iter().copied().max().unwrap_or(0);

  metrics.title_height
    + metrics.line_height * i32::try_from(extra_lines).unwrap_or(0)
}

/// Rounds a size in physical pixels.
fn to_px(value: f32) -> i32 {
  // LINT: Values are small positive pixel sizes.
  #[allow(clippy::cast_possible_truncation)]
  let px = value.round() as i32;
  px
}

/// Items and hints of the overlay, positioned relative to its top-left
/// corner.
struct Layout<'a> {
  items: Vec<DrawItem>,
  hints: Vec<Hint>,
  metrics: &'a Metrics,
}

impl Layout<'_> {
  /// Positions the keyboards with their titles and extra lines in a grid
  /// of `column_count` columns, below the top padding.
  ///
  /// Returns the width and height of the grid.
  fn push_keyboards(
    &mut self,
    keyboards: &[KeyboardSection],
    physical_keys: &[PhysicalKey],
    column_count: usize,
    key_size: f32,
  ) -> (i32, i32) {
    if keyboards.is_empty() {
      return (0, 0);
    }

    let metrics = self.metrics;
    let keyboard_width = to_px(KEYBOARD_WIDTH * key_size);
    let keyboard_height = to_px(KEYBOARD_HEIGHT * key_size);
    let mut y = metrics.padding;

    for row in keyboards.chunks(column_count) {
      let mut x = metrics.padding;

      for (section, extras) in row {
        self.push_title(&section.title, x, y, keyboard_width);

        let keyboard_top = y + metrics.title_height;

        for physical_key in physical_keys {
          self.push_key(physical_key, section, key_size, x, keyboard_top);
        }

        self.push_lines(
          extras,
          x,
          keyboard_top + keyboard_height,
          keyboard_width,
        );

        x += keyboard_width + metrics.section_gap;
      }

      let extra_counts = row
        .iter()
        .map(|(_, extras)| extras.lines.len())
        .collect::<Vec<_>>();

      y += keyboard_height
        + row_fixed_height(&extra_counts, metrics)
        + metrics.section_gap;
    }

    let column_count = column_count.min(keyboards.len());
    let width = i32::try_from(column_count).unwrap_or(1) * keyboard_width
      + gaps(column_count) * metrics.section_gap;

    (width, y - metrics.section_gap - metrics.padding)
  }

  /// Positions the lists in `rows`, starting at `top`.
  ///
  /// Returns the width of the widest row.
  fn push_lists(
    &mut self,
    lists: &[List],
    rows: &[Range<usize>],
    top: i32,
  ) -> i32 {
    let metrics = self.metrics;
    let mut y = top;
    let mut max_width = 0;

    for row in rows {
      let mut x = metrics.padding;

      for list in &lists[row.clone()] {
        let width = list.width(metrics);

        if let Some(title) = list.title {
          self.push_title(title, x, y, width);
        }

        let lines_top = y + list.height(metrics)
          - metrics.line_height
            * i32::try_from(list.lines.len()).unwrap_or(0);

        self.push_lines(list, x, lines_top, width);
        x += width + metrics.section_gap;
      }

      max_width = max_width.max(x - metrics.section_gap - metrics.padding);
      y += row_height(&lists[row.clone()], metrics) + metrics.section_gap;
    }

    max_width
  }

  /// Adds a section title.
  fn push_title(&mut self, title: &str, x: i32, y: i32, width: i32) {
    self.items.push(DrawItem {
      kind: ItemKind::Title,
      rect: RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + self.metrics.title_height,
      },
      text: title.encode_utf16().collect(),
    });
  }

  /// Adds the lines of `list` (without its title), `width` wide, from
  /// (`x`, `y`).
  ///
  /// Truncated actions get a hint with their full text.
  fn push_lines(&mut self, list: &List, x: i32, y: i32, width: i32) {
    let metrics = self.metrics;
    let action_left = x + list.key_width + metrics.cell_gap;
    let mut y = y;

    for line in &list.lines {
      let line_rect = |left, right| RECT {
        left,
        top: y,
        right,
        bottom: y + metrics.line_height,
      };

      self.items.push(DrawItem {
        kind: ItemKind::ListKey,
        rect: line_rect(x, x + list.key_width),
        text: line.key.encode_utf16().collect(),
      });

      let action_rect = line_rect(action_left, x + width);
      let action_text = line.action.encode_utf16().collect::<Vec<_>>();

      if line.action_width > action_rect.right - action_rect.left {
        self.hints.push(Hint {
          target: line_rect(x, x + width),
          text: action_text.clone(),
        });
      }

      self.items.push(DrawItem {
        kind: ItemKind::ListAction,
        rect: action_rect,
        text: action_text,
      });

      y += metrics.line_height;
    }
  }

  /// Adds the keycap, legend and action of `physical_key` for `section`,
  /// with the keyboard's top-left corner at (`left`, `top`).
  ///
  /// Bound keys get a hint with the full action.
  fn push_key(
    &mut self,
    physical_key: &PhysicalKey,
    section: &OverlaySection,
    key_size: f32,
    left: i32,
    top: i32,
  ) {
    let action = section
      .bindings
      .iter()
      .find(|(key, _)| physical_key_matches(physical_key, *key))
      .map(|(_, action)| action);

    let is_held = section
      .held_keys
      .iter()
      .any(|key| physical_key_matches(physical_key, *key));

    let state = match (is_held, action) {
      (true, _) => KeyState::Held,
      (false, Some(_)) => KeyState::Bound,
      (false, None) => KeyState::Idle,
    };

    let margin = key_size * KEY_MARGIN;

    // The lower part of a two-row key overlaps the upper part's margin,
    // so that both parts look like a single key.
    let top_margin = if physical_key.is_continuation {
      -margin
    } else {
      margin
    };

    let cap = RECT {
      left: left + to_px(physical_key.x * key_size + margin),
      top: top + to_px(physical_key.y * key_size + top_margin),
      right: left
        + to_px((physical_key.x + physical_key.width) * key_size - margin),
      bottom: top + to_px((physical_key.y + 1.) * key_size - margin),
    };

    self.items.push(DrawItem {
      kind: ItemKind::KeyCap(state),
      rect: cap,
      text: Vec::new(),
    });

    if physical_key.is_continuation {
      return;
    }

    let padding = to_px(key_size * KEY_PADDING);
    let legend_height = to_px(key_size * LEGEND_FONT_SIZE * 1.3);

    self.items.push(DrawItem {
      kind: ItemKind::Legend(state),
      rect: RECT {
        left: cap.left + padding,
        top: cap.top + padding,
        right: cap.right - padding,
        bottom: cap.top + padding + legend_height,
      },
      text: physical_key.legend.encode_utf16().collect(),
    });

    if let Some(action) = action {
      let text = action.encode_utf16().collect::<Vec<_>>();

      self.hints.push(Hint {
        target: cap,
        text: text.clone(),
      });

      self.items.push(DrawItem {
        kind: ItemKind::Action,
        rect: RECT {
          left: cap.left + padding,
          top: cap.top + padding + legend_height,
          right: cap.right - padding,
          bottom: cap.bottom - padding,
        },
        text,
      });
    }
  }
}

/// Measures the width of `text` when drawn with `font`.
fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
  let text = text.encode_utf16().collect::<Vec<_>>();
  let mut size = SIZE::default();

  unsafe {
    SelectObject(hdc, font);
    let _ = GetTextExtentPoint32W(hdc, &text, &raw mut size);
  }

  size.cx
}

/// Creates a UI font of the given pixel height and weight.
fn create_font(height: i32, weight: u32) -> crate::Result<HFONT> {
  let mut log_font = LOGFONTW {
    // Negative height selects the font by its character height.
    lfHeight: -height,
    lfWeight: i32::try_from(weight).unwrap_or(400),
    ..Default::default()
  };

  for (dest, src) in log_font
    .lfFaceName
    .iter_mut()
    .zip("Segoe UI".encode_utf16())
  {
    *dest = src;
  }

  let font = unsafe { CreateFontIndirectW(&raw const log_font) };

  if font.is_invalid() {
    return Err(crate::Error::Platform(
      "Failed to create overlay font.".to_string(),
    ));
  }

  Ok(font)
}

/// Creates the overlay window, hidden, at the given position.
fn create_window(rect: &Rect) -> crate::Result<HWND> {
  let wnd_class = WNDCLASSW {
    lpszClassName: w!("GlazeWMOverlay"),
    lpfnWndProc: Some(window_proc),
    ..Default::default()
  };

  // Registration fails if the class already exists, which is fine.
  unsafe { RegisterClassW(&raw const wnd_class) };

  let hwnd = unsafe {
    CreateWindowExW(
      // `WS_EX_NOACTIVATE` and `WS_EX_TOOLWINDOW` also keep the WM from
      // managing the window.
      WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
      w!("GlazeWMOverlay"),
      PCWSTR::null(),
      WS_POPUP,
      rect.x(),
      rect.y(),
      rect.width(),
      rect.height(),
      None,
      None,
      wnd_class.hInstance,
      None,
    )
  };

  if hwnd.0 == 0 {
    return Err(crate::Error::Platform(
      "Creation of overlay window failed.".to_string(),
    ));
  }

  Ok(hwnd)
}

/// Draws the overlay's content, and the hovered hint on top.
fn paint(hwnd: HWND, state: &OverlayState) {
  let mut paint_struct = PAINTSTRUCT::default();
  let hdc = unsafe { BeginPaint(hwnd, &raw mut paint_struct) };
  let brushes = &state.brushes;

  unsafe {
    FillRect(hdc, &raw const paint_struct.rcPaint, brushes.background);
    SetBkMode(hdc, TRANSPARENT);
    // Keycaps and hints are drawn without an outline.
    SelectObject(hdc, GetStockObject(NULL_PEN));
  }

  let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS;
  let wrapped =
    DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX | DT_END_ELLIPSIS;

  for item in &state.items {
    let (font, color, format) = match item.kind {
      ItemKind::KeyCap(key_state) => {
        let brush = match key_state {
          KeyState::Idle => brushes.idle_key,
          KeyState::Bound => brushes.bound_key,
          KeyState::Held => brushes.held_key,
        };

        draw_rounded_rect(hdc, &item.rect, brush);
        continue;
      }
      ItemKind::Title => (state.fonts.title, ACCENT_COLOR, line),
      ItemKind::Legend(key_state) => {
        let color = match key_state {
          KeyState::Idle => DIM_TEXT_COLOR,
          KeyState::Bound => ACCENT_COLOR,
          KeyState::Held => BACKGROUND_COLOR,
        };

        (state.fonts.legend, color, line)
      }
      ItemKind::Action => (state.fonts.action, TEXT_COLOR, wrapped),
      ItemKind::ListKey => (state.fonts.text, ACCENT_COLOR, line),
      ItemKind::ListAction => (state.fonts.text, TEXT_COLOR, line),
    };

    draw_text(hdc, &item.text, item.rect, font, color, format);
  }

  if let Some((index, hint_box)) = state.hovered {
    if let Some(hint) = state.hints.get(index) {
      draw_rounded_rect(hdc, &hint_box, brushes.hint);
      draw_text(
        hdc,
        &hint.text,
        inset(hint_box, state.metrics.hint_padding),
        state.fonts.text,
        TEXT_COLOR,
        DT_WORDBREAK | DT_NOPREFIX,
      );
    }
  }

  unsafe {
    let _ = EndPaint(hwnd, &raw const paint_struct);
  }
}

/// Fills a rect with rounded corners.
fn draw_rounded_rect(hdc: HDC, rect: &RECT, brush: HBRUSH) {
  // Corners are rounded by an eighth of the height, up to 8 pixels.
  let diameter = ((rect.bottom - rect.top) / 4).min(16);

  unsafe {
    SelectObject(hdc, brush);
    // Without an outline, the filled area excludes the right and bottom
    // edges, hence the extra pixel.
    let _ = RoundRect(
      hdc,
      rect.left,
      rect.top,
      rect.right + 1,
      rect.bottom + 1,
      diameter,
      diameter,
    );
  }
}

/// Draws `text` within `rect`.
fn draw_text(
  hdc: HDC,
  text: &[u16],
  rect: RECT,
  font: HFONT,
  color: COLORREF,
  format: DRAW_TEXT_FORMAT,
) {
  let mut text = text.to_vec();
  let mut rect = rect;

  unsafe {
    SelectObject(hdc, font);
    SetTextColor(hdc, color);
    DrawTextW(hdc, &mut text, &raw mut rect, format);
  }
}

/// Shrinks `rect` by `amount` on each side.
fn inset(rect: RECT, amount: i32) -> RECT {
  RECT {
    left: rect.left + amount,
    top: rect.top + amount,
    right: rect.right - amount,
    bottom: rect.bottom - amount,
  }
}

/// Whether the point (`x`, `y`) is within `rect`.
fn contains(rect: &RECT, x: i32, y: i32) -> bool {
  x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// Box of a hint below its target, or above it if there isn't room
/// below, kept within `client`.
fn hint_box(
  hwnd: HWND,
  state: &OverlayState,
  hint: &Hint,
  client: &RECT,
) -> RECT {
  let metrics = &state.metrics;
  let mut text_rect = RECT {
    right: metrics.hint_max_width,
    ..Default::default()
  };

  unsafe {
    let hdc = GetDC(hwnd);
    SelectObject(hdc, state.fonts.text);
    DrawTextW(
      hdc,
      &mut hint.text.clone(),
      &raw mut text_rect,
      DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
    );
    ReleaseDC(hwnd, hdc);
  }

  let width = text_rect.right + 2 * metrics.hint_padding;
  let height = text_rect.bottom + 2 * metrics.hint_padding;
  let gap = metrics.hint_padding / 2;

  let top = if hint.target.bottom + gap + height <= client.bottom {
    hint.target.bottom + gap
  } else {
    hint.target.top - gap - height
  };

  let left = hint
    .target
    .left
    .min(client.right - gap - width)
    .max(client.left + gap);

  RECT {
    left,
    top,
    right: left + width,
    bottom: top + height,
  }
}

/// Updates the hovered hint on mouse moves, and repaints the boxes of
/// the previous and new hint if it changed.
///
/// `point` is in client coordinates, or `None` when the mouse left the
/// window.
fn update_hover(hwnd: HWND, point: Option<(i32, i32)>) {
  OVERLAY.with(|overlay| {
    let mut overlay = overlay.borrow_mut();
    let Some(state) = overlay.as_mut().filter(|state| state.hwnd == hwnd)
    else {
      return;
    };

    let index = point.and_then(|(x, y)| {
      state
        .hints
        .iter()
        .position(|hint| contains(&hint.target, x, y))
    });

    if index == state.hovered.map(|(index, _)| index) {
      return;
    }

    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &raw mut client) };

    let hovered = index.map(|index| {
      (index, hint_box(hwnd, state, &state.hints[index], &client))
    });

    for (_, hint_box) in [state.hovered, hovered].into_iter().flatten() {
      unsafe { InvalidateRect(hwnd, Some(&raw const hint_box), false) };
    }

    state.hovered = hovered;
  });
}

/// Window procedure of the overlay window.
unsafe extern "system" fn window_proc(
  hwnd: HWND,
  msg: u32,
  wparam: WPARAM,
  lparam: LPARAM,
) -> LRESULT {
  match msg {
    WM_PAINT => {
      OVERLAY.with(|overlay| {
        if let Some(state) =
          overlay.borrow().as_ref().filter(|state| state.hwnd == hwnd)
        {
          paint(hwnd, state);
        }
      });
      LRESULT(0)
    }
    WM_MOUSEMOVE => {
      // Request `WM_MOUSELEAVE`, which is sent once per request.
      let mut track = TRACKMOUSEEVENT {
        // LINT: Size of the struct fits in a `u32`.
        #[allow(clippy::cast_possible_truncation)]
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
      };
      let _ = TrackMouseEvent(&raw mut track);

      // Coordinates are signed 16-bit values in the low and high words.
      #[allow(clippy::cast_possible_truncation)]
      let point = (
        i32::from(lparam.0 as i16),
        i32::from((lparam.0 >> 16) as i16),
      );

      update_hover(hwnd, Some(point));
      LRESULT(0)
    }
    WM_MOUSELEAVE => {
      update_hover(hwnd, None);
      LRESULT(0)
    }
    WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
      let _ = DestroyWindow(hwnd);
      LRESULT(0)
    }
    WM_DESTROY => {
      let state = OVERLAY.with(|overlay| {
        let mut overlay = overlay.borrow_mut();
        let is_current = overlay.as_ref().is_some_and(|s| s.hwnd == hwnd);
        is_current.then(|| overlay.take()).flatten()
      });

      if let Some(state) = state {
        let Fonts {
          title,
          text,
          legend,
          action,
        } = state.fonts;
        let Brushes {
          background,
          idle_key,
          bound_key,
          held_key,
          hint,
        } = state.brushes;

        for font in [title, text, legend, action] {
          let _ = DeleteObject(font);
        }

        for brush in [background, idle_key, bound_key, held_key, hint] {
          let _ = DeleteObject(brush);
        }
      }

      LRESULT(0)
    }
    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const METRICS: Metrics = Metrics {
    padding: 0,
    section_gap: 0,
    title_height: 0,
    line_height: 10,
    cell_gap: 0,
    hint_padding: 0,
    hint_max_width: 0,
    list_action_max_width: 0,
  };

  #[test]
  fn fits_grid_to_largest_keys() {
    let width = to_px(KEYBOARD_WIDTH * 100.);
    let height = to_px(KEYBOARD_HEIGHT * 100.);

    // A single keyboard fills the area, up to the maximum key size.
    assert_eq!(fit_grid(&[0], &METRICS, width, height, 200.), (1, 100.));
    assert_eq!(fit_grid(&[0], &METRICS, width, height, 50.), (1, 50.));

    // Two keyboards in a wide area are placed side by side.
    assert_eq!(
      fit_grid(&[0, 0], &METRICS, 2 * width, height, 200.),
      (2, 100.)
    );

    // Extra lines below a keyboard shrink its keys.
    let (columns, key_size) =
      fit_grid(&[0, 2], &METRICS, 2 * width, height, 200.);
    assert_eq!(columns, 2);
    assert!(key_size < 100.);
  }

  #[test]
  fn flows_items_into_rows() {
    assert_eq!(flow_rows(&[], 100, 10), Vec::<Range<usize>>::new());
    assert_eq!(flow_rows(&[40, 40, 40], 100, 10), vec![0..2, 2..3]);
    assert_eq!(flow_rows(&[40, 50], 100, 10), vec![0..2]);

    // Items wider than the row get a row of their own.
    assert_eq!(
      flow_rows(&[150, 40, 150], 100, 10),
      vec![0..1, 1..2, 2..3]
    );
  }
}
