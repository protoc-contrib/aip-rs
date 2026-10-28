use buffa_types::FieldMask;
use bytes::Bytes;
use connectrpc::handler::handler_fn;
use connectrpc::{ConnectRpcService, Router, Spec, StreamType};
use http_body_util::{BodyExt as _, Full};
use tower::ServiceExt as _;

use super::*;
use crate::error::ConnectErrorExt as _;

/// A request that upper-cases its paths, and refuses one that is empty.
impl RequestPreparer for FieldMask {
    fn prepare(&mut self) -> Result<(), PrepareError> {
        for (index, path) in self.paths.iter_mut().enumerate() {
            if path.is_empty() {
                return Err(FieldError::new(
                    crate::FieldPath::index("paths", 1, crate::FieldType::String, index),
                    PrepareFailure::new("it is empty"),
                ));
            }
            *path = path.to_uppercase();
        }
        Ok(())
    }
}

const ECHO: &str = "/example.v1.Masks/Echo";
const UNLISTED: &str = "/example.v1.Masks/Unlisted";
const CLIENT: &str = "/example.v1.Masks/Client";

static PREPARERS: &[Preparer] = &[
    Preparer::new::<FieldMask>(ECHO),
    Preparer::new::<FieldMask>(CLIENT),
];

fn service(interceptor: PrepareInterceptor) -> ConnectRpcService {
    let echo = || handler_fn(|_, request: FieldMask| async move { Ok(request.into()) });
    let router = Router::new()
        .route("example.v1.Masks", "Echo", echo())
        .route("example.v1.Masks", "Unlisted", echo())
        .route("example.v1.Masks", "Client", echo())
        .with_spec(Spec::server(ECHO, StreamType::Unary))
        .with_spec(Spec::server(UNLISTED, StreamType::Unary))
        .with_spec(Spec::client(CLIENT, StreamType::Unary));
    ConnectRpcService::new(router).with_interceptor(interceptor)
}

/// The paths a call answered with, or the error: its message and the field
/// its `BadRequest` names.
async fn call(
    service: &ConnectRpcService,
    procedure: &str,
    paths: &[&str],
) -> Result<Vec<String>, (String, String)> {
    use buffa::Message as _;

    let body = FieldMask {
        paths: paths.iter().map(ToString::to_string).collect(),
        ..Default::default()
    }
    .encode_to_vec();
    let request = http::Request::post(procedure)
        .header("content-type", "application/proto")
        .body(Full::new(Bytes::from(body)))
        .expect("a request");
    let response = service.clone().oneshot(request).await.expect("infallible");
    let ok = response.status().is_success();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    if ok {
        return Ok(FieldMask::decode_from_slice(&body).expect("a mask").paths);
    }
    let error: ConnectError = serde_json::from_slice(&body).expect("an error");
    let field = error
        .detail::<buffa_protos::google::rpc::BadRequest>()
        .and_then(|request| request.field_violations.first().map(|v| v.field.clone()))
        .unwrap_or_default();
    Err((error.message.unwrap_or_default(), field))
}

#[tokio::test]
async fn a_request_is_prepared_before_its_handler_sees_it() {
    let service = service(PrepareInterceptor::new(PREPARERS));

    assert_eq!(
        call(&service, ECHO, &["a", "b"]).await,
        Ok(vec!["A".into(), "B".into()])
    );
}

#[tokio::test]
async fn a_request_that_cannot_be_prepared_is_refused_naming_its_field() {
    let service = service(PrepareInterceptor::new(PREPARERS));

    let (message, field) = call(&service, ECHO, &["a", ""]).await.unwrap_err();

    assert_eq!(message, "The request contains invalid parameters");
    assert_eq!(field, "paths[1]");
}

#[tokio::test]
async fn a_refusal_is_answered_as_the_service_says() {
    let service = service(
        PrepareInterceptor::new(PREPARERS)
            .with_reject(|_| ConnectError::invalid_argument("not like that")),
    );

    let (message, _) = call(&service, ECHO, &[""]).await.unwrap_err();

    assert_eq!(message, "not like that");
}

#[tokio::test]
async fn a_call_a_client_makes_and_a_procedure_not_in_the_table_pass_through() {
    let service = service(PrepareInterceptor::new(PREPARERS));

    assert_eq!(call(&service, CLIENT, &["a"]).await, Ok(vec!["a".into()]));
    assert_eq!(call(&service, UNLISTED, &["a"]).await, Ok(vec!["a".into()]));
}
