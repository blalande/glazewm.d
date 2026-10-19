use std::cell::RefCell;

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
        FillRect, GetTextExtentPoint32W, SelectObject, SetBkMode,
        SetTextColor, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
        DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT, LOGFONTW,
        PAINTSTRUCT, TRANSPARENT,
      },
    },
    UI::{
      Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE,
      },
      WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW,
        ShowWindow, SW_SHOWNOACTIVATE, WM_DESTROY, WM_HOTKEY,
        WM_LBUTTONDOWN, WM_PAINT, WM_RBUTTONDOWN, WNDCLASSW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
      },
    },
  },
};

use crate::{OverlaySection, Rect};

/// ID of the escape hotkey registered while the overlay is shown.
const HOTKEY_ID: i32 = 1;

const BACKGROUND_COLOR: COLORREF = COLORREF(0x002E_1E1E);
const TEXT_COLOR: COLORREF = COLORREF(0x00F4_D6CD);
const ACCENT_COLOR: COLORREF = COLORREF(0x00FA_B489);

/// Font size, padding and gaps in logical pixels (i.e. at 100% scale).
const FONT_SIZE: f32 = 15.;
const LINE_HEIGHT: f32 = 24.;
const PADDING: f32 = 24.;
const CELL_GAP: f32 = 16.;
const COLUMN_GAP: f32 = 40.;

/// Number of columns that the content is split into, if it fits.
const COLUMN_COUNT: usize = 2;

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
  title_font: HFONT,
  text_font: HFONT,
  background: HBRUSH,
  items: Vec<DrawItem>,
}

/// A piece of text to draw at a fixed position in the overlay.
struct DrawItem {
  text: Vec<u16>,
  rect: RECT,
  kind: LineKind,
}

#[derive(Clone, Copy, PartialEq)]
enum LineKind {
  Title,
  Label,
  Value,
}

/// A line of the overlay's content, prior to positioning.
enum Line<'a> {
  Spacer,
  Title(&'a str),
  Row(&'a str, &'a str),
}

/// Shows an overlay listing `sections`, centered within `working_area`.
///
/// Replaces the overlay if one is already shown. The overlay is topmost,
/// never takes focus, and closes when escape is pressed or when it is
/// clicked.
///
/// Must be called on the event loop thread.
pub(crate) fn show_overlay(
  sections: &[OverlaySection],
  working_area: &Rect,
  scale_factor: f32,
) -> crate::Result<()> {
  close_overlay();

  let scale = |value: f32| {
    // LINT: Values are small positive pixel sizes.
    #[allow(clippy::cast_possible_truncation)]
    let scaled = (value * scale_factor).round() as i32;
    scaled
  };

  let title_font = create_font(scale(FONT_SIZE), FW_SEMIBOLD.0)?;
  let text_font = create_font(scale(FONT_SIZE), FW_NORMAL.0)?;

  let max_width = working_area.width() * MAX_AREA_PERCENT / 100;
  let max_height = working_area.height() * MAX_AREA_PERCENT / 100;

  let padding = scale(PADDING);
  let line_height = scale(LINE_HEIGHT);
  let max_lines =
    usize::try_from(((max_height - 2 * padding) / line_height).max(2))
      .unwrap_or(2);

  let columns = balanced_columns(sections, max_lines);

  let (items, width, height) = layout_items(
    &columns,
    title_font,
    text_font,
    padding,
    line_height,
    scale(CELL_GAP),
    scale(COLUMN_GAP),
    max_width,
  )?;

  // ponytail: labels wider than their column are clipped; add wrapping
  // if bindings grow that long.
  let rect = Rect::from_xy(0, 0, width.min(max_width), height)
    .translate_to_center(working_area);

  let hwnd = create_window(&rect)?;

  OVERLAY.with(|overlay| {
    *overlay.borrow_mut() = Some(OverlayState {
      hwnd,
      title_font,
      text_font,
      background: unsafe { CreateSolidBrush(BACKGROUND_COLOR) },
      items,
    });
  });

  // Escape is captured via a hotkey, since the overlay never has focus.
  if let Err(err) = unsafe {
    RegisterHotKey(hwnd, HOTKEY_ID, MOD_NOREPEAT, u32::from(VK_ESCAPE.0))
  } {
    tracing::warn!(
      "Failed to register escape hotkey for overlay. Click it to close \
       instead: {err}"
    );
  }

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
fn close_overlay() {
  let hwnd = OVERLAY
    .with(|overlay| overlay.borrow().as_ref().map(|state| state.hwnd));

  // State is released in `WM_DESTROY`, so the borrow must be dropped
  // before destroying the window.
  if let Some(hwnd) = hwnd {
    if let Err(err) = unsafe { DestroyWindow(hwnd) } {
      tracing::warn!("Failed to destroy overlay window: {err}");
    }
  }
}

/// Distributes the sections' lines across `COLUMN_COUNT` columns of
/// similar height.
///
/// More columns are used if the lines don't fit within `max_lines` per
/// column.
fn balanced_columns(
  sections: &[OverlaySection],
  max_lines: usize,
) -> Vec<Vec<Line<'_>>> {
  // Titles, rows, and a spacer between each pair of sections.
  let total_lines = sections
    .iter()
    .map(|section| section.rows.len() + 2)
    .sum::<usize>()
    .saturating_sub(1);

  // Grow the column height until the lines fit in `COLUMN_COUNT`
  // columns, since titles are never split from their first row.
  let mut lines = total_lines.div_ceil(COLUMN_COUNT).clamp(2, max_lines);

  loop {
    let columns = split_into_columns(sections, lines);

    if columns.len() <= COLUMN_COUNT || lines >= max_lines {
      return columns;
    }

    lines += 1;
  }
}

