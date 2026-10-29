//! [`ErrorInterceptor`]: logs a failed call once, and answers it without what is
//! only for the log.
//!
//! The Rust counterpart of aip-go's `error_interceptor.go`. The details an error
//! carries are [`crate::error`]'s.

use std::sync::Arc;

use buffa::MessageName as _;
use buffa_protos::google::rpc::{DebugInfo, ErrorInfo};
use connectrpc::interceptor::{UnaryRequest, UnaryResponse};
use connectrpc::{ConnectError, Interceptor, Next, RequestContext, SpecOrigin};

use crate::error::{ConnectErrorExt as _, error_info, error_log_level, request_info};

/// What failed underneath `error`, which the client never sees, and where: the
/// stack of the `DebugInfo` a handler attached.
fn causes(error: &ConnectError) -> (String, String) {
    let source = std::error::Error::source(error)
        .map(ToString::to_string)
        .unwrap_or_default();
    let stack = error
        .detail::<DebugInfo>()
        .map(|info| info.stack_entries.join("\n"))
        .unwrap_or_default();
    (source, stack)
}

/// What [`ErrorInterceptor`] needs from the request to find its id.
type RequestId = Arc<dyn Fn(&RequestContext) -> Option<String> + Send + Sync>;

/// Logs every RPC that fails, once, with its cause, and answers it without what
/// is only for the log.
///
/// A handler builds its error rather than logging it. A [`ConnectError`]
/// carries its cause as a source, which is kept out of the response body, and
/// this is where it is written down: beside the procedure and which side of the
/// call it was, into whatever span is current, so a server that opens one per
/// request ties each line to its request.
///
/// At the level [`error_log_level`] gives its code: ERROR for a
/// fault the request did not cause, WARN for a caller's mistake. The error's
/// public message goes in as `reason`, its source, what actually failed, as
/// `source`, and where as `stack`: the stack of its `google.rpc.DebugInfo`.
///
/// The client then gets the error less that `DebugInfo` (AIP-193) and with
/// two details added where they are wanted:
///
/// - [`with_default_error_info`](Self::with_default_error_info): an
///   `ErrorInfo` for an error that has none, its code as the reason, which is
///   how the errors no handler built -- a request the validation refused, a
///   body that does not decode -- carry one too;
/// - [`with_request_id`](Self::with_request_id): the id of the request as a
///   `RequestInfo`, which a client that does not see the response headers can
///   still quote.
///
/// Both are off until asked for. Register it first among the interceptors,
/// with `ConnectRpcService::with_interceptor`, so what the others refuse is
/// logged and answered the same way. Unary calls only: a streamed call is not
/// touched, and a call a client makes passes through logged as the client's.
#[derive(Clone, Default)]
pub struct ErrorInterceptor {
    domain: Option<Arc<str>>,
    request_id: Option<RequestId>,
}

impl std::fmt::Debug for ErrorInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorInterceptor")
            .field("domain", &self.domain)
            .field("request_id", &self.request_id.is_some())
            .finish()
    }
}

impl ErrorInterceptor {
    /// An interceptor that logs, and strips the `DebugInfo`, and adds nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Give an error that has no `ErrorInfo` one in `domain`, with its code as
    /// the reason: see [`with_default_error_info`](Self::with_default_error_info).
    #[must_use]
    pub fn with_default_error_info(mut self, domain: impl Into<Arc<str>>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Send the client the id of the request, which `request_id` finds, as a
    /// `RequestInfo`. A request it finds none for gets none.
    #[must_use]
    pub fn with_request_id(
        mut self,
        request_id: impl Fn(&RequestContext) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.request_id = Some(Arc::new(request_id));
        self
    }
}

#[connectrpc::async_trait]
impl Interceptor for ErrorInterceptor {
    async fn intercept_unary(
        &self,
        request: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        // Read before the request moves into the call.
        let procedure = request.ctx.spec().map_or("", |spec| spec.procedure);
        let origin = origin(&request.ctx);
        let request_id = self
            .request_id
            .as_ref()
            .and_then(|request_id| request_id(&request.ctx));

        let mut error = match next.run(request).await {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };

        // `message` is the one field name tracing reserves for the event's
        // own text, so the error's goes in as `reason`.
        let reason = error.message.as_deref().unwrap_or_default();
        let (source, stack) = causes(&error);
        if error_log_level(error.code) == tracing::Level::ERROR {
            tracing::error!(procedure, origin, code = ?error.code, reason, source, stack, "rpc failed");
        } else {
            tracing::warn!(procedure, origin, code = ?error.code, reason, source, stack, "rpc failed");
        }

        // The DebugInfo is for the log, not the client (AIP-193).
        error
            .details
            .retain(|detail| detail.type_url != DebugInfo::FULL_NAME);
        if let Some(domain) = &self.domain
            && !error
                .details
                .iter()
                .any(|detail| detail.type_url == ErrorInfo::FULL_NAME)
        {
            // Its code as the reason -- `INVALID_ARGUMENT`, `NOT_FOUND` --
            // first among its details.
            let reason = error.code.as_str().to_ascii_uppercase();
            error.details.insert(0, error_info(domain, &reason));
        }
        if let Some(request_id) = request_id {
            error.details.push(request_info(&request_id));
        }
        Err(error)
    }
}

/// Which side of the call `ctx` is: "server" or "client". `SpecOrigin` is
/// exhaustive -- a call is served or made -- and unset only for a request built
/// by hand, outside any dispatcher.
fn origin(ctx: &RequestContext) -> &'static str {
    match ctx.spec().map(|spec| spec.origin) {
        Some(SpecOrigin::Server) => "server",
        Some(SpecOrigin::Client) => "client",
        None => "",
    }
}

