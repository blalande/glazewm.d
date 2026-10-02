use std::{iter, path::PathBuf};

use clap::{error::KindFormatter, ArgGroup, Args, Parser, ValueEnum};
use serde::{Deserialize, Deserializer, Serialize};
use tracing::Level;
use uuid::Uuid;
use wm_platform::{Delta, Direction, LengthValue, OpacityValue};

use crate::{Layout, Ordinal};

const VERSION: &str = env!("VERSION_NUMBER");

#[derive(Clone, Debug, Parser)]
#[clap(name = "glazewm", author, version = VERSION, about, long_about = None)]
pub enum AppCommand {
  /// Starts the window manager.
  Start {
    /// Custom path to user config file.
    ///
    /// The default path is `%userprofile%/.glzr/glazewm/config.yaml`
    #[clap(short = 'c', long = "config", value_hint = clap::ValueHint::FilePath)]
    config_path: Option<PathBuf>,

    #[clap(flatten)]
    verbosity: Verbosity,
  },

  /// Retrieves and outputs a specific part of the window manager's state.
  ///
  /// Requires an already running instance of the window manager.
  #[clap(alias = "q")]
  Query {
    #[clap(subcommand)]
    command: QueryCommand,
  },

  /// Invokes a window manager command.
  ///
  /// Requires an already running instance of the window manager.
  #[clap(alias = "c")]
  Command {
    #[clap(long = "id")]
    subject_container_id: Option<Uuid>,

    #[clap(subcommand)]
    command: InvokeCommand,
  },

  /// Subscribes to one or more WM events (e.g. `window_close`), and
  /// continuously outputs the incoming events.
  ///
  /// Requires an already running instance of the window manager.
  Sub {
    /// WM event(s) to subscribe to.
    #[clap(short = 'e', long, value_enum, num_args = 1..)]
    events: Vec<SubscribableEvent>,
  },

  /// Unsubscribes from a prior event subscription.
  ///
  /// Requires an already running instance of the window manager.
  Unsub {
    /// Subscription ID to unsubscribe from.
    #[clap(long = "id")]
    subscription_id: Uuid,
  },
}

impl AppCommand {
  /// Parses `AppCommand` from command line arguments.
  ///
  /// Defaults to `AppCommand::Start` if no arguments are provided.
  #[must_use]
  pub fn parse_with_default(args: &Vec<String>) -> Self {
    if args.len() == 1 {
      AppCommand::Start {
        config_path: None,
        verbosity: Verbosity {
          verbose: false,
          quiet: false,
        },
      }
    } else {
      AppCommand::parse_from(args)
    }
  }
}

/// Verbosity flags to be used with `#[command(flatten)]`.
#[derive(Args, Clone, Debug)]
#[clap(about = None, long_about = None)]
pub struct Verbosity {
  /// Enables verbose logging.
  #[clap(short = 'v', long, action)]
  verbose: bool,

  /// Disables logging.
  #[clap(short = 'q', long, action, conflicts_with = "verbose")]
  quiet: bool,
}

impl Verbosity {
  /// Gets the log level based on the verbosity flags.
  #[must_use]
  pub fn level(&self) -> Level {
    match (self.verbose, self.quiet) {
      (true, _) => Level::DEBUG,
      (_, true) => Level::ERROR,
      _ => Level::INFO,
    }
  }
}

