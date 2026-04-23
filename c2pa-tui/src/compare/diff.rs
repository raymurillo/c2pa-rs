use crate::manifest::tree::DisplayNode;

/// The full diff between two loaded manifests.
#[derive(Debug, Clone)]
pub struct ManifestDiff {
    /// Label of the left manifest.
    pub left_label: String,
    /// Label of the right manifest.
    pub right_label: String,
    /// Per-field diff entries.
    pub fields: Vec<FieldDiff>,
}

/// The diff status for a single manifest field.
#[derive(Debug, Clone)]
pub enum FieldDiff {
    /// Field is identical in both manifests.
    Equal { path: String, value: String },
    /// Field exists in both but has different values.
    Changed {
        path: String,
        left: String,
        right: String,
    },
    /// Field exists only in the left manifest.
    OnlyLeft { path: String, value: String },
    /// Field exists only in the right manifest.
    OnlyRight { path: String, value: String },
}

/// Compute the diff between two manifest node trees.
///
/// Stub: not yet implemented. Implemented in spec-04.
pub fn diff(
    _left_label: &str,
    _left: &[DisplayNode],
    _right_label: &str,
    _right: &[DisplayNode],
) -> ManifestDiff {
    todo!("spec-04: implement diff()")
}
