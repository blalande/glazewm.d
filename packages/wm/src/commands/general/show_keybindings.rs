use anyhow::Context;
use serde::Deserialize;
use wm_platform::{DispatcherExtWindows, OverlaySection};

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
  group: Option<String>,
}

/// Group of keybindings without a `group`.
const DEFAULT_GROUP: &str = "Generic Keybinding";

#[derive(Deserialize)]
struct RawBindingMode {
  name: String,
  #[serde(default)]
  display_name: Option<String>,
  #[serde(default)]
  keybindings: Vec<RawKeybinding>,
}

/// Shows the keybindings of the user config in an overlay centered on
/// the focused monitor.
pub fn show_keybindings(
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let sections = keybinding_sections(&config.value_str)?;

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

/// Builds one overlay section per group of the main keybindings,
/// followed by the sections of each binding mode.
///
/// A binding mode gets one section per group if its keybindings span
/// multiple groups, and a single section otherwise. Keybindings without
/// a `group` belong to the `DEFAULT_GROUP`.
///
/// A keybinding's `comment` is displayed if set, otherwise its commands.
/// Commands are taken from the unparsed config string, so that they are
/// displayed exactly as written.
fn keybinding_sections(
  config_str: &str,
) -> anyhow::Result<Vec<OverlaySection>> {
  let config: RawKeybindingsConfig = serde_yaml::from_str(config_str)?;

  let main_sections = group_keybindings(config.keybindings)
    .into_iter()
    .map(|(group, rows)| OverlaySection { title: group, rows });

  let mode_sections = config.binding_modes.into_iter().flat_map(|mode| {
    let mode_title =
      format!("Binding mode: {}", mode.display_name.unwrap_or(mode.name));

    let groups = group_keybindings(mode.keybindings);
    let has_multiple_groups = groups.len() > 1;

    groups.into_iter().map(move |(group, rows)| OverlaySection {
      title: if has_multiple_groups {
        format!("{mode_title} › {group}")
      } else {
        mode_title.clone()
      },
      rows,
    })
  });

  Ok(main_sections.chain(mode_sections).collect())
}

/// Groups keybindings by their `group` into overlay rows.
///
/// Returns `(group, rows)` pairs, ordered by the first occurrence of each
/// group.
fn group_keybindings(
  keybindings: Vec<RawKeybinding>,
) -> Vec<(String, Vec<(String, String)>)> {
  let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();

  for keybinding in keybindings {
    let group = keybinding
      .group
      .map(|group| group.trim().to_string())
      .filter(|group| !group.is_empty())
      .unwrap_or_else(|| DEFAULT_GROUP.to_string());

    let row = (
      keybinding.bindings.join(" / "),
      keybinding
        .comment
        .unwrap_or_else(|| keybinding.commands.join("; ")),
    );

    match groups.iter_mut().find(|(name, _)| *name == group) {
      Some((_, rows)) => rows.push(row),
      None => groups.push((group, vec![row])),
    }
  }

  groups
}

#[cfg(test)]
mod tests {
  use super::*;

  fn section(title: &str, rows: &[(&str, &str)]) -> OverlaySection {
    OverlaySection {
      title: title.to_string(),
      rows: rows
        .iter()
        .map(|(label, value)| (label.to_string(), value.to_string()))
        .collect(),
    }
  }

  #[test]
  fn groups_keybindings_into_sections() {
    let config_str = r"
general:
  focus_follows_cursor: false
keybindings:
  - commands: ['focus --direction left']
    bindings: ['alt+h', 'alt+left']
    group: Focus
  - commands: ['wm-enable-binding-mode --name resize', 'focus --next']
    bindings: ['alt+r']
    comment: Resize mode
  - commands: ['focus --direction right']
    bindings: ['alt+l']
    group: ' Focus '
  - commands: ['wm-redraw']
    bindings: ['alt+shift+w']
    group: ''
binding_modes:
  - name: resize
    display_name: Resize
    keybindings:
      - commands: ['resize --width -2%']
        bindings: ['h']
        group: Size
      - commands: ['wm-disable-binding-mode --name resize']
        bindings: ['escape']
  - name: move
    keybindings:
      - commands: ['move --direction left']
        bindings: ['h']
        group: Move
  - name: empty
";

    let sections =
      keybinding_sections(config_str).expect("Valid config string.");

    assert_eq!(
      sections,
      vec![
        section(
          "Focus",
          &[
            ("alt+h / alt+left", "focus --direction left"),
            ("alt+l", "focus --direction right"),
          ]
        ),
        section(
          DEFAULT_GROUP,
          &[("alt+r", "Resize mode"), ("alt+shift+w", "wm-redraw")]
        ),
        section(
          "Binding mode: Resize › Size",
          &[("h", "resize --width -2%")]
        ),
        section(
          &format!("Binding mode: Resize › {DEFAULT_GROUP}"),
          &[("escape", "wm-disable-binding-mode --name resize")]
        ),
        section("Binding mode: move", &[("h", "move --direction left")]),
      ]
    );
  }

  #[test]
  fn builds_sections_from_sample_config() {
    let sections = keybinding_sections(&UserConfig::mock().value_str)
      .expect("Valid sample config.");

    assert!(sections.iter().all(|section| !section.rows.is_empty()));
    assert_eq!(sections[0].title, DEFAULT_GROUP);
  }
}