#[derive(Clone, Debug, Parser)]
pub enum QueryCommand {
  /// Outputs metadata about the application (e.g. version number).
  AppMetadata,
  /// Outputs the active binding modes.
  BindingModes,
  /// Outputs the focused container (either a window or an empty
  /// workspace).
  Focused,
  /// Outputs the layout and layout params of the focused workspace.
  Layout,
  /// Outputs all monitors.
  Monitors,
  /// Outputs all windows.
  Windows,
  /// Outputs all active workspaces.
  Workspaces,
  /// Outputs whether the window manager is paused.
  Paused,
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum SubscribableEvent {
  All,
  ApplicationExiting,
  BindingModesChanged,
  FocusChanged,
  FocusedContainerMoved,
  LayoutChanged,
  MonitorAdded,
  MonitorUpdated,
  MonitorRemoved,
  UserConfigChanged,
  WindowManaged,
  WindowUnmanaged,
  WorkspaceActivated,
  WorkspaceDeactivated,
  WorkspaceUpdated,
  PauseChanged,
}

#[derive(Clone, Debug, Parser, PartialEq, Serialize)]
pub enum InvokeCommand {
  AdjustBorders(InvokeAdjustBordersCommand),
  Close,
  Focus(InvokeFocusCommand),
  Ignore,
  Move(InvokeMoveCommand),
  MoveWorkspace {
    #[clap(long)]
    direction: Direction,
  },
  /// Swaps the focused workspace with another one on a different monitor,
  /// so that each takes the other's place. Focus follows the focused
  /// workspace.
  SwapWorkspace(InvokeSwapWorkspaceCommand),
  Position(InvokePositionCommand),
  Resize(InvokeResizeCommand),
  UpdateWorkspaceConfig {
    #[clap(long, allow_hyphen_values = true)]
    workspace: Option<String>,
    #[clap(flatten)]
    new_config: InvokeUpdateWorkspaceConfig,
  },
  SetFloating {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    centered: Option<bool>,

    #[clap(long, allow_hyphen_values = true)]
    x_pos: Option<i32>,

    #[clap(long, allow_hyphen_values = true)]
    y_pos: Option<i32>,

    #[clap(long, allow_hyphen_values = true)]
    width: Option<LengthValue>,

    #[clap(long, allow_hyphen_values = true)]
    height: Option<LengthValue>,
  },
  SetFullscreen {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    maximized: Option<bool>,
  },
  SetMinimized,
  SetTiling,
  SetTitleBarVisibility {
    #[clap(required = true, value_enum)]
    visibility: TitleBarVisibility,
  },
  SetTransparency(SetTransparencyCommand),
  ShellExec {
    #[clap(long, action)]
    hide_window: bool,

    #[clap(required = true, trailing_var_arg = true)]
    command: Vec<String>,
  },
  // Reuse `InvokeResizeCommand` struct.
  Size(InvokeResizeCommand),
  ToggleFloating {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    centered: Option<bool>,
  },
  ToggleFullscreen {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    maximized: Option<bool>,
  },
  ToggleMinimized,
  ToggleTiling,
  /// Sets the layout of the focused workspace.
  SetLayout {
    #[clap(required = true)]
    layout: Layout,
  },
  /// Switches the focused workspace to the next layout.
  CycleLayout,
  /// Adjusts the master ratio of the focused workspace (e.g. `0.05`).
  AdjustMasterRatio {
    #[clap(required = true, allow_hyphen_values = true)]
    delta: f32,
  },
  /// Adjusts the master count of the focused workspace (e.g. `1`).
  AdjustMasterCount {
    #[clap(required = true, allow_hyphen_values = true)]
    delta: i32,
  },
  /// Mirrors the layout of the focused workspace.
  FlipLayout,
  /// Swaps the focused tiling window with the first window of its
  /// workspace. Swaps with the second window if it is already first.
  Promote,
  WmCycleFocus {
    #[clap(long, default_value_t = false)]
    omit_floating: bool,

    #[clap(long, default_value_t = false)]
    omit_fullscreen: bool,

    #[clap(long, default_value_t = true)]
    omit_minimized: bool,

    #[clap(long, default_value_t = false)]
    omit_tiling: bool,
  },
  WmDisableBindingMode {
    #[clap(long)]
    name: String,
  },
  WmEnableBindingMode {
    #[clap(long)]
    name: String,
  },
  WmExit,
  WmRedraw,
  WmReloadConfig,
  WmTogglePause,
}

impl<'de> Deserialize<'de> for InvokeCommand {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    // Clap expects an array of string slices where the first argument is
    // the binary name/path. When deserializing commands from the user
    // config, we therefore have to prepend an additional empty argument.
    let unparsed = String::deserialize(deserializer)?;
    let unparsed_split = iter::once("").chain(unparsed.split_whitespace());

    InvokeCommand::try_parse_from(unparsed_split).map_err(|err| {
      // Format the error message and remove the "error: " prefix.
      let err_msg = err.apply::<KindFormatter>().to_string();
      serde::de::Error::custom(err_msg.trim_start_matches("error: "))
    })
  }
}

