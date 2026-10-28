use buffa::ExtensionSet;
use buffa_descriptor::DescriptorPool;
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FieldOptions, FileDescriptorProto, FileDescriptorSet,
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
