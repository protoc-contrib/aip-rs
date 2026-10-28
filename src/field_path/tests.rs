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
