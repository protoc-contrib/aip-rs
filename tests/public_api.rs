//! What a service reaches aip-rs through, from outside the crate: each
//! interceptor built and registered, and each function of `aip::error` called
//! as a handler calls it. The behaviour is tested beside the code, in each
//! module's `tests`; this is what keeps an export from going missing.
#![cfg(feature = "connect")]

use std::time::Duration;

use aip::error::{
    ConnectErrorExt as _, bad_request, debug_info, entry_status, error_info, log_level, retry_info,
};
use aip::error_interceptor::ErrorInterceptor;
use aip::{
    FieldBehaviorInterceptor, FieldError, FieldPath, FieldType, PrepareFailure, PrepareInterceptor,
    Preparer, Procedure, reject_prepare,
};
use buffa_protos::google::rpc::{BadRequest, ErrorInfo};
use connectrpc::{ConnectError, ConnectRpcService, ErrorCode, Router};

static PROCEDURES: &[Procedure] = &[];
static PREPARERS: &[Preparer] = &[];

#[test]
fn every_interceptor_registers_on_a_service() {
    let _service = ConnectRpcService::new(Router::new())
        .with_interceptor(
            ErrorInterceptor::new()
                .with_default_error_info("example.com")
                .with_request_id(|_| None),
        )
        .with_interceptor(FieldBehaviorInterceptor::new(PROCEDURES))
        .with_interceptor(PrepareInterceptor::new(PREPARERS).with_reject(reject_prepare));
}

#[test]
fn an_error_is_built_and_read_back_with_the_functions_a_handler_calls() {
    let field = FieldError::new(
        FieldPath::field("name", 1, FieldType::String),
        PrepareFailure::new("it is empty"),
    );
    let cause = std::io::Error::other("connection reset");
    let error = ConnectError::unavailable("The service is unavailable at this time")
        .with_detail(error_info("example.com", "SERVICE_UNAVAILABLE"))
        .with_detail(retry_info(Duration::from_secs(1)))
        .with_detail(bad_request(&field))
        .with_detail(debug_info(&cause))
        .with_source(cause);

    assert_eq!(
        error.detail::<ErrorInfo>().expect("an ErrorInfo").reason,
        "SERVICE_UNAVAILABLE"
    );
    assert_eq!(
        error
            .detail::<BadRequest>()
            .expect("a BadRequest")
            .field_violations[0]
            .field,
        "name"
    );
    assert_eq!(log_level(error.code), tracing::Level::ERROR);
    assert_eq!(log_level(ErrorCode::NotFound), tracing::Level::WARN);

    // A failed batch entry: the same details, less the DebugInfo.
    let status = entry_status(&error, 3);
    assert_eq!(status.code, 14);
    assert_eq!(status.details.len(), 3);
}
