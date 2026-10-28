use super::*;

struct Shelf;

impl std::fmt::Display for Shelf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("shelves/s1")
    }
}

impl ResourceName for Shelf {
    fn resource_type(&self) -> &str {
        "example.com/Shelf"
    }

    fn pattern(&self) -> &str {
        "shelves/{shelf}"
    }

    fn validate(&self) -> Result<(), crate::resource::InvalidResourceIdError> {
        Ok(())
    }

    fn contains_wildcard(&self) -> bool {
        false
    }
}

#[test]
fn error_info_carries_the_reason_in_its_domain() {
    let error =
        ConnectError::not_found("x").with_detail(error_info("example.com", "SHELF_NOT_FOUND"));

    let info = error.detail::<ErrorInfo>().expect("an ErrorInfo");
    assert_eq!(info.reason, "SHELF_NOT_FOUND");
    assert_eq!(info.domain, "example.com");
}

#[test]
fn resource_info_names_the_type_and_the_name() {
    let error = ConnectError::not_found("x").with_detail(resource_info(&Shelf));

    let info = error.detail::<ResourceInfo>().expect("a ResourceInfo");
    assert_eq!(info.resource_type, "example.com/Shelf");
    assert_eq!(info.resource_name, "shelves/s1");
}

#[test]
fn retry_info_carries_the_delay() {
    let error = ConnectError::unavailable("x").with_detail(retry_info(Duration::from_millis(1500)));

    let delay = error
        .detail::<RetryInfo>()
        .and_then(|info| info.retry_delay.as_option().cloned())
        .expect("a delay");
    assert_eq!((delay.seconds, delay.nanos), (1, 500_000_000));
}

#[test]
fn bad_request_names_the_field_with_a_fixed_description() {
    use crate::{FieldPath, FieldType};

    let secret = FieldError::new(
        FieldPath::field("name", 1, FieldType::String),
        "shelves/s1 is taken",
    );
    let error = ConnectError::invalid_argument("x").with_detail(bad_request(&secret));

    let request = error.detail::<BadRequest>().expect("a BadRequest");
    let [violation] = request.field_violations.as_slice() else {
        panic!("one violation: {request:?}");
    };
    assert_eq!(violation.field, "name");
    assert_eq!(violation.description, "value is invalid");
}

#[test]
fn debug_info_has_the_cause_and_where() {
    let cause = std::io::Error::other("connection reset");
    let error = ConnectError::internal("x").with_detail(debug_info(&cause));

    let info = error.detail::<DebugInfo>().expect("a DebugInfo");
    assert_eq!(info.detail, "connection reset");
    assert!(!info.stack_entries.is_empty());
}

#[test]
fn a_server_fault_logs_as_an_error_and_a_caller_s_mistake_as_a_warning() {
    for error in [
        ConnectError::internal("x"),
        ConnectError::new(ErrorCode::Unknown, "x"),
        ConnectError::new(ErrorCode::DataLoss, "x"),
        ConnectError::unavailable("x"),
    ] {
        assert_eq!(
            log_level(error.code),
            tracing::Level::ERROR,
            "{:?}",
            error.code
        );
    }
    for error in [
        ConnectError::invalid_argument("x"),
        ConnectError::not_found("x"),
        ConnectError::already_exists("x"),
        ConnectError::failed_precondition("x"),
        ConnectError::aborted("x"),
    ] {
        assert_eq!(
            log_level(error.code),
            tracing::Level::WARN,
            "{:?}",
            error.code
        );
    }
}

#[test]
fn a_status_carries_the_code_the_message_and_every_detail_but_the_debug_info() {
    let cause = std::io::Error::other("connection reset");
    let status = entry_status(
        &ConnectError::not_found("The requested shelf was not found")
            .with_detail(error_info("example.com", "SHELF_NOT_FOUND"))
            .with_detail(resource_info(&Shelf))
            .with_detail(debug_info(&cause)),
        0,
    );

    assert_eq!(status.code, 5);
    assert_eq!(status.message, "The requested shelf was not found");
    let types: Vec<&str> = status
        .details
        .iter()
        .map(|detail| detail.type_url.as_str())
        .collect();
    assert_eq!(
        types,
        [
            "type.googleapis.com/google.rpc.ErrorInfo",
            "type.googleapis.com/google.rpc.ResourceInfo"
        ]
    );
    let info = ErrorInfo::decode_from_slice(&status.details[0].value).expect("an ErrorInfo");
    assert_eq!(info.reason, "SHELF_NOT_FOUND");
}

#[test]
fn an_entry_status_names_the_field_under_the_entry() {
    use crate::{FieldPath, FieldType};

    let field = FieldError::new(
        FieldPath::field("name", 1, FieldType::String),
        "no such shelf",
    );
    let status = entry_status(
        &ConnectError::invalid_argument("The request contains invalid parameters")
            .with_detail(error_info("example.com", "INVALID_NAME"))
            .with_detail(bad_request(&field)),
        2,
    );

    let request = status
        .details
        .iter()
        .find(|detail| detail.type_url.ends_with("google.rpc.BadRequest"))
        .map(|detail| BadRequest::decode_from_slice(&detail.value).expect("a BadRequest"))
        .expect("a BadRequest");
    assert_eq!(request.field_violations[0].field, "requests[2].name");
    // The other details are as they were.
    assert_eq!(status.details.len(), 2);
}
