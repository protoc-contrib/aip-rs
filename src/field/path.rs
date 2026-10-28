//! A path from a message to one of the fields beneath it.
//!
//! See [`FieldPath`], and [`FieldError`](crate::FieldError) for what reports one.

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

    /// The path to the field `name` of the message `M`: a field name, or field
    /// names joined by `.` through singular message fields, as
    /// `collection.display_name`. How a caller names a request field it wrote
    /// out -- `update_mask`, `collection` -- without copying its number and type
    /// out of the proto, and with the name checked against the message.
    ///
    /// ```
    /// use aip::{FieldPath, FieldType, FieldResolver};
    ///
    /// struct Item;
    /// impl FieldResolver for Item {
    ///     fn field_path(name: &str) -> Option<FieldPath> {
    ///         match name {
    ///             "display_name" => Some(FieldPath::field("display_name", 2, FieldType::String)),
    ///             _ => None,
    ///         }
    ///     }
    /// }
    ///
    /// assert_eq!(FieldPath::of::<Item>("display_name").to_string(), "display_name");
    /// ```
    ///
    /// # Panics
    ///
    /// If `name` is not a field of `M`, or walks through a field that holds no
    /// message: a name written in a handler is checked the first time it runs,
    /// not shipped to a client as a field that does not exist, as aip-go's
    /// `FieldPath` does.
    #[must_use]
    pub fn of<M: FieldResolver>(name: &str) -> Self {
        M::field_path(name).unwrap_or_else(|| {
            panic!(
                "`{name}` is not a field of `{}`, or walks through one that holds no message",
                core::any::type_name::<M>()
            )
        })
    }
}

/// A message that resolves the name of one of its fields to its [`FieldPath`], so
/// a field can be named by its name and the name checked: [`FieldPath::of`].
///
/// protoc-gen-rust-aip implements it for the messages it generates for, with
/// each field's number and type filled in from the schema. A field that holds a
/// message resolves the rest of a dotted name through that message's own
/// implementation and [`join`](FieldPath::join)s the two; a name that is not a
/// field, or walks through a field that holds no message, is `None`.
pub trait FieldResolver {
    /// The path to the field `name` of this message, as [`FieldPath::of`]
    /// takes it, or `None` if it is not one.
    fn field_path(name: &str) -> Option<FieldPath>;
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

#[cfg(test)]
mod tests {
    use super::{FieldPath, FieldResolver, FieldType, Step};

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

    struct Item;

    impl FieldResolver for Item {
        fn field_path(name: &str) -> Option<FieldPath> {
            match name {
                "display_name" => Some(FieldPath::field("display_name", 2, FieldType::String)),
                _ => None,
            }
        }
    }

    struct Request;

    impl FieldResolver for Request {
        fn field_path(name: &str) -> Option<FieldPath> {
            let (head, rest) = match name.split_once('.') {
                Some((head, rest)) => (head, Some(rest)),
                None => (name, None),
            };
            match head {
                "item" => {
                    let item = FieldPath::field("item", 1, FieldType::Message);
                    match rest {
                        None => Some(item),
                        Some(rest) => Item::field_path(rest).map(|beneath| item.join(beneath)),
                    }
                }
                "update_mask" if rest.is_none() => {
                    Some(FieldPath::field("update_mask", 3, FieldType::Message))
                }
                _ => None,
            }
        }
    }

    #[test]
    fn of_names_a_field_by_its_name() {
        assert_eq!(
            FieldPath::of::<Request>("update_mask"),
            FieldPath::field("update_mask", 3, FieldType::Message)
        );
    }

    #[test]
    fn of_walks_through_a_message_field() {
        let path = FieldPath::of::<Request>("item.display_name");
        assert_eq!(path.to_string(), "item.display_name");
        assert_eq!(
            path.steps().collect::<Vec<_>>(),
            [
                Step::Field {
                    name: "item",
                    number: 1,
                    ty: FieldType::Message
                },
                Step::Field {
                    name: "display_name",
                    number: 2,
                    ty: FieldType::String
                }
            ]
        );
    }

    #[test]
    #[should_panic(expected = "`nope` is not a field of")]
    fn of_panics_for_a_name_that_is_not_a_field() {
        let _ = FieldPath::of::<Request>("nope");
    }

    #[test]
    #[should_panic(expected = "`update_mask.paths` is not a field of")]
    fn of_panics_for_a_name_through_a_field_that_holds_no_message() {
        let _ = FieldPath::of::<Request>("update_mask.paths");
    }

    #[test]
    #[should_panic(expected = "`item.nope` is not a field of")]
    fn of_panics_for_a_name_that_is_not_a_field_beneath_a_message_field() {
        let _ = FieldPath::of::<Request>("item.nope");
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
