//! A failure of the value of a request field, and which field it was.
//!
//! See [`FieldError`]; its path is a [`FieldPath`].

use core::fmt;

use crate::field_path::{FieldPath, FieldResolver};

/// Why the value of a request field is invalid, and which field it was: what
/// a generated `parse` method on a request or a resource returns, so a caller
/// can name the field in an `InvalidArgument` -- AIP-193's
/// `google.rpc.BadRequest` -- without typing it again at the call site.
///
/// `E` is the failure itself, e.g. [`ParseError`]. `Display` is
/// `name: <the failure>`, and [`source`](std::error::Error::source) is the
/// failure, so a chain that logs sources shows both. The path is the field's
/// own, from the message that was parsed: the generated `parse_name` on a
/// request names `name`, and a caller holding that message inside another puts
/// it where it is with [`prefixed`](Self::prefixed).
///
/// protoc-gen-rust-aip's generated `parse*` methods return it, with the field's
/// number and type already filled in.
///
/// The failure and the path are boxed together, so a `Result` of it stays one
/// pointer wide in its `Err`, whatever `E` is: a bare path and a
/// [`ParseError`] next to each other would trip clippy's `result_large_err`
/// on every function that returns one.
///
/// ```
/// use aip::{FieldError, FieldPath, FieldType};
///
/// let error = FieldError::new(FieldPath::field("name", 1, FieldType::String), "no such segment")
///     .prefixed(&FieldPath::field("item", 2, FieldType::Message));
///
/// assert_eq!(error.path().to_string(), "item.name");
/// assert_eq!(error.to_string(), "item.name: no such segment");
/// ```
///
/// [`ParseError`]: crate::resource::ParseError
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError<E> {
    inner: Box<FieldErrorInner<E>>,
}

/// What a [`FieldError`] holds, behind its box.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldErrorInner<E> {
    path: FieldPath,
    source: E,
}

impl<E> FieldError<E> {
    /// The failure `source` of the field at `path`.
    #[must_use]
    pub fn new(path: FieldPath, source: E) -> Self {
        Self {
            inner: Box::new(FieldErrorInner { path, source }),
        }
    }

    /// The failure `source` of the field `name` of the message `M`, as
    /// [`FieldPath::of`] takes it: how a caller names a request field it wrote
    /// out, in one call. As aip-go's `NewFieldError`.
    ///
    /// # Panics
    ///
    /// As [`FieldPath::of`], if `name` is not a field of `M`.
    #[must_use]
    pub fn of<M: FieldResolver>(name: &str, source: E) -> Self {
        Self::new(FieldPath::of::<M>(name), source)
    }

    /// The field, as a caller reports it: `name`, `item.name`,
    /// `requests[1].item.name`.
    #[must_use]
    pub fn path(&self) -> &FieldPath {
        &self.inner.path
    }

    /// Why the value is invalid: the failure the error was made of.
    #[must_use]
    pub fn failure(&self) -> &E {
        &self.inner.source
    }

    /// The failure the error was made of, without the field: how a caller
    /// gives the same failure another field to be about.
    #[must_use]
    pub fn into_failure(self) -> E {
        self.inner.source
    }

    /// The field, prefixed with `prefix`: how a caller holding the parsed
    /// message inside another reports it, e.g. under `item` or
    /// `requests[1].item`.
    #[must_use]
    pub fn prefixed(mut self, prefix: &FieldPath) -> Self {
        self.inner.path = prefix.clone().join(core::mem::take(&mut self.inner.path));
        self
    }
}

/// Writes `item.name: <the failure>`.
impl<E: fmt::Display> fmt::Display for FieldError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.inner.path, self.inner.source)
    }
}

impl<E: core::error::Error + 'static> core::error::Error for FieldError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.inner.source)
    }
}

#[cfg(test)]
mod tests;
