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
mod tests {
    use crate::{FieldError, FieldPath, FieldResolver, FieldType};

    fn item() -> FieldPath {
        FieldPath::field("item", 1, FieldType::Message)
    }

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
            core::mem::size_of::<FieldError<crate::resource::ParseError>>(),
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
    fn a_field_error_clones_and_compares() {
        let error = FieldError::new(name(), Refused);
        assert_eq!(error.clone(), error);
        assert_ne!(error, FieldError::new(item(), Refused));
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
        assert_eq!(*error.failure(), "no such path");
    }

    #[test]
    fn a_field_error_gives_its_failure_back() {
        let error = FieldError::new(name(), "no such segment");
        assert_eq!(*error.failure(), "no such segment");

        // The same failure, about another field.
        let other = FieldError::new(item(), error.into_failure());
        assert_eq!(other.path(), &item());
        assert_eq!(*other.failure(), "no such segment");
    }
}
