//! [`ErrorInterceptor`] on the path a request takes through a connectrpc server
//! -- HTTP in, the router, the interceptor chain, the handler, and back -- as
//! aip-go's `error_interceptor_test.go` drives its own.
//!
//! What a client reads is the Connect error body, so each test calls over HTTP
//! and reads the details back out of it.
#![cfg(feature = "connect")]

use aip::error::{debug_info, entry_status, error_info, find_detail, resource_info};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use buffa::Message as _;
use buffa_protos::google::rpc::{DebugInfo, ErrorInfo, RequestInfo};
use buffa_types::google::protobuf::Empty;
use bytes::Bytes;
use connectrpc::handler::handler_fn;
use connectrpc::{ConnectError, ConnectRpcService, Router, Spec, StreamType};
use http_body_util::{BodyExt as _, Full};
use tower::ServiceExt as _;

const FAIL: &str = "/example.v1.Shelves/Fail";
const OK: &str = "/example.v1.Shelves/Ok";

/// Fails with `error`: a handler that built it as the service would, with its
/// cause and where.
fn service(
    error: impl Fn() -> ConnectError + Clone + Send + Sync + 'static,
    interceptor: aip::error::ErrorInterceptor,
) -> ConnectRpcService {
    let fail = move |_, _: Empty| {
        let error = error();
        async move { Err::<connectrpc::Response<Empty>, _>(error) }
    };
    let router = Router::new()
        .route("example.v1.Shelves", "Fail", handler_fn(fail))
        .route(
            "example.v1.Shelves",
            "Ok",
            handler_fn(|_, _: Empty| async { Ok(Empty::default().into()) }),
        )
        .with_spec(Spec::server(FAIL, StreamType::Unary))
        .with_spec(Spec::server(OK, StreamType::Unary));
    ConnectRpcService::new(router).with_interceptor(interceptor)
}

/// The error a call answered with, as a client reads it: its code, message and
/// details.
async fn call(service: &ConnectRpcService, procedure: &str) -> (String, ConnectError) {
    let request = http::Request::post(procedure)
        .header("content-type", "application/proto")
        .header("x-request-id", "request-1")
        .body(Full::new(Bytes::new()))
        .expect("a request");
    let response = service.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    assert!(!status.is_success(), "the call succeeded");
    let error: ConnectError = serde_json::from_slice(&body).expect("a Connect error");
    (String::from_utf8_lossy(&body).into_owned(), error)
}

fn details(error: &ConnectError) -> Vec<&str> {
    error
        .details
        .iter()
        .map(|detail| detail.type_url.as_str())
        .collect()
}

/// An error as a handler builds it: a reason, the resource, and the cause
/// with where.
fn built() -> ConnectError {
    let cause = std::io::Error::other("connection reset by peer");
    ConnectError::not_found("The requested shelf was not found")
        .with_detail(error_info("example.com", "SHELF_NOT_FOUND"))
        .with_detail(debug_info(&cause))
        .with_source(cause)
}

#[tokio::test]
async fn the_client_gets_the_error_without_its_debug_info() {
    let service = service(built, aip::error::ErrorInterceptor::new());

    let (_, error) = call(&service, FAIL).await;

    assert_eq!(
        error.message.as_deref(),
        Some("The requested shelf was not found")
    );
    assert_eq!(details(&error), ["google.rpc.ErrorInfo"]);
}

#[tokio::test]
async fn the_cause_never_reaches_the_client() {
    let service = service(built, aip::error::ErrorInterceptor::new());

    let (body, _) = call(&service, FAIL).await;

    assert!(!body.contains("connection reset"), "{body}");
    assert!(!body.contains("DebugInfo"), "{body}");
}

#[tokio::test]
async fn an_error_that_has_an_error_info_keeps_it() {
    let service = service(
        built,
        aip::error::ErrorInterceptor::new().with_default_error_info("other.example.com"),
    );

    let (_, error) = call(&service, FAIL).await;

    let info = find_detail::<ErrorInfo>(&error).expect("an ErrorInfo");
    assert_eq!(info.reason, "SHELF_NOT_FOUND");
    assert_eq!(info.domain, "example.com");
    assert_eq!(details(&error), ["google.rpc.ErrorInfo"]);
}

