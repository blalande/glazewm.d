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
        GetAsyncKeyState, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
      },
      WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
        RegisterClassW, SetLayeredWindowAttributes, ShowWindow, LWA_ALPHA,
        SW_SHOWNOACTIVATE, WM_DESTROY, WM_LBUTTONDOWN, WM_MOUSEMOVE,
        WM_PAINT, WM_RBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
      },
    },
  },
};

use super::keyboard_layout::{
  physical_keys, PhysicalKey, KEYBOARD_HEIGHT, KEYBOARD_WIDTH,
};
use crate::{
  keybinding_listener::MODIFIER_GROUPS, Key, KeyCode, OpacityValue,
  OverlaySection, Rect,
};

const BACKGROUND_COLOR: COLORREF = COLORREF(0x002E_1E1E);
const TEXT_COLOR: COLORREF = COLORREF(0x00F4_D6CD);
const ACCENT_COLOR: COLORREF = COLORREF(0x00FA_B489);
const DIM_TEXT_COLOR: COLORREF = COLORREF(0x0086_706C);
const IDLE_KEY_COLOR: COLORREF = COLORREF(0x0044_3231);
const BOUND_KEY_COLOR: COLORREF = COLORREF(0x005A_4745);
const AVAILABLE_KEY_COLOR: COLORREF = COLORREF(0x0078_5A45);
const HINT_COLOR: COLORREF = COLORREF(0x0070_5B58);

/// Font size, padding and gaps in logical pixels (i.e. at 100% scale).
const FONT_SIZE: f32 = 15.;
const LINE_HEIGHT: f32 = 24.;
const TITLE_HEIGHT: f32 = 32.;
const PADDING: f32 = 24.;
const CELL_GAP: f32 = 16.;
const HINT_PADDING: f32 = 8.;
const HINT_MAX_WIDTH: f32 = 360.;

/// Key size in logical pixels whose legend and action fonts are kept for
/// larger keys.
const FONT_KEY_SIZE: f32 = 72.;

/// Sizes relative to the key size.
const KEY_MARGIN: f32 = 0.05;
const KEY_PADDING: f32 = 0.08;
const LEGEND_FONT_SIZE: f32 = 0.24;
const ACTION_FONT_SIZE: f32 = 0.18;

/// Message sent when the mouse leaves a window that requested it with
/// `TrackMouseEvent` (not exposed by the `windows` crate's enabled
/// features).
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Percentage of the working area's width or height that the overlay
/// covers, whichever is reached first.
const AREA_PERCENT: i32 = 70;

/// Modifier keys whose state selects the shown section.
const MODIFIER_KEYS: [Key; 8] = [
  Key::LShift,
  Key::RShift,
  Key::LCtrl,
  Key::RCtrl,
  Key::LAlt,
  Key::RAlt,
  Key::LWin,
  Key::RWin,
];

/// Title shown when no section matches the held modifiers.
const NO_KEYBINDINGS_TITLE: &str = "No keybindings";

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
  sections: Vec<OverlaySection>,
  physical_keys: Vec<PhysicalKey>,

  /// Size of a letter key in physical pixels.
  key_size: f32,

  /// Modifier keys held down, in the order of `MODIFIER_KEYS`.
  held: Vec<Key>,

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
  available_key: HBRUSH,
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

  /// Modifier that leads to more keybindings when also held.
  Available,

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
  title_height: i32,
  line_height: i32,
  cell_gap: i32,
  hint_padding: i32,
  hint_max_width: i32,
}

/// Keybindings listed as a column of trigger keys and a column of
/// actions, measured with the overlay's fonts.
struct List<'a> {
  lines: Vec<ListLine<'a>>,

  /// Width of the widest key label.
  key_width: i32,
}

/// A trigger key and its action in a `List`.
struct ListLine<'a> {
  key: String,
  action: &'a str,
  action_width: i32,
}

/// What the keyboard picture shows.
struct KeyboardView<'a> {
  title: &'a str,

  /// Keys highlighted as held.
  held_keys: &'a [Key],

  bindings: &'a [(Key, String)],

  /// Modifier keys highlighted as leading to more keybindings.
  available_keys: &'a [Key],
}

