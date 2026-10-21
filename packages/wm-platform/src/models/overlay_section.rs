use crate::Key;

/// Keybindings that share the same held keys, shown in an overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlaySection {
  /// Heading displayed above the keybindings.
  pub title: String,

  /// Keys held while pressing a trigger key (e.g. `Key::Alt`), which are
  /// highlighted on the keyboard.
  pub held_keys: Vec<Key>,

  /// Trigger keys with the label shown on them.
  pub bindings: Vec<(Key, String)>,
}