#[tokio::test]
async fn an_error_that_has_none_gets_its_code_as_the_reason() {
    let service = service(
        || ConnectError::invalid_argument("The request contains invalid parameters"),
        aip::error::ErrorInterceptor::new().with_default_error_info("example.com"),
    );

    let (_, error) = call(&service, FAIL).await;

    let info = find_detail::<ErrorInfo>(&error).expect("an ErrorInfo");
    assert_eq!(info.reason, "INVALID_ARGUMENT");
    assert_eq!(info.domain, "example.com");
}

#[tokio::test]
async fn an_error_gets_no_error_info_unless_asked_to() {
    let service = service(
        || ConnectError::invalid_argument("The request contains invalid parameters"),
        aip::error::ErrorInterceptor::new(),
    );

    let (_, error) = call(&service, FAIL).await;

    assert!(error.details.is_empty(), "{:?}", details(&error));
}

#[tokio::test]
async fn the_request_id_is_sent_as_a_request_info() {
    let service = service(
        built,
        aip::error::ErrorInterceptor::new().with_request_id(|ctx| {
            ctx.header("x-request-id")
                .and_then(|id| id.to_str().ok())
                .map(str::to_owned)
        }),
    );

    let (_, error) = call(&service, FAIL).await;

    let info = find_detail::<RequestInfo>(&error).expect("a RequestInfo");
    assert_eq!(info.request_id, "request-1");
    // After what the handler built, which the interceptor does not reorder.
    assert_eq!(
        details(&error),
        ["google.rpc.ErrorInfo", "google.rpc.RequestInfo"]
    );
}

#[tokio::test]
async fn a_request_with_no_id_gets_no_request_info() {
    let service = service(
        built,
        aip::error::ErrorInterceptor::new().with_request_id(|_| None),
    );

    let (_, error) = call(&service, FAIL).await;

    assert_eq!(details(&error), ["google.rpc.ErrorInfo"]);
}

#[tokio::test]
async fn the_other_details_are_sent_as_the_handler_built_them() {
    #[derive(Debug)]
    struct Shelf;
    impl std::fmt::Display for Shelf {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("shelves/s1")
        }
    }
    impl aip::ResourceName for Shelf {
        fn resource_type(&self) -> &str {
            "example.com/Shelf"
        }
        fn pattern(&self) -> &str {
            "shelves/{shelf}"
        }
        fn validate(&self) -> Result<(), aip::resource::InvalidResourceIdError> {
            Ok(())
        }
        fn contains_wildcard(&self) -> bool {
            false
        }
    }

    let service = service(
        || built().with_detail(resource_info(&Shelf)),
        aip::error::ErrorInterceptor::new(),
    );

    let (_, error) = call(&service, FAIL).await;

    assert_eq!(
        details(&error),
        ["google.rpc.ErrorInfo", "google.rpc.ResourceInfo"]
    );
}

#[tokio::test]
async fn a_call_that_succeeds_is_answered_as_it_is() {
    let service = service(built, aip::error::ErrorInterceptor::new());
    let request = http::Request::post(OK)
        .header("content-type", "application/proto")
        .body(Full::new(Bytes::new()))
        .expect("a request");

    let response = service.oneshot(request).await.expect("infallible");

    assert!(response.status().is_success());
}

/// The detail a Connect error body carries, as the bytes it was encoded from.
#[test]
fn a_detail_value_is_unpadded_base64() {
    let error = ConnectError::internal("x").with_detail(debug_info(&std::io::Error::other("y")));

    let value = error.details[0].value.as_deref().expect("a value");
    let info = DebugInfo::decode_from_slice(&STANDARD_NO_PAD.decode(value).expect("base64"))
        .expect("a DebugInfo");
    assert_eq!(info.detail, "y");
    // And the status of the same error has none.
    assert!(entry_status(&error, 0).details.is_empty());
}
