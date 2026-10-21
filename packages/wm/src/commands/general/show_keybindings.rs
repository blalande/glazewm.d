use anyhow::Context;
use serde::Deserialize;
use wm_platform::{
  DispatcherExtWindows, Key, OverlaySection, OverlaySectionStyle,
};

use crate::{
  traits::CommonGetters, user_config::UserConfig, wm_state::WmState,
};

/// Keybinding-related subset of the user config, with bindings and
/// commands kept as written by the user.
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawKeybindingsConfig {
  keybindings: Vec<RawKeybinding>,
  binding_modes: Vec<RawBindingMode>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RawKeybinding {
  bindings: Vec<String>,
  commands: Vec<String>,
  comment: Option<String>,
}

#[derive(Deserialize)]
struct RawBindingMode {
  name: String,
  #[serde(default)]
  display_name: Option<String>,
  #[serde(default)]
  keybindings: Vec<RawKeybinding>,
}

/// Order in which held modifiers are listed in section titles.
const MODIFIER_ORDER: &[Key] = &[
  Key::Ctrl,
  Key::LCtrl,
  Key::RCtrl,
  Key::Alt,
  Key::LAlt,
  Key::RAlt,
  Key::Shift,
  Key::LShift,
  Key::RShift,
  Key::Win,
  Key::LWin,
  Key::RWin,
  Key::Cmd,
  Key::LCmd,
  Key::RCmd,
];

/// Title of the section of keybindings without held keys.
const NO_MODIFIER_TITLE: &str = "No modifier";

/// Shows the currently active keybindings in an overlay centered on the
/// focused monitor, as a keyboard picture per combination of held keys.
///
/// Shows the keybindings of the active binding mode if one is enabled,
/// and the main keybindings otherwise.
pub fn show_keybindings(
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let active_mode =
    state.binding_modes.first().map(|mode| mode.name.as_str());
  let sections = keybinding_sections(
    &config.value_str,
    active_mode,
    config.value.general.keybindings_overlay_list_threshold,
  )?;

  let display = state
    .focused_container()
    .and_then(|container| container.monitor())
    .context("No focused monitor.")?
    .native();

  state.dispatcher.show_overlay(
    sections,
    display.working_area()?,
    display.scale_factor()?,
  )?;

  Ok(())
}

/// Builds the overlay sections of the active keybindings.
///
/// Without an active binding mode, returns the sections of the main
/// keybindings. With an active binding mode, returns only the sections of
/// that mode, since its keybindings replace the main ones.
///
/// Returns one section per combination of held keys (i.e. all keys of a
/// binding but its last), ordered by first occurrence. Sections with at
/// most `list_threshold` keybindings are listed instead of shown on a
/// keyboard.
///
/// A keybinding's `comment` is displayed if set, otherwise its commands.
/// Commands are taken from the unparsed config string, so that they are
/// displayed exactly as written.
fn keybinding_sections(
  config_str: &str,
  active_mode: Option<&str>,
  list_threshold: usize,
) -> anyhow::Result<Vec<OverlaySection>> {
  let config: RawKeybindingsConfig = serde_yaml::from_str(config_str)?;

  let mut sections = match active_mode {
    None => group_by_held_keys(config.keybindings, None)?,
    Some(active_mode) => {
      let mode = config
        .binding_modes
        .into_iter()
        .find(|mode| mode.name == active_mode)
        .with_context(|| {
          format!("No binding mode named '{active_mode}'.")
        })?;

      let mode_title = format!(
        "Binding mode: {}",
        mode.display_name.unwrap_or(mode.name)
      );

      group_by_held_keys(mode.keybindings, Some(&mode_title))?
    }
  };

  for section in &mut sections {
    if section.bindings.len() <= list_threshold {
      section.style = OverlaySectionStyle::List;
    }
  }

  Ok(sections)
}

/// Groups keybindings into one section per combination of held keys.
///
/// Section titles list the held keys, prefixed by `title_prefix` if set.
/// Labels of bindings with the same keys are joined.
fn group_by_held_keys(
  keybindings: Vec<RawKeybinding>,
  title_prefix: Option<&str>,
) -> anyhow::Result<Vec<OverlaySection>> {
  let mut sections: Vec<OverlaySection> = Vec::new();

  for keybinding in keybindings {
    let label = keybinding
      .comment
      .unwrap_or_else(|| keybinding.commands.join("; "));

    for binding in &keybinding.bindings {
      let mut keys = binding
        .split('+')
        .map(Key::from_binding_str)
        .collect::<Result<Vec<_>, _>>()?;

      let trigger_key = keys
        .pop()
        .with_context(|| format!("Empty binding '{binding}'."))?;

      keys.sort_by_key(|key| {
        MODIFIER_ORDER
          .iter()
          .position(|modifier| modifier == key)
          .unwrap_or(MODIFIER_ORDER.len())
      });

      let index = if let Some(index) = sections
        .iter()
        .position(|section| section.held_keys == keys)
      {
        index
      } else {
        sections.push(OverlaySection {
          title: section_title(&keys, title_prefix),
          held_keys: keys,
          bindings: Vec::new(),
          style: OverlaySectionStyle::Keyboard,
        });
        sections.len() - 1
      };

      let bindings = &mut sections[index].bindings;

      match bindings.iter_mut().find(|(key, _)| *key == trigger_key) {
        Some((_, existing)) => *existing = format!("{existing} / {label}"),
        None => bindings.push((trigger_key, label.clone())),
      }
    }
  }

  Ok(sections)
}

/// Title of a section, listing its held keys (e.g. `Alt + Shift`).
fn section_title(held_keys: &[Key], title_prefix: Option<&str>) -> String {
  let keys = if held_keys.is_empty() {
    NO_MODIFIER_TITLE.to_string()
  } else {
    held_keys
      .iter()
      .map(|key| capitalize(&key.to_string()))
      .collect::<Vec<_>>()
      .join(" + ")
  };

  match title_prefix {
    Some(prefix) => format!("{prefix} › {keys}"),
    None => keys,
  }
}

/// Uppercases the first character of `text`.
fn capitalize(text: &str) -> String {
  let mut chars = text.chars();

  chars
    .next()
    .map_or_default(|first| first.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn section(
    title: &str,
    held_keys: &[Key],
    bindings: &[(Key, &str)],
    style: OverlaySectionStyle,
  ) -> OverlaySection {
    OverlaySection {
      title: title.to_string(),
      held_keys: held_keys.to_vec(),
      bindings: bindings
        .iter()
        .map(|(key, label)| (*key, label.to_string()))
        .collect(),
      style,
    }
  }

  const CONFIG_STR: &str = r"
general:
  focus_follows_cursor: false
keybindings:
  - commands: ['focus --direction left']
    bindings: ['alt+h', 'alt+left']
  - commands: ['wm-enable-binding-mode --name resize', 'focus --next']
    bindings: ['alt+r']
    comment: Resize mode
  - commands: ['move --direction left']
    bindings: ['shift+alt+h']
  - commands: ['wm-redraw']
    bindings: ['alt+shift+w', 'alt+r']
binding_modes:
  - name: resize
    display_name: Resize
    keybindings:
      - commands: ['resize --width -2%']
        bindings: ['h']
      - commands: ['wm-disable-binding-mode --name resize']
        bindings: ['escape', 'ctrl+c']
  - name: empty
";

  #[test]
  fn groups_main_keybindings_by_held_keys() {
    let sections = keybinding_sections(CONFIG_STR, None, 2)
      .expect("Valid config string.");

    assert_eq!(
      sections,
      vec![
        section(
          "Alt",
          &[Key::Alt],
          &[
            (Key::H, "focus --direction left"),
            (Key::Left, "focus --direction left"),
            (Key::R, "Resize mode / wm-redraw"),
          ],
          OverlaySectionStyle::Keyboard
        ),
        section(
          "Alt + Shift",
          &[Key::Alt, Key::Shift],
          &[(Key::H, "move --direction left"), (Key::W, "wm-redraw")],
          OverlaySectionStyle::List
        ),
      ]
    );
  }

  #[test]
  fn shows_only_active_mode_keybindings() {
    let sections = keybinding_sections(CONFIG_STR, Some("resize"), 1)
      .expect("Valid config string.");

    assert_eq!(
      sections,
      vec![
        section(
          &format!("Binding mode: Resize › {NO_MODIFIER_TITLE}"),
          &[],
          &[
            (Key::H, "resize --width -2%"),
            (Key::Escape, "wm-disable-binding-mode --name resize"),
          ],
          OverlaySectionStyle::Keyboard
        ),
        section(
          "Binding mode: Resize › Ctrl",
          &[Key::Ctrl],
          &[(Key::C, "wm-disable-binding-mode --name resize")],
          OverlaySectionStyle::List
        ),
      ]
    );

    let sections = keybinding_sections(CONFIG_STR, Some("empty"), 2)
      .expect("Valid config string.");

    assert_eq!(sections, Vec::new());
  }

  #[test]
  fn fails_on_unknown_active_mode() {
    assert!(keybinding_sections(CONFIG_STR, Some("unknown"), 2).is_err());
  }

  #[test]
  fn builds_sections_from_sample_config() {
    let sections =
      keybinding_sections(&UserConfig::mock().value_str, None, 2)
        .expect("Valid sample config.");

    assert!(sections.iter().all(|section| !section.bindings.is_empty()));
    assert_eq!(sections[0].held_keys, vec![Key::Alt]);
  }
}