/// Shows an overlay with the keybindings of `sections`, centered within
/// `working_area`.
///
/// A single keyboard picture is shown: that of the section whose held
/// keys are the currently held modifiers, or the section with the fewest
/// held keys when no modifier is held. It is updated as modifiers are
/// pressed and released (see `update_held_modifiers`), and modifiers
/// that lead to more keybindings are highlighted. Hovering a bound key or
/// a truncated action shows its full text.
///
/// The keyboard fills `AREA_PERCENT` of `working_area`, and the overlay
/// is drawn with the given `opacity`.
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
  opacity: &OpacityValue,
) -> crate::Result<()> {
  close_overlay();

  let scale = |value: f32| to_px(value * scale_factor);

  let metrics = Metrics {
    padding: scale(PADDING),
    title_height: scale(TITLE_HEIGHT),
    line_height: scale(LINE_HEIGHT),
    cell_gap: scale(CELL_GAP),
    hint_padding: scale(HINT_PADDING),
    hint_max_width: scale(HINT_MAX_WIDTH),
  };

  let physical_keys = physical_keys();

  // The window is sized for the section with the most extra lines, so
  // that it doesn't change size when switching sections.
  let extra_count = sections
    .iter()
    .map(|section| {
      extra_bindings(&section.bindings, &physical_keys).count()
    })
    .max()
    .unwrap_or(0);

  let key_size = fit_key_size(
    &metrics,
    extra_count,
    working_area.width() * AREA_PERCENT / 100,
    working_area.height() * AREA_PERCENT / 100,
  );

  let font_key_size = key_size.min(FONT_KEY_SIZE * scale_factor);

  let fonts = Fonts {
    title: create_font(scale(FONT_SIZE), FW_SEMIBOLD.0)?,
    text: create_font(scale(FONT_SIZE), FW_NORMAL.0)?,
    legend: create_font(
      to_px(font_key_size * LEGEND_FONT_SIZE).max(1),
      FW_SEMIBOLD.0,
    )?,
    action: create_font(
      to_px(font_key_size * ACTION_FONT_SIZE).max(1),
      FW_NORMAL.0,
    )?,
  };

  let width = 2 * metrics.padding + to_px(KEYBOARD_WIDTH * key_size);
  let height = 2 * metrics.padding
    + metrics.title_height
    + to_px(KEYBOARD_HEIGHT * key_size)
    + metrics.line_height * i32::try_from(extra_count).unwrap_or(0);

  let rect =
    Rect::from_xy(0, 0, width, height).translate_to_center(working_area);

  let mut state = OverlayState {
    hwnd: create_window(&rect)?,
    fonts,
    brushes: Brushes {
      background: unsafe { CreateSolidBrush(BACKGROUND_COLOR) },
      idle_key: unsafe { CreateSolidBrush(IDLE_KEY_COLOR) },
      bound_key: unsafe { CreateSolidBrush(BOUND_KEY_COLOR) },
      available_key: unsafe { CreateSolidBrush(AVAILABLE_KEY_COLOR) },
      held_key: unsafe { CreateSolidBrush(ACCENT_COLOR) },
      hint: unsafe { CreateSolidBrush(HINT_COLOR) },
    },
    metrics,
    sections: sections.to_vec(),
    physical_keys,
    key_size,
    held: held_modifiers(None),
    items: Vec::new(),
    hints: Vec::new(),
    hovered: None,
  };

  let hwnd = state.hwnd;
  let layout_result = state.relayout();

  // State is stored before checking the layout, so that its resources
  // are released in `WM_DESTROY` on failure.
  OVERLAY.with(|overlay| *overlay.borrow_mut() = Some(state));

  if let Err(err) = layout_result {
    close_overlay();
    return Err(err);
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

    SetLayeredWindowAttributes(
      hwnd,
      COLORREF(0),
      opacity.to_alpha(),
      LWA_ALPHA,
    )?;
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

/// Switches the shown section after the key of `key_code` was pressed or
/// released, if it is a modifier and an overlay is shown.
///
/// Must be called on the event loop thread.
pub(crate) fn update_held_modifiers(key_code: KeyCode, is_down: bool) {
  let is_modifier = MODIFIER_KEYS
    .iter()
    .any(|key| KeyCode::try_from(*key).is_ok_and(|code| code == key_code));

  if !is_modifier {
    return;
  }

  OVERLAY.with(|overlay| {
    let mut overlay = overlay.borrow_mut();
    let Some(state) = overlay.as_mut() else {
      return;
    };

    // Key repeats of held modifiers don't change anything.
    let held = held_modifiers(Some((key_code, is_down)));
    if held == state.held {
      return;
    }

    state.held = held;

    if let Err(err) = state.relayout() {
      tracing::warn!("Failed to lay out overlay: {err}");
    }

    unsafe { InvalidateRect(state.hwnd, None, false) };
  });
}

/// Modifier keys currently held down, in the order of `MODIFIER_KEYS`.
///
/// `changed` (a key code and whether it is down) overrides the queried
/// key state, since the keyboard hook is called before it is updated.
fn held_modifiers(changed: Option<(KeyCode, bool)>) -> Vec<Key> {
  MODIFIER_KEYS
    .into_iter()
    .filter(|key| {
      let Ok(code) = KeyCode::try_from(*key) else {
        return false;
      };

      match changed {
        Some((changed_code, is_down)) if changed_code == code => is_down,
        // The most significant bit is set if the key is down.
        _ => (unsafe { GetAsyncKeyState(i32::from(code.0)) }) < 0,
      }
    })
    .collect()
}

/// Whether `binding_key` of a keybinding is matched by pressing `key`.
///
/// Generic modifiers (e.g. `Key::Alt`) match both their left and right
/// keys.
fn key_matches(binding_key: Key, key: Key) -> bool {
  binding_key == key
    || MODIFIER_GROUPS.iter().any(|(generic, variants)| {
      *generic == binding_key && variants.contains(&key)
    })
}

/// Whether pressing `key` is done with `physical_key`.
fn physical_key_matches(physical_key: &PhysicalKey, key: Key) -> bool {
  physical_key
    .key
    .is_some_and(|physical| key_matches(key, physical))
}

/// Whether all `keys` are matched by some of `held_keys`.
fn covers(held_keys: &[Key], keys: &[Key]) -> bool {
  keys.iter().all(|key| {
    held_keys
      .iter()
      .any(|held_key| key_matches(*held_key, *key))
  })
}

/// Section to show while the `held` modifiers are down.
///
/// Returns the section whose held keys are exactly matched by `held`, or
/// the first section with the fewest held keys if `held` is empty.
fn displayed_section<'a>(
  sections: &'a [OverlaySection],
  held: &[Key],
) -> Option<&'a OverlaySection> {
  if held.is_empty() {
    return sections
      .iter()
      .min_by_key(|section| section.held_keys.len());
  }

  sections.iter().find(|section| {
    covers(&section.held_keys, held)
      && section.held_keys.iter().all(|held_key| {
        held.iter().any(|key| key_matches(*held_key, *key))
      })
  })
}

