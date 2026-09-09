//! Field masks: the error a rejected `google.protobuf.FieldMask` reports with.
//!
//! Named for the mask rather than for AIP-134's `update_mask`, because "which
//! paths may this mask name?" is a question a `read_mask` selecting a
//! projection asks too, of a different allow-list. What is shared is the shape
//! of the answer: which paths were refused.
//!
//! Only the error is here. The allow-list is read off the schema at codegen
//! time, so the check itself is generated — see the scope note in [the crate
//! root](crate).
//!
//! See <https://google.aip.dev/134#field-masks> for the update case.

use core::fmt;

/// Why a field mask was rejected.
///
/// Carries every offending path rather than the first, so a client naming two
/// bad paths is told about both and needs one round-trip rather than two.
///
/// The paths are the strings the client sent, not their positions in the mask.
/// A caller building a structured field-path pointer — `update_mask.paths[1]` —
/// still holds the request, so it can find the position; carrying an index here
/// would mean a second type earning its place on the strength of the one case
/// where the path alone is ambiguous, which is a mask that repeats itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMaskError {
    field: String,
    paths: Vec<String>,
}

impl FieldMaskError {
    /// The error for these paths, on the mask field named `field`.
    ///
    /// `field` is the name the *request* gives the mask — `update_mask` for
    /// AIP-134 — so the message names something the client can find in the
    /// request it sent, rather than "the mask".
    ///
    /// Constructed by generated code, which is what knows the allowed set.
    #[must_use]
    pub fn new(field: impl Into<String>, paths: Vec<String>) -> Self {
        Self {
            field: field.into(),
            paths,
        }
    }

    /// The name of the mask field that was rejected, e.g. `update_mask`.
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// The offending paths, in the order the mask listed them.
    #[must_use]
    pub fn paths(&self) -> &[String] {
        &self.paths
    }
}

impl fmt::Display for FieldMaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: ", self.field)?;
        for (position, path) in self.paths.iter().enumerate() {
            if position > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{path:?}")?;
        }
        f.write_str(if self.paths.len() == 1 {
            " is not a permitted field path"
        } else {
            " are not permitted field paths"
        })
    }
}

impl core::error::Error for FieldMaskError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(field: &str, paths: &[&str]) -> FieldMaskError {
        FieldMaskError::new(field, paths.iter().map(|path| (*path).to_owned()).collect())
    }

    #[test]
    fn one_path_reads_as_singular() {
        assert_eq!(
            error("update_mask", &["create_time"]).to_string(),
            r#"invalid update_mask: "create_time" is not a permitted field path"#
        );
    }

    #[test]
    fn reports_every_offending_path() {
        let error = error("update_mask", &["create_time", "name"]);
        assert_eq!(error.paths().len(), 2);
        assert_eq!(
            error.to_string(),
            r#"invalid update_mask: "create_time", "name" are not permitted field paths"#
        );
    }

    #[test]
    fn the_message_names_whichever_mask_field_was_rejected() {
        // The type is not update-specific: a read_mask checked against some
        // other allow-list reports the same way, naming its own field.
        let error = error("read_mask", &["secret"]);
        assert_eq!(error.field(), "read_mask");
        assert!(error.to_string().starts_with("invalid read_mask: "));
    }
}