#[derive(Clone, Debug, PartialEq, Serialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TitleBarVisibility {
  Shown,
  Hidden,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokeAdjustBordersCommand {
  #[clap(long, allow_hyphen_values = true)]
  pub top: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub right: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub bottom: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub left: Option<LengthValue>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(skip)]
#[command(group(ArgGroup::new("target").required(true).multiple(false)))]
#[allow(clippy::struct_excessive_bools)]
pub struct InvokeFocusCommand {
  /// Direction to focus. Floating windows focus the next floating
  /// window; tiling windows focus the monitor in that direction (use
  /// `--next` and `--previous` to move within a workspace).
  #[clap(long, group = "target")]
  pub direction: Option<Direction>,

  /// Focus the next tiling window of the workspace, wrapping at the end.
  #[clap(long, groups = ["target", "ordinal"])]
  pub next: bool,

  /// Focus the previous tiling window of the workspace, wrapping at the
  /// start.
  #[clap(long, groups = ["target", "ordinal"])]
  pub previous: bool,

  /// With `--next` or `--previous`, continue onto the next or previous
  /// monitor instead of wrapping within the workspace.
  #[clap(long, requires = "ordinal")]
  pub cross_monitor: bool,

  #[clap(long, group = "target")]
  pub container_id: Option<Uuid>,

  #[clap(long, group = "target")]
  pub workspace_in_direction: Option<Direction>,

  #[clap(long, group = "target")]
  pub workspace: Option<String>,

  #[clap(long, group = "target")]
  pub monitor: Option<usize>,

  #[clap(long, group = "target")]
  pub next_active_workspace: bool,

  #[clap(long, group = "target")]
  pub prev_active_workspace: bool,

  #[clap(long, group = "target")]
  pub next_workspace: bool,

  #[clap(long, group = "target")]
  pub prev_workspace: bool,

  #[clap(long, group = "target")]
  pub next_active_workspace_on_monitor: bool,

  #[clap(long, group = "target")]
  pub prev_active_workspace_on_monitor: bool,

  #[clap(long, group = "target")]
  pub recent_workspace: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(skip)]
#[command(group(ArgGroup::new("target").required(true).multiple(false)))]
#[allow(clippy::struct_excessive_bools)]
pub struct InvokeMoveCommand {
  /// Direction to move the window. Floating windows move in that
  /// direction; tiling windows move to the monitor in that direction (use
  /// `--next` and `--previous` to reorder within a workspace).
  #[clap(long, group = "target")]
  pub direction: Option<Direction>,

  /// Swap the tiling window with the next one, wrapping at the end.
  #[clap(long, groups = ["target", "ordinal"])]
  pub next: bool,

  /// Swap the tiling window with the previous one, wrapping at the start.
  #[clap(long, groups = ["target", "ordinal"])]
  pub previous: bool,

  /// With `--next` or `--previous`, move onto the next or previous
  /// monitor instead of wrapping within the workspace.
  #[clap(long, requires = "ordinal")]
  pub cross_monitor: bool,

  /// Move window to workspace in specified direction.
  #[clap(long, group = "target")]
  pub workspace_in_direction: Option<Direction>,

  /// Name of workspace to move the window.
  #[clap(long, group = "target")]
  pub workspace: Option<String>,

  #[clap(long, group = "target")]
  pub next_active_workspace: bool,

  #[clap(long, group = "target")]
  pub prev_active_workspace: bool,

  #[clap(long, group = "target")]
  pub next_workspace: bool,

  #[clap(long, group = "target")]
  pub prev_workspace: bool,

  #[clap(long, group = "target")]
  pub next_active_workspace_on_monitor: bool,

  #[clap(long, group = "target")]
  pub prev_active_workspace_on_monitor: bool,