/// Whether also holding `key` with the `held` modifiers leads to a
/// section, possibly requiring more modifiers.
fn is_available(
  sections: &[OverlaySection],
  held: &[Key],
  key: Key,
) -> bool {
  let keys = [held, &[key]].concat();

  sections
    .iter()
    .any(|section| covers(&section.held_keys, &keys))
}

/// Bindings whose keys aren't on the keyboard picture.
fn extra_bindings<'a>(
  bindings: &'a [(Key, String)],
  physical_keys: &'a [PhysicalKey],
) -> impl Iterator<Item = &'a (Key, String)> {
  bindings.iter().filter(|(key, _)| {
    !physical_keys
      .iter()
      .any(|physical| physical_key_matches(physical, *key))
  })
}

/// Measures a list of `bindings`, labelling each key with its legend on
/// the keyboard picture if it has one.
fn measure_list<'a>(
  text_font: HFONT,
  bindings: impl IntoIterator<Item = &'a (Key, String)>,
  physical_keys: &[PhysicalKey],
) -> crate::Result<List<'a>> {
  let hdc = unsafe { CreateCompatibleDC(None) };

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

  let key_width = lines
    .iter()
    .map(|line| text_width(hdc, text_font, &line.key))
    .max()
    .unwrap_or(0);

  unsafe { DeleteDC(hdc) }.ok()?;

  Ok(List { lines, key_width })
}

