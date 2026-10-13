//! `google.api.field_behavior` read off a buffa-descriptor [`FieldDescriptor`],
//! for code that works from a descriptor pool rather than from generated
//! types: a test that every writable field has a column, say, or a server layer
//! that inspects a request's schema.
//!
//! Behind the `descriptor` feature. Generated code does not use it: it knows
//! each field's behaviors when it is generated.

use buffa::ExtensionSet;
use buffa::extension::Extension;
use buffa::extension::codecs::{EnumI32, Repeated};
use buffa_descriptor::FieldDescriptor;

use crate::{FieldBehaviorKind, FieldBehaviorSet};

/// `google.api.field_behavior`: extension 1052 of `google.protobuf.FieldOptions`,
/// a repeated `google.api.FieldBehavior`.
const FIELD_BEHAVIOR: Extension<Repeated<EnumI32>> =
    Extension::new(1052, "google.protobuf.FieldOptions");

/// Reads a field's `google.api.field_behavior`.
///
/// ```ignore
/// use aip::{FieldBehaviorKind, FieldDescriptorExt};
///
/// let unwritable = FieldBehaviorKind::OutputOnly | FieldBehaviorKind::Identifier | FieldBehaviorKind::Immutable;
/// let writable: Vec<&str> = pool
///     .message(index)
///     .fields()
///     .iter()
///     .filter(|field| !field.field_behaviors().intersects(unwritable))
///     .map(|field| field.name())
///     .collect();
/// ```
pub trait FieldDescriptorExt {
    /// The behaviors the field is annotated with; empty when it has none.
    /// Values this crate does not know are ignored.
    fn field_behaviors(&self) -> FieldBehaviorSet;
}

impl FieldDescriptorExt for FieldDescriptor {
    fn field_behaviors(&self) -> FieldBehaviorSet {
        self.options()
            .map(|options| options.extension(&FIELD_BEHAVIOR))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| FieldBehaviorKind::try_from(value).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use buffa::ExtensionSet;
    use buffa_descriptor::DescriptorPool;
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, FieldOptions, FileDescriptorProto,
        FileDescriptorSet,
        field_descriptor_proto::{Label, Type},
    };

    use super::{FIELD_BEHAVIOR, FieldDescriptorExt};
    use crate::{FieldBehaviorKind, FieldBehaviorSet};

    fn field(name: &str, number: i32, behaviors: Option<Vec<i32>>) -> FieldDescriptorProto {
        let options = behaviors.map(|behaviors| {
            let mut options = FieldOptions::default();
            options.set_extension(&FIELD_BEHAVIOR, behaviors);
            options
        });
        FieldDescriptorProto {
            name: Some(name.to_owned()),
            number: Some(number),
            r#type: Some(Type::TYPE_STRING),
            label: Some(Label::LABEL_OPTIONAL),
            options: options.into(),
            ..Default::default()
        }
    }

    fn pool() -> DescriptorPool {
        DescriptorPool::new(FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("book.proto".to_owned()),
                package: Some("example".to_owned()),
                syntax: Some("proto3".to_owned()),
                message_type: vec![DescriptorProto {
                    name: Some("Book".to_owned()),
                    field: vec![
                        field("name", 1, Some(vec![8])),
                        // OUTPUT_ONLY and IMMUTABLE, and a value from a newer
                        // google.api than this crate knows.
                        field("etag", 2, Some(vec![3, 5, 99])),
                        field("title", 3, None),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        })
        .expect("a valid descriptor set")
    }

    #[test]
    fn reads_each_fields_behaviors() {
        let pool = pool();
        let index = pool
            .message_index("example.Book")
            .expect("Book is in the pool");
        let behaviors: Vec<FieldBehaviorSet> = pool
            .message(index)
            .fields()
            .iter()
            .map(FieldDescriptorExt::field_behaviors)
            .collect();
        assert_eq!(
            behaviors,
            [
                FieldBehaviorKind::Identifier.into(),
                FieldBehaviorKind::OutputOnly | FieldBehaviorKind::Immutable,
                FieldBehaviorSet::empty(),
            ]
        );
    }
}
