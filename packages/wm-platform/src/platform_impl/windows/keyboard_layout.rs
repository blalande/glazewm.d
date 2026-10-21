use std::ops::RangeInclusive;

use windows::Win32::UI::{
  Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyExW, ToUnicodeEx, MAPVK_VSC_TO_VK,
    VK_SHIFT,
  },
  TextServices::HKL,
  WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};

use crate::{Key, KeyCode};

/// Width and height of the keyboard picture, in key units (1 unit = width
/// of a letter key).
pub(crate) const KEYBOARD_WIDTH: f32 = 18.25;
pub(crate) const KEYBOARD_HEIGHT: f32 = 6.5;

/// A key of the keyboard picture, positioned in key units.
pub(crate) struct PhysicalKey {
  /// Key it produces, if it has a `Key` variant.
  pub key: Option<Key>,

  /// Text printed on the keycap.
  pub legend: String,

  pub x: f32,
  pub y: f32,
  pub width: f32,

  /// Whether this is the lower part of a key spanning two rows (i.e. ISO
  /// enter), which is drawn without text.
  pub is_continuation: bool,
}

/// Identity of a key on the keyboard picture.
#[derive(Clone, Copy)]
enum Source {
  /// Layout-dependent key, identified by its scan code.
  Scan(u32),

  /// Layout-independent key with a fixed legend.
  Fixed(Key, &'static str),

  /// Key without a `Key` variant, which can't be bound.
  Label(&'static str),
}

/// An entry of a keyboard row.
#[derive(Clone, Copy)]
enum Slot {
  Key(Source, f32),
  Continuation(Key, f32),
  Gap(f32),
}

/// Returns the keys of an ISO keyboard (main block, function row and
/// navigation cluster), with legends of the keyboard layout of the
/// foreground window.
///
/// ponytail: always ISO, so ANSI keyboards show an extra key left of `Z`
/// and an L-shaped enter; detect the physical layout if that bothers.
pub(crate) fn physical_keys() -> Vec<PhysicalKey> {
  let layout = unsafe {
    GetKeyboardLayout(GetWindowThreadProcessId(
      GetForegroundWindow(),
      None,
    ))
  };

  let mut keys = Vec::new();

  for (y, slots) in rows() {
    let mut x = 0.;

    for slot in slots {
      let (width, physical_key) = match slot {
        Slot::Gap(width) => (width, None),
        Slot::Continuation(key, width) => {
          (width, Some((Some(key), String::new(), true)))
        }
        Slot::Key(source, width) => {
          let (key, legend) = match source {
            Source::Scan(scan_code) => scan_code_key(scan_code, layout),
            Source::Fixed(key, legend) => (Some(key), legend.to_string()),
            Source::Label(legend) => (None, legend.to_string()),
          };

          (width, Some((key, legend, false)))
        }
      };

      if let Some((key, legend, is_continuation)) = physical_key {
        keys.push(PhysicalKey {
          key,
          legend,
          x,
          y,
          width,
          is_continuation,
        });
      }

      x += width;
    }
  }

  keys
}

/// Returns the rows of an ISO keyboard as `(y, slots)` pairs, in key
/// units.
// LINT: Data table of the keyboard's rows.
#[allow(clippy::too_many_lines)]
fn rows() -> [(f32, Vec<Slot>); 6] {
  let key = |source, width| Slot::Key(source, width);
  let fixed = |key: Key, legend| Slot::Key(Source::Fixed(key, legend), 1.);
  let scans = |range: RangeInclusive<u32>| {
    range.map(|scan_code| Slot::Key(Source::Scan(scan_code), 1.))
  };

  [
    (
      0.,
      [
        fixed(Key::Escape, "Esc"),
        Slot::Gap(1.),
        fixed(Key::F1, "F1"),
        fixed(Key::F2, "F2"),
        fixed(Key::F3, "F3"),
        fixed(Key::F4, "F4"),
        Slot::Gap(0.5),
        fixed(Key::F5, "F5"),
        fixed(Key::F6, "F6"),
        fixed(Key::F7, "F7"),
        fixed(Key::F8, "F8"),
        Slot::Gap(0.5),
        fixed(Key::F9, "F9"),
        fixed(Key::F10, "F10"),
        fixed(Key::F11, "F11"),
        fixed(Key::F12, "F12"),
        Slot::Gap(0.25),
        fixed(Key::PrintScreen, "PrtSc"),
        fixed(Key::ScrollLock, "ScrLk"),
        key(Source::Label("Pause"), 1.),
      ]
      .to_vec(),
    ),
    (
      1.5,
      scans(0x29..=0x29)
        .chain(scans(0x02..=0x0D))
        .chain([
          key(Source::Fixed(Key::Backspace, "Backspace"), 2.),
          Slot::Gap(0.25),
          fixed(Key::Insert, "Ins"),
          fixed(Key::Home, "Home"),
          fixed(Key::PageUp, "PgUp"),
        ])
        .collect(),
    ),
    (
      2.5,
      [key(Source::Fixed(Key::Tab, "Tab"), 1.5)]
        .into_iter()
        .chain(scans(0x10..=0x1B))
        .chain([
          key(Source::Fixed(Key::Enter, "Enter"), 1.5),
          Slot::Gap(0.25),
          fixed(Key::Delete, "Del"),
          fixed(Key::End, "End"),
          fixed(Key::PageDown, "PgDn"),
        ])
        .collect(),
    ),
    (
      3.5,
      [key(Source::Fixed(Key::CapsLock, "Caps"), 1.75)]
        .into_iter()
        .chain(scans(0x1E..=0x28))
        .chain(scans(0x2B..=0x2B))
        .chain([Slot::Continuation(Key::Enter, 1.25)])
        .collect(),
    ),
    (
      4.5,
      [key(Source::Fixed(Key::LShift, "Shift"), 1.25)]
        .into_iter()
        .chain(scans(0x56..=0x56))
        .chain(scans(0x2C..=0x35))
        .chain([
          key(Source::Fixed(Key::RShift, "Shift"), 2.75),
          Slot::Gap(1.25),
          fixed(Key::Up, "↑"),
        ])
        .collect(),
    ),
    (
      5.5,
      [
        key(Source::Fixed(Key::LCtrl, "Ctrl"), 1.25),
        key(Source::Fixed(Key::LWin, "Win"), 1.25),
        key(Source::Fixed(Key::LAlt, "Alt"), 1.25),
        key(Source::Fixed(Key::Space, "Space"), 6.25),
        key(Source::Fixed(Key::RAlt, "Alt"), 1.25),
        key(Source::Fixed(Key::RWin, "Win"), 1.25),
        key(Source::Label("Menu"), 1.25),
        key(Source::Fixed(Key::RCtrl, "Ctrl"), 1.25),
        Slot::Gap(0.25),
        fixed(Key::Left, "←"),
        fixed(Key::Down, "↓"),
        fixed(Key::Right, "→"),
      ]
      .to_vec(),
    ),
  ]
}

/// Returns the key produced by `scan_code` on `layout`, and its legend.
///
/// The legend shows the shifted character before the unshifted one when
/// they differ by more than case (e.g. `1 &` on AZERTY).
fn scan_code_key(scan_code: u32, layout: HKL) -> (Option<Key>, String) {
  let vk_code =
    unsafe { MapVirtualKeyExW(scan_code, MAPVK_VSC_TO_VK, layout) };

  let key = u16::try_from(vk_code)
    .ok()
    .filter(|vk_code| *vk_code != 0)
    .and_then(|vk_code| Key::try_from(KeyCode(vk_code)).ok());

  let base = key_text(vk_code, scan_code, layout, false);
  let shifted = key_text(vk_code, scan_code, layout, true);

  let legend = match (shifted, base) {
    (Some(shifted), Some(base)) if shifted == base.to_uppercase() => {
      shifted
    }
    (Some(shifted), Some(base)) => format!("{shifted} {base}"),
    (shifted, base) => shifted.or(base).unwrap_or_default(),
  };

  (key, legend)
}

/// Returns the text typed by a key on `layout`, with or without shift.
fn key_text(
  vk_code: u32,
  scan_code: u32,
  layout: HKL,
  with_shift: bool,
) -> Option<String> {
  let mut key_state = [0u8; 256];

  if with_shift {
    key_state[usize::from(VK_SHIFT.0)] = 0x80;
  }

  let mut buffer = [0u16; 8];

  // Flag bit 2 leaves the keyboard state (e.g. a pending dead key)
  // unchanged.
  let length = unsafe {
    ToUnicodeEx(vk_code, scan_code, &key_state, &mut buffer, 0x4, layout)
  };

  // Dead keys return -1, with their spacing character in the buffer.
  let length = if length < 0 {
    1
  } else {
    usize::try_from(length).ok()?
  };

  String::from_utf16(buffer.get(..length)?)
    .ok()
    .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rows_fit_within_keyboard_size() {
    for (y, slots) in rows() {
      let width = slots
        .iter()
        .map(|slot| match slot {
          Slot::Key(_, width)
          | Slot::Continuation(_, width)
          | Slot::Gap(width) => width,
        })
        .sum::<f32>();

      assert!(width <= KEYBOARD_WIDTH, "Row at {y} is {width} wide.");
      assert!(y + 1. <= KEYBOARD_HEIGHT);
    }
  }
}