/// Largest key size that fits the keyboard and `extra_count` lines below
/// it within `max_width` and `max_height`.
///
/// Returns the key size in physical pixels.
fn fit_key_size(
  metrics: &Metrics,
  extra_count: usize,
  max_width: i32,
  max_height: i32,
) -> f32 {
  let fixed_width = 2 * metrics.padding;
  let fixed_height = 2 * metrics.padding
    + metrics.title_height
    + metrics.line_height * i32::try_from(extra_count).unwrap_or(0);

  #[allow(clippy::cast_precision_loss)]
  f32::min(
    (max_width - fixed_width) as f32 / KEYBOARD_WIDTH,
    (max_height - fixed_height) as f32 / KEYBOARD_HEIGHT,
  )
  .max(1.)
}

/// Rounds a size in physical pixels.
fn to_px(value: f32) -> i32 {
  // LINT: Values are small positive pixel sizes.
  #[allow(clippy::cast_possible_truncation)]
  let px = value.round() as i32;
  px
}

impl OverlayState {
  /// Lays out the keyboard of the section to show while the `held`
  /// modifiers are down.
  ///
  /// Without a matching section, shows an empty keyboard with the held
  /// modifiers highlighted.
  fn relayout(&mut self) -> crate::Result<()> {
    let view = match displayed_section(&self.sections, &self.held) {
      Some(section) => (
        section.title.as_str(),
        section.held_keys.as_slice(),
        section.bindings.as_slice(),
      ),
      None => (NO_KEYBINDINGS_TITLE, self.held.as_slice(), &[][..]),
    };

    let available_keys = MODIFIER_KEYS
      .into_iter()
      .filter(|key| {
        !self.held.contains(key)
          && is_available(&self.sections, &self.held, *key)
      })
      .collect::<Vec<_>>();

    let view = KeyboardView {
      title: view.0,
      held_keys: view.1,
      bindings: view.2,
      available_keys: &available_keys,
    };

    let extras = measure_list(
      self.fonts.text,
      extra_bindings(view.bindings, &self.physical_keys),
      &self.physical_keys,
    )?;

    let mut layout = Layout {
      items: Vec::new(),
      hints: Vec::new(),
      metrics: &self.metrics,
    };

    layout.push_keyboard(
      &view,
      &extras,
      &self.physical_keys,
      self.key_size,
    );

    let Layout { items, hints, .. } = layout;
    self.items = items;
    self.hints = hints;
    self.hovered = None;

    Ok(())
  }
}

/// Items and hints of the overlay, positioned relative to its top-left
/// corner.
struct Layout<'a> {
  items: Vec<DrawItem>,
  hints: Vec<Hint>,
  metrics: &'a Metrics,
}

