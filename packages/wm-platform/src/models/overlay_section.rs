/// A titled group of rows shown in an overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlaySection {
  /// Heading displayed above the rows.
  pub title: String,

  /// Rows as `(label, value)` pairs, displayed in two aligned columns.
  pub rows: Vec<(String, String)>,
}