#[cfg(test)]
mod tests {
    //! [`ErrorInterceptor`] on the path a request takes through a connectrpc server
    //! -- HTTP in, the router, the interceptor chain, the handler, and back -- as
    //! aip-go's `error_interceptor_test.go` drives its own.
    //!
    //! What a client reads is the Connect error body, so each test calls over HTTP
    //! and reads the details back out of it.
    use super::ErrorInterceptor;
    use crate::error::{ConnectErrorExt as _, debug_info, error_info, resource_info, status};
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
        interceptor: ErrorInterceptor,
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
        let service = service(built, ErrorInterceptor::new());

        let (_, error) = call(&service, FAIL).await;

        assert_eq!(
            error.message.as_deref(),
            Some("The requested shelf was not found")
        );
        assert_eq!(details(&error), ["google.rpc.ErrorInfo"]);
    }

    #[tokio::test]
    async fn the_cause_never_reaches_the_client() {
        let service = service(built, ErrorInterceptor::new());

        let (body, _) = call(&service, FAIL).await;

        assert!(!body.contains("connection reset"), "{body}");
        assert!(!body.contains("DebugInfo"), "{body}");
    }

    #[tokio::test]
    async fn an_error_that_has_an_error_info_keeps_it() {
        let service = service(
            built,
            ErrorInterceptor::new().with_default_error_info("other.example.com"),
        );

        let (_, error) = call(&service, FAIL).await;

        let info = error.detail::<ErrorInfo>().expect("an ErrorInfo");
        assert_eq!(info.reason, "SHELF_NOT_FOUND");
        assert_eq!(info.domain, "example.com");
        assert_eq!(details(&error), ["google.rpc.ErrorInfo"]);
    }

    #[tokio::test]
    async fn an_error_that_has_none_gets_its_code_as_the_reason() {
        let service = service(
            || ConnectError::invalid_argument("The request contains invalid parameters"),
            ErrorInterceptor::new().with_default_error_info("example.com"),
        );

        let (_, error) = call(&service, FAIL).await;

        let info = error.detail::<ErrorInfo>().expect("an ErrorInfo");
        assert_eq!(info.reason, "INVALID_ARGUMENT");
        assert_eq!(info.domain, "example.com");
    }

    #[tokio::test]
    async fn an_error_gets_no_error_info_unless_asked_to() {
        let service = service(
            || ConnectError::invalid_argument("The request contains invalid parameters"),
            ErrorInterceptor::new(),
        );

        let (_, error) = call(&service, FAIL).await;

        assert!(error.details.is_empty(), "{:?}", details(&error));
    }

    #[tokio::test]
    async fn the_request_id_is_sent_as_a_request_info() {
        let service = service(
            built,
            ErrorInterceptor::new().with_request_id(|ctx| {
                ctx.header("x-request-id")
                    .and_then(|id| id.to_str().ok())
                    .map(str::to_owned)
            }),
        );

        let (_, error) = call(&service, FAIL).await;

        let info = error.detail::<RequestInfo>().expect("a RequestInfo");
        assert_eq!(info.request_id, "request-1");
        // After what the handler built, which the interceptor does not reorder.
        assert_eq!(
            details(&error),
            ["google.rpc.ErrorInfo", "google.rpc.RequestInfo"]
        );
    }

    #[tokio::test]
    async fn a_request_with_no_id_gets_no_request_info() {
        let service = service(built, ErrorInterceptor::new().with_request_id(|_| None));

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
        impl crate::ResourceName for Shelf {
            fn resource_type(&self) -> &str {
                "example.com/Shelf"
            }
            fn pattern(&self) -> &str {
                "shelves/{shelf}"
            }
            fn validate(&self) -> Result<(), crate::resource::ResourceNameError> {
                Ok(())
            }
        }

        let service = service(
            || built().with_detail(resource_info(&Shelf)),
            ErrorInterceptor::new(),
        );

        let (_, error) = call(&service, FAIL).await;

        assert_eq!(
            details(&error),
            ["google.rpc.ErrorInfo", "google.rpc.ResourceInfo"]
        );
    }

    #[tokio::test]
    async fn a_call_that_succeeds_is_answered_as_it_is() {
        let service = service(built, ErrorInterceptor::new());
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
        let error =
            ConnectError::internal("x").with_detail(debug_info(&std::io::Error::other("y")));

        let value = error.details[0].value.as_deref().expect("a value");
        let info = DebugInfo::decode_from_slice(&STANDARD_NO_PAD.decode(value).expect("base64"))
            .expect("a DebugInfo");
        assert_eq!(info.detail, "y");
        // And the status of the same error has none.
        assert!(status(&error).details.is_empty());
    }
}
