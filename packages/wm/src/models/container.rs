use std::{
  cell::{Ref, RefMut},
  collections::VecDeque,
};

use ambassador::Delegate;
use enum_as_inner::EnumAsInner;
use uuid::Uuid;
use wm_common::{
  ActiveDrag, ContainerDto, DisplayState, WindowRuleConfig, WindowState,
};
use wm_platform::{NativeWindow, Rect, RectDelta};

#[allow(clippy::wildcard_imports)]
use crate::{
  models::{
    Monitor, NativeWindowProperties, NonTilingWindow, RootContainer,
    TilingWindow, Workspace,
  },
  traits::*,
  user_config::UserConfig,
};

/// A container of any type.
///
/// Uses:
///
///  * [`wm_macros::SubEnum`] to define subtypes of containers.
///  * [`wm_macros::EnumFromInner`] to define conversions between the enum
///    and wrapped types.
///  * [`ambassador::Delegate`] to delegate common getters to the contained
///    types. E.g. implements [`CommonGetters`] for [Container] by
///    forwarding the call to the item contained in the enum variant.
///
/// # Example
/// Conversion between the different container types:
/// ```
/// use wm::models::{Container, TilingWindow, WindowContainer};
/// use wm::traits::WindowGetters;
///
/// fn example(tiling: TilingWindow) {
///   // Convert a `TilingWindow` into a `Container`
///   let container: Container = tiling.into(); // Will be a `Container::TilingWindow`
///
///   // Try to convert a [Container] into a sub container type ([WindowContainer] in this case).
///   let window: WindowContainer = container.try_into().unwrap(); // Will be a `WindowContainer::TilingWindow`
///   window.state(); // Can use methods from the `WindowGetters` trait.
///
///   // Convert a sub container back into a [Container]
///   let container: Container = window.into(); // Will be a `Container::TilingWindow`
/// }
/// ```
#[derive(
  Clone,
  Debug,
  EnumAsInner,
  wm_macros::EnumFromInner,
  Delegate,
  wm_macros::SubEnum,
)]
#[delegate(CommonGetters)]
#[delegate(PositionGetters)]
#[subenum(defaults, {
  /// Subenum of [Container]
  #[derive(Clone, Debug, EnumAsInner, Delegate, wm_macros::EnumFromInner)]
  #[delegate(CommonGetters)]
  #[delegate(PositionGetters)]
})]
#[subenum(WindowContainer, {
  /// Subset of containers that implement the following traits:
  /// * `CommonGetters`
  /// * `PositionGetters`
  /// * `WindowGetters`
  #[delegate(WindowGetters)]
})]
pub enum Container {
  Root(RootContainer),
  Monitor(Monitor),
  Workspace(Workspace),
  #[subenum(WindowContainer)]
  TilingWindow(TilingWindow),
  #[subenum(WindowContainer)]
  NonTilingWindow(NonTilingWindow),
}

impl PartialEq for Container {
  fn eq(&self, other: &Self) -> bool {
    self.id() == other.id()
  }
}

impl Eq for Container {}

impl PartialEq for WindowContainer {
  fn eq(&self, other: &Self) -> bool {
    self.id() == other.id()
  }
}

impl Eq for WindowContainer {}

impl std::fmt::Display for WindowContainer {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    // Truncate title if longer than 20 chars. Need to use `chars()`
    // instead of byte slices to handle invalid byte indices.
    let title = {
      let title = self.native_properties().title;
      if title.len() > 20 {
        format!("{}...", title.chars().take(17).collect::<String>())
      } else {
        title
      }
    };

    let class = {
      #[cfg(target_os = "windows")]
      {
        self.native_properties().class_name
      }
      #[cfg(not(target_os = "windows"))]
      {
        String::new()
      }
    };

    let process = self.native_properties().process_name;

    write!(
      f,
      "Window(id={:?}, process={}, class={}, title={})",
      self.native().id(),
      process,
      class,
      title,
    )?;

    Ok(())
  }
}

/// Implements the `Debug` trait for a given container struct.
///
/// Expects that the struct has a `to_dto()` method.
#[macro_export]
macro_rules! impl_container_debug {
  ($type:ty) => {
    impl std::fmt::Debug for $type {
      fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Debug::fmt(
          &self.to_dto().map_err(|_| std::fmt::Error),
          f,
        )
      }
    }
  };
}
