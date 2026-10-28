//! AIP-193 errors on a Connect server: the details an error carries, and the
//! `google.rpc.Status` of a failed batch entry. What logs a failed call, and
//! answers the client without what is only for the log, is the
//! [`ErrorInterceptor`](crate::error_interceptor::ErrorInterceptor).
//!
//! The Rust counterpart of aip-go's `errors.go`.
//! Nothing here knows a service: the domain of an `ErrorInfo`, the reasons and
//! the public messages are the service's, and it builds its errors from these
//! parts.
//!
//! ```
//! use aip::error::{debug_info, error_info};
//! use connectrpc::ConnectError;
//!
//! let cause = std::io::Error::other("connection reset");
//! let error = ConnectError::internal("Unable to complete the operation at this time")
//!     .with_detail(error_info("example.com", "INTERNAL_ERROR"))
//!     .with_detail(debug_info(&cause))
//!     .with_source(cause);
//! assert_eq!(error.details.len(), 2);
//! ```
//!
//! # What the client sees
//!
//! A handler builds an error with every detail it knows, `DebugInfo` included,
//! and returns it. The [`ErrorInterceptor`](crate::error_interceptor::ErrorInterceptor) is where that is told from what a
//! client may have: it logs the error with its cause and where it happened,
//! then answers without the `DebugInfo` -- AIP-193 keeps debugging information
//! out of a response -- and with the request's id as a `RequestInfo`, which a
//! client can quote. A failed entry of a `Batch*` response is an error that is
//! not returned but carried, as a `google.rpc.Status`: [`entry_status`]
//! builds it the same way. Nothing wraps the response to find such an entry,
//! as aip-go's interceptor does, so whatever builds one logs it, as
//! [`log_level`] says it should be.

