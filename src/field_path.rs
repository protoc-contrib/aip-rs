//! A path from a message to one of the fields beneath it.
//!
//! See [`FieldPath`], and [`FieldError`] for what reports one.

use core::fmt;

/// A path from a message to a field at any depth beneath it, through singular
/// message fields and list elements: `requests[1].item.display_name`.
///
/// The Rust counterpart of Go's `protopath.Path`. Each field step carries the
/// field's name, number and [`FieldType`], so a caller can report the path as
/// protovalidate reports a violation's -- generated code fills them in from the
/// schema. It is what a [`FieldBehaviorError`] holds, so a caller can report an
/// unset `REQUIRED` field of a batch entry under the entry's index.
///
/// ```
/// use aip::{FieldPath, FieldType};
///
/// let path = FieldPath::index("requests", 1, FieldType::Message, 1)
///     .join(FieldPath::field("item", 1, FieldType::Message))
///     .join(FieldPath::field("display_name", 2, FieldType::String));
/// assert_eq!(path.to_string(), "requests[1].item.display_name");
/// ```
///
/// [`FieldBehaviorError`]: crate::FieldBehaviorError
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FieldPath {
    steps: Vec<Step>,
}

/// One step of a [`FieldPath`]: a field, or an element of the list the step
/// before it names, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// A field of the message the path has reached.
    Field {
        /// The field's name, e.g. `display_name`.
        name: &'static str,
        /// The field's number.
        number: u32,
        /// The field's protobuf type.
        ty: FieldType,
    },
    /// The element at this index, of the repeated field the path has reached.
    Index(usize),
}

/// A protobuf field type, as `google.protobuf.FieldDescriptorProto.Type` and
/// protovalidate's violation paths name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldType {
    /// `double`
    Double,
    /// `float`
    Float,
    /// `int64`
    Int64,
    /// `uint64`
    Uint64,
    /// `int32`
    Int32,
    /// `fixed64`
    Fixed64,
    /// `fixed32`
    Fixed32,
    /// `bool`
    Bool,
    /// `string`
    String,
    /// a group, the proto2 form of a delimited message
    Group,
    /// a message
    Message,
    /// `bytes`
    Bytes,
    /// `uint32`
    Uint32,
    /// an enum
    Enum,
    /// `sfixed32`
    Sfixed32,
    /// `sfixed64`
    Sfixed64,
    /// `sint32`
    Sint32,
    /// `sint64`
    Sint64,
}

impl FieldPath {
    /// The path to the field `name`, numbered `number`, of type `ty`.
    #[must_use]
    pub fn field(name: &'static str, number: u32, ty: FieldType) -> Self {
        Self {
            steps: vec![Step::Field { name, number, ty }],
        }
    }

    /// The path to element `index` of the repeated field `name`, numbered
    /// `number`, whose elements are of type `ty`: `name[index]`.
    #[must_use]
    pub fn index(name: &'static str, number: u32, ty: FieldType, index: usize) -> Self {
        Self {
            steps: vec![Step::Field { name, number, ty }, Step::Index(index)],
        }
    }

    /// This path extended by `path`, which continues from where this one ends.
    #[must_use]
    pub fn join(mut self, path: Self) -> Self {
        self.steps.extend(path.steps);
        self
    }

    /// The steps of this path, from the message it starts at.
    pub fn steps(&self) -> impl ExactSizeIterator<Item = Step> + '_ {
        self.steps.iter().copied()
    }

    /// Whether this path has no steps: the message it starts at.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Writes the path as `requests[1].item.display_name`: field names joined by
/// `.`, an index in brackets after the field it is an element of.
impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, step) in self.steps.iter().enumerate() {
            match step {
                Step::Field { name, .. } if i == 0 => f.write_str(name)?,
                Step::Field { name, .. } => write!(f, ".{name}")?,
                Step::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

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

    /// The field, as a caller reports it: `name`, `item.name`,
    /// `requests[1].item.name`.
    #[must_use]
    pub fn path(&self) -> &FieldPath {
        &self.inner.path
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
    use super::{FieldError, FieldPath, FieldType, Step};

    fn item() -> FieldPath {
        FieldPath::field("item", 1, FieldType::Message)
    }

    #[test]
    fn displays_field_names_and_indices() {
        let path = FieldPath::index("requests", 1, FieldType::Message, 1)
            .join(item())
            .join(FieldPath::field("display_name", 2, FieldType::String));
        assert_eq!(path.to_string(), "requests[1].item.display_name");
        assert_eq!(item().to_string(), "item");
        assert_eq!(FieldPath::default().to_string(), "");
    }

    #[test]
    fn a_field_step_carries_its_number_and_type() {
        assert_eq!(
            FieldPath::field("quantity", 4, FieldType::Int32)
                .steps()
                .collect::<Vec<_>>(),
            [Step::Field {
                name: "quantity",
                number: 4,
                ty: FieldType::Int32
            }]
        );
    }

    #[test]
    fn index_is_the_field_then_the_element() {
        assert_eq!(
            FieldPath::index("requests", 1, FieldType::Message, 2)
                .steps()
                .collect::<Vec<_>>(),
            [
                Step::Field {
                    name: "requests",
                    number: 1,
                    ty: FieldType::Message
                },
                Step::Index(2)
            ]
        );
    }

    #[test]
    fn joining_the_empty_path_changes_nothing() {
        assert_eq!(item().join(FieldPath::default()), item());
        assert!(FieldPath::default().is_empty());
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
    fn equal_paths_hash_alike() {
        use std::collections::HashSet;
        let city = || FieldPath::field("city", 1, FieldType::String);
        let set: HashSet<_> = [item().join(city()), item().join(city())]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 1);
    }
}