impl Layout<'_> {
  /// Positions the keyboard of `view` with its title and `extras` below
  /// it, within the padding.
  fn push_keyboard(
    &mut self,
    view: &KeyboardView,
    extras: &List,
    physical_keys: &[PhysicalKey],
    key_size: f32,
  ) {
    let metrics = self.metrics;
    let keyboard_width = to_px(KEYBOARD_WIDTH * key_size);
    let keyboard_top = metrics.padding + metrics.title_height;

    self.push_title(view.title, metrics.padding, keyboard_width);

    for physical_key in physical_keys {
      self.push_key(
        physical_key,
        view,
        key_size,
        metrics.padding,
        keyboard_top,
      );
    }

    self.push_lines(
      extras,
      metrics.padding,
      keyboard_top + to_px(KEYBOARD_HEIGHT * key_size),
      keyboard_width,
    );
  }

  /// Adds the title, `width` wide, at `x` below the top padding.
  fn push_title(&mut self, title: &str, x: i32, width: i32) {
    let top = self.metrics.padding;

    self.items.push(DrawItem {
      kind: ItemKind::Title,
      rect: RECT {
        left: x,
        top,
        right: x + width,
        bottom: top + self.metrics.title_height,
      },
      text: title.encode_utf16().collect(),
    });
  }

  /// Adds the lines of `list`, `width` wide, from (`x`, `y`).
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

  /// Adds the keycap, legend and action of `physical_key` for `view`,
  /// with the keyboard's top-left corner at (`left`, `top`).
  ///
  /// Bound keys get a hint with the full action.
  fn push_key(
    &mut self,
    physical_key: &PhysicalKey,
    view: &KeyboardView,
    key_size: f32,
    left: i32,
    top: i32,
  ) {
    let action = view
      .bindings
      .iter()
      .find(|(key, _)| physical_key_matches(physical_key, *key))
      .map(|(_, action)| action);

    let is_held = view
      .held_keys
      .iter()
      .any(|key| physical_key_matches(physical_key, *key));

    let is_available = physical_key
      .key
      .is_some_and(|key| view.available_keys.contains(&key));

    let state = match (is_held, is_available, action) {
      (true, _, _) => KeyState::Held,
      (false, true, _) => KeyState::Available,
      (false, false, Some(_)) => KeyState::Bound,
      (false, false, None) => KeyState::Idle,
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
      // `WS_EX_LAYERED` allows the window to be translucent.
      WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
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
          KeyState::Available => brushes.available_key,
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
          KeyState::Available => TEXT_COLOR,
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
          available_key,
          held_key,
          hint,
        } = state.brushes;

        for font in [title, text, legend, action] {
          let _ = DeleteObject(font);
        }

        for brush in [
          background,
          idle_key,
          bound_key,
          available_key,
          held_key,
          hint,
        ] {
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
    title_height: 0,
    line_height: 10,
    cell_gap: 0,
    hint_padding: 0,
    hint_max_width: 0,
  };

  fn section(title: &str, held_keys: &[Key]) -> OverlaySection {
    OverlaySection {
      title: title.to_string(),
      held_keys: held_keys.to_vec(),
      bindings: Vec::new(),
    }
  }

  #[test]
  fn fits_key_size_to_area() {
    let width = to_px(KEYBOARD_WIDTH * 100.);
    let height = to_px(KEYBOARD_HEIGHT * 100.);

    // The keyboard fills the area, in its tightest dimension.
    assert!((fit_key_size(&METRICS, 0, width, height) - 100.).abs() < 0.1);
    assert!(
      (fit_key_size(&METRICS, 0, width, 2 * height) - 100.).abs() < 0.1
    );

    // Extra lines below the keyboard shrink its keys.
    assert!(fit_key_size(&METRICS, 2, width, height) < 100.);
  }

  #[test]
  fn displays_section_of_held_modifiers() {
    let sections = [
      section("Alt", &[Key::Alt]),
      section("Ctrl", &[Key::Ctrl]),
      section("Alt + Shift", &[Key::Alt, Key::LShift]),
    ];
    let title = |held: &[Key]| {
      displayed_section(&sections, held)
        .map(|section| section.title.as_str())
    };

    // Without held modifiers, the section with the fewest is shown.
    assert_eq!(title(&[]), Some("Alt"));
    assert_eq!(title(&[Key::RAlt]), Some("Alt"));
    assert_eq!(title(&[Key::LAlt, Key::RAlt]), Some("Alt"));
    assert_eq!(title(&[Key::LShift, Key::LAlt]), Some("Alt + Shift"));

    // Held modifiers must match a section exactly.
    assert_eq!(title(&[Key::RShift, Key::LAlt]), None);
    assert_eq!(title(&[Key::LShift]), None);
    assert_eq!(title(&[Key::LCtrl, Key::LAlt]), None);
  }

  #[test]
  fn displays_section_with_fewest_held_keys_by_default() {
    let sections = [
      section("Alt + Shift", &[Key::Alt, Key::Shift]),
      section("Ctrl", &[Key::Ctrl]),
      section("Win", &[Key::Win]),
    ];

    assert_eq!(
      displayed_section(&sections, &[]).map(|section| &section.title),
      Some(&"Ctrl".to_string())
    );
    assert_eq!(displayed_section(&[], &[]), None);
  }

  #[test]
  fn finds_available_modifiers() {
    let sections = [
      section("Alt", &[Key::Alt]),
      section("Alt + Ctrl + Shift", &[Key::Alt, Key::Ctrl, Key::Shift]),
    ];

    assert!(is_available(&sections, &[], Key::LAlt));
    assert!(is_available(&sections, &[], Key::RShift));
    assert!(is_available(&sections, &[Key::LAlt], Key::RCtrl));
    assert!(is_available(
      &sections,
      &[Key::LAlt, Key::LShift],
      Key::LCtrl
    ));
    assert!(!is_available(&sections, &[], Key::LWin));
    assert!(!is_available(&sections, &[Key::LAlt], Key::LWin));
  }
}
