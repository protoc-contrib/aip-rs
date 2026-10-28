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
            (name == "update_mask").then(|| FieldPath::field("update_mask", 2, FieldType::Message))
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