  #[clap(long, group = "target")]
  pub recent_workspace: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokeResizeCommand {
  #[clap(long, allow_hyphen_values = true)]
  pub width: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub height: Option<LengthValue>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct SetTransparencyCommand {
  #[clap(long)]
  pub opacity: Option<OpacityValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub opacity_delta: Option<Delta<OpacityValue>>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokePositionCommand {
  #[clap(long, action)]
  pub centered: bool,

  #[clap(long, allow_hyphen_values = true)]
  pub x_pos: Option<i32>,

  #[clap(long, allow_hyphen_values = true)]
  pub y_pos: Option<i32>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokeUpdateWorkspaceConfig {
  #[clap(long, allow_hyphen_values = true)]
  pub name: Option<String>,

  #[clap(long, allow_hyphen_values = true)]
  pub display_name: Option<String>,

  #[clap(long)]
  pub bind_to_monitor: Option<u32>,

  #[clap(long)]
  pub keep_alive: Option<bool>,
}

impl InvokeFocusCommand {
  /// Ordinal step requested via `--next` or `--previous`.
  #[must_use]
  pub fn ordinal(&self) -> Option<Ordinal> {
    Ordinal::from_flags(self.next, self.previous)
  }
}

impl InvokeMoveCommand {
  /// Ordinal step requested via `--next` or `--previous`.
  #[must_use]
  pub fn ordinal(&self) -> Option<Ordinal> {
    Ordinal::from_flags(self.next, self.previous)
  }
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(skip)]
#[command(group(ArgGroup::new("target").required(true).multiple(false)))]
pub struct InvokeSwapWorkspaceCommand {
  /// Name of the workspace to swap with. It must be active on another
  /// monitor.
  #[clap(long, group = "target")]
  pub workspace: Option<String>,

  /// Swap with the workspace displayed on the next monitor.
  #[clap(long, group = "target")]
  pub next_monitor: bool,

  /// Swap with the workspace displayed on the previous monitor.
  #[clap(long, group = "target")]
  pub prev_monitor: bool,
}

impl InvokeSwapWorkspaceCommand {
  /// Gets the monitor to swap with, relative to the focused one, if the
  /// target is given as `--next-monitor` or `--prev-monitor`.
  #[must_use]
  pub fn monitor_ordinal(&self) -> Option<Ordinal> {
    Ordinal::from_flags(self.next_monitor, self.prev_monitor)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_swap_workspace_targets() {
    let Ok(InvokeCommand::SwapWorkspace(args)) =
      parse("swap-workspace --workspace 3")
    else {
      panic!("Expected a swap-workspace command.");
    };
    assert_eq!(args.workspace.as_deref(), Some("3"));
    assert_eq!(args.monitor_ordinal(), None);

    let Ok(InvokeCommand::SwapWorkspace(args)) =
      parse("swap-workspace --prev-monitor")
    else {
      panic!("Expected a swap-workspace command.");
    };
    assert_eq!(args.monitor_ordinal(), Some(Ordinal::Previous));

    assert!(parse("swap-workspace").is_err());
    assert!(parse("swap-workspace --next-monitor --prev-monitor").is_err());
    assert!(parse("swap-workspace --next-monitor --workspace 3").is_err());
  }

  fn parse(args: &str) -> Result<InvokeCommand, clap::Error> {
    InvokeCommand::try_parse_from(
      std::iter::once("").chain(args.split_whitespace()),
    )
  }

  #[test]
  fn parses_ordinal_focus_and_move() {
    let Ok(InvokeCommand::Focus(focus)) =
      parse("focus --next --cross-monitor")
    else {
      panic!("Expected a focus command.");
    };
    assert_eq!(focus.ordinal(), Some(Ordinal::Next));
    assert!(focus.cross_monitor);

    let Ok(InvokeCommand::Move(args)) = parse("move --previous") else {
      panic!("Expected a move command.");
    };
    assert_eq!(args.ordinal(), Some(Ordinal::Previous));
    assert!(!args.cross_monitor);
  }

  #[test]
  fn rejects_invalid_ordinal_combinations() {
    assert!(parse("focus --next --previous").is_err());
    assert!(parse("focus --next --workspace 1").is_err());
    assert!(parse("move --cross-monitor").is_err());
    assert!(parse("move --direction left --cross-monitor").is_err());
    assert!(parse("focus").is_err());
  }

  #[test]
  fn keeps_existing_targets_parsable() {
    assert!(parse("focus --direction left").is_ok());
    assert!(parse("focus --workspace 1").is_ok());
    assert!(parse("move --direction left").is_ok());
    assert!(parse("move --workspace 2").is_ok());
    assert!(parse("move --next-workspace").is_ok());
    assert!(parse("promote").is_ok());
  }

  #[test]
  fn removed_tiling_direction_commands_are_rejected() {
    assert!(parse("toggle-tiling-direction").is_err());
    assert!(parse("set-tiling-direction horizontal").is_err());
  }
}