/// Distributes the sections' lines across columns of at most
/// `max_lines` lines each.
///
/// A title is never left as the last line of a column, and sections are
/// separated by a spacer unless they start a column.
fn split_into_columns(
  sections: &[OverlaySection],
  max_lines: usize,
) -> Vec<Vec<Line<'_>>> {
  let mut columns = Vec::new();
  let mut column = Vec::new();

  for section in sections {
    let lines = std::iter::once(Line::Title(&section.title)).chain(
      section
        .rows
        .iter()
        .map(|(label, value)| Line::Row(label, value)),
    );

    for line in lines {
      let is_title = matches!(line, Line::Title(_));

      // Space needed for the line, including a spacer before a title
      // and the first row after it.
      let needed = match (is_title, column.is_empty()) {
        (true, true) => 2,
        (true, false) => 3,
        (false, _) => 1,
      };

      if !column.is_empty() && column.len() + needed > max_lines {
        columns.push(std::mem::take(&mut column));
      }

      if is_title && !column.is_empty() {
        column.push(Line::Spacer);
      }

      column.push(line);
    }
  }

  columns.push(column);
  columns
}

/// Positions the lines of each column.
///
/// Returns the items to draw, and the total width and height of the
/// overlay.
#[allow(clippy::too_many_arguments)]
fn layout_items(
  columns: &[Vec<Line>],
  title_font: HFONT,
  text_font: HFONT,
  padding: i32,
  line_height: i32,
  cell_gap: i32,
  column_gap: i32,
  max_width: i32,
) -> crate::Result<(Vec<DrawItem>, i32, i32)> {
  let hdc = unsafe { CreateCompatibleDC(None) };
  let mut items = Vec::new();
  let mut x = padding;
  let mut max_column_lines = 0;

  // Columns share the available width equally at most. Values and titles
  // beyond it are truncated with an ellipsis.
  let column_count = i32::try_from(columns.len()).unwrap_or(1).max(1);
  let max_column_width =
    (max_width - 2 * padding - column_gap * (column_count - 1))
      / column_count;

  for column in columns {
    let mut label_width = 0;
    let mut value_width = 0;
    let mut title_width = 0;

    for line in column {
      match line {
        Line::Title(title) => {
          title_width =
            title_width.max(text_width(hdc, title_font, title));
        }
        Line::Row(label, value) => {
          label_width = label_width.max(text_width(hdc, text_font, label));
          value_width = value_width.max(text_width(hdc, text_font, value));
        }
        Line::Spacer => {}
      }
    }

    let value_width = value_width
      .min(max_column_width - label_width - cell_gap)
      .max(0);
    let column_width = title_width
      .min(max_column_width)
      .max(label_width + cell_gap + value_width);
    let mut y = padding;

    for line in column {
      let line_rect = |left, right| RECT {
        left,
        top: y,
        right,
        bottom: y + line_height,
      };

      match line {
        Line::Title(title) => items.push(DrawItem {
          text: title.encode_utf16().collect(),
          rect: line_rect(x, x + column_width),
          kind: LineKind::Title,
        }),
        Line::Row(label, value) => {
          items.push(DrawItem {
            text: label.encode_utf16().collect(),
            rect: line_rect(x, x + label_width),
            kind: LineKind::Label,
          });
          items.push(DrawItem {
            text: value.encode_utf16().collect(),
            rect: line_rect(x + label_width + cell_gap, x + column_width),
            kind: LineKind::Value,
          });
        }
        Line::Spacer => {}
      }

      y += line_height;
    }

    x += column_width + column_gap;
    max_column_lines = max_column_lines.max(column.len());
  }

  unsafe { DeleteDC(hdc) }.ok()?;

  let width = x - column_gap + padding;
  let height = 2 * padding
    + line_height * i32::try_from(max_column_lines).unwrap_or(i32::MAX);

  Ok((items, width, height))
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

/// Draws the overlay's content.
fn paint(hwnd: HWND, state: &OverlayState) {
  let mut paint_struct = PAINTSTRUCT::default();
  let hdc = unsafe { BeginPaint(hwnd, &raw mut paint_struct) };

  unsafe {
    FillRect(hdc, &raw const paint_struct.rcPaint, state.background);
    SetBkMode(hdc, TRANSPARENT);
  }

  for item in &state.items {
    let (font, color) = match item.kind {
      LineKind::Title => (state.title_font, ACCENT_COLOR),
      LineKind::Label => (state.text_font, ACCENT_COLOR),
      LineKind::Value => (state.text_font, TEXT_COLOR),
    };

    let mut text = item.text.clone();
    let mut rect = item.rect;

    unsafe {
      SelectObject(hdc, font);
      SetTextColor(hdc, color);
      DrawTextW(
        hdc,
        &mut text,
        &raw mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
      );
    }
  }

  unsafe {
    let _ = EndPaint(hwnd, &raw const paint_struct);
  }
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
    WM_HOTKEY if wparam.0 == HOTKEY_ID as usize => {
      let _ = DestroyWindow(hwnd);
      LRESULT(0)
    }
    WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
      let _ = DestroyWindow(hwnd);
      LRESULT(0)
    }
    WM_DESTROY => {
      let _ = UnregisterHotKey(hwnd, HOTKEY_ID);

      let state = OVERLAY.with(|overlay| {
        let mut overlay = overlay.borrow_mut();
        let is_current = overlay.as_ref().is_some_and(|s| s.hwnd == hwnd);
        is_current.then(|| overlay.take()).flatten()
      });

      if let Some(state) = state {
        let _ = DeleteObject(state.title_font);
        let _ = DeleteObject(state.text_font);
        let _ = DeleteObject(state.background);
      }

      LRESULT(0)
    }
    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
  }
}