use std::{backtrace::Backtrace, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use buffa::{Message as _, MessageName};
use buffa_protos::google::rpc::{
    BadRequest, DebugInfo, ErrorInfo, RequestInfo, ResourceInfo, RetryInfo, Status,
    bad_request::FieldViolation,
};
use buffa_types::google::protobuf::Any;
use connectrpc::{ConnectError, ErrorCode, ErrorDetail};

use crate::{FieldError, ResourceName};

/// The `google.rpc.ErrorInfo` detail of an error (AIP-193): why it happened,
/// as `reason`, in `domain`, the name of the service that generated it.
///
/// A reason is unique within its domain, at most 63 characters, and matches
/// `[A-Z][A-Z0-9_]+[A-Z0-9]`, such as `ITEM_NOT_FOUND`.
#[must_use]
pub fn error_info(domain: &str, reason: &str) -> ErrorDetail {
    ErrorDetail::from_message(
        ErrorInfo::FULL_NAME,
        &ErrorInfo {
            reason: reason.to_owned(),
            domain: domain.to_owned(),
            ..ErrorInfo::default()
        },
    )
}

/// The `google.rpc.ResourceInfo` detail of an error about the resource named
/// `name`: its type and its name.
#[must_use]
pub fn resource_info(name: &impl ResourceName) -> ErrorDetail {
    ErrorDetail::from_message(
        ResourceInfo::FULL_NAME,
        &ResourceInfo {
            resource_type: name.resource_type().to_owned(),
            resource_name: name.to_string(),
            ..ResourceInfo::default()
        },
    )
}

/// The `google.rpc.RetryInfo` detail of an error the client may retry: how long
/// to wait before it does (AIP-194).
#[must_use]
pub fn retry_info(delay: Duration) -> ErrorDetail {
    ErrorDetail::from_message(
        RetryInfo::FULL_NAME,
        &RetryInfo {
            retry_delay: buffa::MessageField::some(delay.into()),
            ..RetryInfo::default()
        },
    )
}

/// The `google.rpc.BadRequest` detail of an error about the request field
/// `error` names (AIP-193): what a generated `parse*` returns, or a
/// [`FieldError`] a handler builds from a path it names with
/// [`FieldPath::of`](crate::FieldPath::of).
///
/// The description is fixed, `value is invalid`: the field says where, and the
/// `ErrorInfo` reason says which kind, and the error's own text can carry the
/// value.
#[must_use]
pub fn bad_request<E>(error: &FieldError<E>) -> ErrorDetail {
    ErrorDetail::from_message(
        BadRequest::FULL_NAME,
        &BadRequest {
            field_violations: vec![FieldViolation {
                field: error.path().to_string(),
                description: "value is invalid".to_owned(),
                ..FieldViolation::default()
            }],
            ..BadRequest::default()
        },
    )
}

/// The `google.rpc.RequestInfo` detail of an error that answers the request
/// with the id `request_id`: the id the logs carry, for a client to quote.
#[must_use]
pub(crate) fn request_info(request_id: &str) -> ErrorDetail {
    ErrorDetail::from_message(
        RequestInfo::FULL_NAME,
        &RequestInfo {
            request_id: request_id.to_owned(),
            ..RequestInfo::default()
        },
    )
}

/// The `google.rpc.DebugInfo` detail of an error caused by `cause`: its text,
/// and the stack where the detail was built.
///
/// AIP-193 keeps debugging information out of what a client sees:
/// [`ErrorInterceptor`](crate::error_interceptor::ErrorInterceptor) logs it and answers without it, and
/// [`entry_status`] leaves it out of a failed batch entry.
#[must_use]
pub fn debug_info(cause: &dyn std::error::Error) -> ErrorDetail {
    ErrorDetail::from_message(
        DebugInfo::FULL_NAME,
        &DebugInfo {
            detail: cause.to_string(),
            stack_entries: Backtrace::capture()
                .to_string()
                .lines()
                .map(|line| line.trim().to_owned())
                .collect(),
            ..DebugInfo::default()
        },
    )
}

/// Reading a detail back from a [`ConnectError`]: a trait because the type is the
/// `connectrpc` crate's, and what lets a caller write
/// `error.detail::<ErrorInfo>()`.
pub trait ConnectErrorExt {
    /// The `M` detail the error carries, if it carries one: how a client reads
    /// [`error_info`] and the others back, and how [`ErrorInterceptor`](crate::error_interceptor::ErrorInterceptor) reads the
    /// [`debug_info`] it logs.
    ///
    /// The first of that type, since AIP-193 sends each at most once. A detail
    /// that is there and does not decode is no different from one that is not:
    /// `None`.
    #[must_use]
    fn detail<M: buffa::Message + MessageName>(&self) -> Option<M>;
}

impl ConnectErrorExt for ConnectError {
    fn detail<M: buffa::Message + MessageName>(&self) -> Option<M> {
        let detail = self
            .details
            .iter()
            .find(|detail| detail.type_url == M::FULL_NAME)?;
        M::decode_from_slice(&decode_value(detail)?).ok()
    }
}

/// A detail's bytes. The Connect protocol sends them unpadded, and a padded
/// value is accepted too.
fn decode_value(detail: &ErrorDetail) -> Option<Vec<u8>> {
    STANDARD_NO_PAD
        .decode(detail.value.as_deref()?.trim_end_matches('='))
        .ok()
}

/// The level a failed call of `code` logs at: ERROR for a fault the request did
/// not cause -- `Internal`, `Unknown`, `DataLoss`, `Unavailable` -- since it
/// needs someone to look, WARN for everything else, a caller's mistake -- a bad
/// name, a duplicate, a missing row -- which stays out of the error stream. As
/// aip-go's `ErrorLogLevel`.
#[must_use]
pub const fn log_level(code: ErrorCode) -> tracing::Level {
    match code {
        ErrorCode::Internal | ErrorCode::Unknown | ErrorCode::DataLoss | ErrorCode::Unavailable => {
            tracing::Level::ERROR
        }
        _ => tracing::Level::WARN,
    }
}

/// `error` as the `google.rpc.Status` a `Batch*` response entry carries when its
/// request failed, for the entry at `index` of a batch request's `requests`: the
/// code, numbered as gRPC numbers it, the message, and the details a unary error
/// would carry -- as `Any` messages, which Connect carries as base64 text -- less
/// its `google.rpc.DebugInfo` (AIP-193), with
/// the fields of its `google.rpc.BadRequest` named under that entry, as
/// `requests[1].name`: a field path is the request's, and the batch is the
/// request a client sent. As aip-go's `NewEntryStatus`.
#[must_use]
pub fn entry_status(error: &ConnectError, index: usize) -> Status {
    build_status(error, |detail| {
        if detail.type_url != BadRequest::FULL_NAME {
            return Some(detail);
        }
        // A BadRequest that does not read has no field to name under the
        // entry, and says nothing the entry's own code and message do not.
        let mut request = BadRequest::decode_from_slice(&decode_value(&detail)?).ok()?;
        for violation in &mut request.field_violations {
            violation.field = format!("requests[{index}].{}", violation.field);
        }
        Some(ErrorDetail::from_message(BadRequest::FULL_NAME, &request))
    })
}

/// What [`entry_status`] builds on: the status of `error`, each of
/// its details but the `DebugInfo` passed through `map`, which drops the ones it
/// answers `None` for.
fn build_status(
    error: &ConnectError,
    mut map: impl FnMut(ErrorDetail) -> Option<ErrorDetail>,
) -> Status {
    Status {
        // Every gRPC code, 1 to 16, fits; 2 is Unknown.
        code: i32::try_from(error.code.grpc_code()).unwrap_or(2),
        message: error.message.clone().unwrap_or_default(),
        details: error
            .details
            .iter()
            .filter(|detail| detail.type_url != DebugInfo::FULL_NAME)
            .filter_map(|detail| map(detail.clone()))
            .filter_map(|detail| {
                Some(Any {
                    type_url: format!("type.googleapis.com/{}", detail.type_url),
                    value: decode_value(&detail)?.into(),
                    ..Any::default()
                })
            })
            .collect(),
        ..Status::default()
    }
}

#[cfg(test)]
mod tests {
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
        let error =
            ConnectError::unavailable("x").with_detail(retry_info(Duration::from_millis(1500)));

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
}
