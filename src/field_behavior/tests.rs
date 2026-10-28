use super::{
    FieldBehavior, FieldBehaviorError, FieldBehaviorKind, FieldBehaviorSet, FieldScope,
    UnknownFieldBehavior,
};
use crate::{FieldPath, FieldType};

fn string(name: &'static str, number: u32) -> FieldPath {
    FieldPath::field(name, number, FieldType::String)
}

#[test]
fn kinds_write_their_proto_names() {
    assert_eq!(FieldBehaviorKind::OutputOnly.to_string(), "OUTPUT_ONLY");
    assert_eq!(
        FieldBehaviorKind::NonEmptyDefault.as_str(),
        "NON_EMPTY_DEFAULT"
    );
    assert_eq!(FieldBehaviorKind::Identifier.as_str(), "IDENTIFIER");
}

#[test]
fn a_kind_round_trips_through_its_number() {
    for value in 1..=8 {
        let kind = FieldBehaviorKind::try_from(value).expect("a known value");
        assert_eq!(i32::from(kind), value);
    }
    assert_eq!(
        FieldBehaviorKind::try_from(3),
        Ok(FieldBehaviorKind::OutputOnly)
    );
    assert_eq!(i32::from(FieldBehaviorKind::Identifier), 8);
}

#[test]
fn unspecified_and_unknown_numbers_are_no_kind() {
    assert_eq!(FieldBehaviorKind::try_from(0), Err(UnknownFieldBehavior(0)));
    assert_eq!(FieldBehaviorKind::try_from(9), Err(UnknownFieldBehavior(9)));
    assert_eq!(
        UnknownFieldBehavior(9).to_string(),
        "unknown google.api.FieldBehavior value 9"
    );
}

#[test]
fn a_set_holds_what_it_is_built_from() {
    use super::FieldBehaviorSet;
    use FieldBehaviorKind::{Identifier, Immutable, InputOnly, OutputOnly, Required};

    let unwritable = OutputOnly | Identifier | Immutable;
    assert!(unwritable.contains(OutputOnly));
    assert!(unwritable.contains(Identifier));
    assert!(unwritable.contains(Immutable));
    assert!(!unwritable.contains(Required));
    assert_eq!(
        unwritable.iter().collect::<Vec<_>>(),
        [OutputOnly, Immutable, Identifier]
    );

    let one = FieldBehaviorSet::from(InputOnly);
    assert!(!one.intersects(unwritable));
    assert!((one | Immutable).intersects(unwritable));
    assert_eq!(one | FieldBehaviorSet::from(Required), Required | InputOnly);

    assert!(FieldBehaviorSet::empty().is_empty());
    assert_eq!(FieldBehaviorSet::default(), FieldBehaviorSet::empty());
    assert!(FieldBehaviorSet::empty().iter().next().is_none());
}

#[test]
fn a_set_collects_and_iterates_in_proto_order() {
    use super::FieldBehaviorSet;
    use FieldBehaviorKind::{Identifier, NonEmptyDefault, Optional, OutputOnly};

    let set: FieldBehaviorSet = [Identifier, Optional, OutputOnly, Optional]
        .into_iter()
        .collect();
    assert_eq!(
        set.into_iter().collect::<Vec<_>>(),
        [Optional, OutputOnly, Identifier]
    );
    assert!(!set.contains(NonEmptyDefault));
    let all: FieldBehaviorSet = FieldBehaviorKind::ALL.into_iter().collect();
    assert_eq!(all.iter().count(), 8);
}

#[test]
fn a_mask_contains_what_it_names_and_what_lies_beneath() {
    let paths = ["item".to_owned(), "origin.city".to_owned()];
    let scope = FieldScope::Mask(&paths);
    assert!(scope.contains("item"));
    assert!(scope.contains("item.title"));
    assert!(scope.contains("origin.city"));
    assert!(!scope.contains("origin"));
    assert!(!scope.contains("origin.street"));
    assert!(!scope.contains("items"));
    assert!(!scope.contains("ite"));
}

#[test]
fn an_empty_mask_contains_nothing_and_all_contains_everything() {
    assert!(!FieldScope::Mask(&[]).contains("item"));
    assert!(FieldScope::All.contains("item.title"));
}

#[test]
fn a_star_is_matched_as_written() {
    let paths = ["*".to_owned()];
    assert!(!FieldScope::Mask(&paths).contains("item"));
}

#[test]
fn check_is_ok_for_no_paths() {
    assert_eq!(
        FieldBehaviorError::check(FieldBehaviorKind::Required, Vec::new()),
        Ok(())
    );
}

#[test]
fn prefixed_prefixes_every_path() {
    let error = FieldBehaviorError::check(
        FieldBehaviorKind::Required,
        vec![
            string("display_name", 2),
            FieldPath::field("origin", 3, FieldType::Message).join(string("city", 1)),
        ],
    )
    .unwrap_err()
    .prefixed(
        &FieldPath::index("requests", 1, FieldType::Message, 1).join(FieldPath::field(
            "item",
            1,
            FieldType::Message,
        )),
    );
    assert_eq!(
        error.to_string(),
        "REQUIRED fields are unset: requests[1].item.display_name, requests[1].item.origin.city"
    );
}

#[test]
fn a_set_reads_its_kinds_in_enum_order() {
    assert_eq!(FieldBehaviorSet::empty().to_string(), "[]");
    assert_eq!(
        FieldBehaviorSet::from(FieldBehaviorKind::OutputOnly).to_string(),
        "[OUTPUT_ONLY]"
    );
    let set: FieldBehaviorSet = [
        FieldBehaviorKind::Identifier,
        FieldBehaviorKind::Immutable,
        FieldBehaviorKind::Required,
    ]
    .into_iter()
    .collect();
    assert_eq!(set.to_string(), "[REQUIRED IMMUTABLE IDENTIFIER]");
}

#[test]
fn the_message_reads_the_kind() {
    let immutable = FieldBehaviorError {
        kind: FieldBehaviorKind::Immutable,
        paths: vec![string("sku", 3)],
    };
    assert_eq!(immutable.to_string(), "IMMUTABLE fields are unset: sku");
    let other = FieldBehaviorError {
        kind: FieldBehaviorKind::Identifier,
        paths: vec![string("name", 1), string("parent", 2)],
    };
    assert_eq!(
        other.to_string(),
        "IDENTIFIER fields are unset: name, parent"
    );
}

#[test]
fn the_defaults_do_nothing_and_the_trait_is_dyn_compatible() {
    struct Plain;
    impl FieldBehavior for Plain {}

    let mut plain = Plain;
    let behavior: &mut dyn FieldBehavior = &mut plain;
    behavior.clear_fields(FieldBehaviorKind::OutputOnly);
    assert_eq!(
        behavior.validate_fields(FieldBehaviorKind::Required, FieldScope::All),
        Ok(())
    );
}

#[test]
fn a_well_known_type_has_the_defaults() {
    let mut empty = buffa_types::Empty::default();
    empty.clear_fields(FieldBehaviorKind::OutputOnly);
    let mut mask = buffa_types::FieldMask {
        paths: vec!["title".to_owned()],
        ..Default::default()
    };
    mask.clear_fields(FieldBehaviorKind::OutputOnly);
    assert_eq!(mask.paths, ["title"]);
    assert_eq!(
        buffa_types::Timestamp::default()
            .validate_fields(FieldBehaviorKind::Required, FieldScope::All),
        Ok(())
    );
}
