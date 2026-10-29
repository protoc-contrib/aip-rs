//! A failure of the value of a request field, and which field it was.
//!
//! See [`FieldError`]; its path is a [`FieldPath`].

use core::fmt;
use std::sync::Arc;

use super::path::{FieldPath, FieldResolver};

/// The failure a [`FieldError`] is about: any error, or a text.
type Failure = Arc<dyn core::error::Error + Send + Sync + 'static>;

/// Why the value of a request field is invalid, and which field it was: what
/// a generated `parse` method on a request or a resource returns, so a caller
/// can name the field in an `InvalidArgument` -- AIP-193's
/// `google.rpc.BadRequest` -- without typing it again at the call site. As
/// aip-go's `*FieldError`, which holds the field's path and an `error`.
///
/// The failure is any error, or a text, as aip-go's is any `error`: what the
/// parse of a name says, say, a [`ResourceNameError`]. `Display` is
/// `name: <the failure>`, and [`source`](core::error::Error::source) is the
/// failure, so a chain that logs sources shows both, and a caller that wants
/// the failure as its own type asks for it with `downcast_ref`. The path is
/// the field's own, from the message that was parsed: the generated
/// `parse_name` on a request names `name`, and a caller holding that message
/// inside another puts it where it is with [`prefixed`](Self::prefixed).
///
/// protoc-gen-rust-aip's generated `parse*` methods return it, with the field's
/// number and type already filled in.
///
/// The failure and the path are boxed together, so a `Result` of it stays one
/// pointer wide in its `Err`: a bare path and a failure next to each other would
/// trip clippy's `result_large_err` on every function that returns one.
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
/// [`ResourceNameError`]: crate::resource::ResourceNameError
#[derive(Debug, Clone)]
pub struct FieldError {
    inner: Box<FieldErrorInner>,
}

/// What a [`FieldError`] holds, behind its box.
#[derive(Debug, Clone)]
struct FieldErrorInner {
    path: FieldPath,
    source: Failure,
}

impl FieldError {
    /// The failure `source` of the field at `path`.
    #[must_use]
    pub fn new(
        path: FieldPath,
        source: impl Into<Box<dyn core::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        Self {
            inner: Box::new(FieldErrorInner {
                path,
                source: Arc::from(source.into()),
            }),
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
    pub fn of<M: FieldResolver>(
        name: &str,
        source: impl Into<Box<dyn core::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        Self::new(FieldPath::of::<M>(name), source)
    }

    /// The field, as a caller reports it: `name`, `item.name`,
    /// `requests[1].item.name`.
    #[must_use]
    pub fn path(&self) -> &FieldPath {
        &self.inner.path
    }

    /// Why the value is invalid: the failure the error was made of, as aip-go's
    /// `Unwrap`.
    #[must_use]
    pub fn failure(&self) -> &(dyn core::error::Error + Send + Sync + 'static) {
        &*self.inner.source
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
impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.inner.path, self.inner.source)
    }
}

impl core::error::Error for FieldError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&*self.inner.source)
    }
}

#[cfg(test)]
mod tests {
    use crate::{FieldError, FieldPath, FieldResolver, FieldType};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Refused;

    impl core::fmt::Display for Refused {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("refused")
        }
    }

    impl core::error::Error for Refused {}

    fn name() -> FieldPath {
        FieldPath::field("name", 1, FieldType::String)
    }

    #[test]
    fn a_field_error_names_its_field_and_says_why() {
        let error = FieldError::new(name(), Refused);
        assert_eq!(error.path(), &name());
        assert_eq!(error.to_string(), "name: refused");
    }

    #[test]
    fn a_field_error_is_one_pointer_wide_whatever_it_wraps() {
        // What keeps clippy's `result_large_err` quiet on every function that
        // returns one.
        assert_eq!(
            core::mem::size_of::<FieldError>(),
            core::mem::size_of::<usize>()
        );
    }

    #[test]
    fn a_field_error_has_its_failure_as_its_source() {
        use core::error::Error as _;

        let error = FieldError::new(name(), Refused);
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("refused")
        );
    }

    #[test]
    fn a_field_error_is_prefixed_with_where_the_message_sits() {
        let error = FieldError::new(name(), Refused)
            .prefixed(&FieldPath::index("requests", 1, FieldType::Message, 2))
            .prefixed(&FieldPath::field("batch", 3, FieldType::Message));
        assert_eq!(error.path().to_string(), "batch.requests[2].name");
        assert_eq!(error.to_string(), "batch.requests[2].name: refused");
    }

    #[test]
    fn a_field_error_clones() {
        let error = FieldError::new(name(), Refused);
        let clone = error.clone();
        assert_eq!(clone.path(), error.path());
        assert_eq!(clone.to_string(), error.to_string());
    }

    #[test]
    fn a_field_error_says_which_error_it_is_about() {
        let error = FieldError::new(name(), Refused);
        assert!(error.failure().downcast_ref::<Refused>().is_some());
        assert!(error.failure().downcast_ref::<std::io::Error>().is_none());
    }

    #[test]
    fn a_field_error_is_made_of_a_field_name_in_one_call() {
        struct Request;
        impl FieldResolver for Request {
            fn field_path(name: &str) -> Option<FieldPath> {
                (name == "update_mask")
                    .then(|| FieldPath::field("update_mask", 2, FieldType::Message))
            }
        }

        let error = FieldError::of::<Request>("update_mask", "no such path");
        assert_eq!(error.path().to_string(), "update_mask");
        assert_eq!(error.failure().to_string(), "no such path");
    }
}
