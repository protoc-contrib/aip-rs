//! AIP-193 errors on a Connect server: the details an error carries, the
//! `google.rpc.Status` of a failed batch entry, and the [`ErrorInterceptor`]
//! that logs a failed call once and answers the client without what is only
//! for the log.
//!
//! The Rust counterpart of aip-go's `errors.go` and `error_interceptor.go`.
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
//! and returns it. [`ErrorInterceptor`] is where that is told from what a
//! client may have: it logs the error with its cause and where it happened,
//! then answers without the `DebugInfo` -- AIP-193 keeps debugging information
//! out of a response -- and with the request's id as a `RequestInfo`, which a
//! client can quote. A failed entry of a `Batch*` response is an error that is
//! not returned but carried, as a `google.rpc.Status`: [`entry_status`]
//! builds it the same way, and [`log_entry`] logs the entries that were the
//! server's fault.

use std::{backtrace::Backtrace, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use buffa::{Message as _, MessageName};
use buffa_protos::google::rpc::{
    BadRequest, DebugInfo, ErrorInfo, RequestInfo, ResourceInfo, RetryInfo, Status,
    bad_request::FieldViolation,
};
use buffa_types::google::protobuf::Any;
use connectrpc::interceptor::{UnaryRequest, UnaryResponse};
use connectrpc::{
    ConnectError, ErrorCode, ErrorDetail, Interceptor, Next, RequestContext, SpecOrigin,
};

use crate::{FieldError, FieldResolver, ResourceName};

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

/// The [`FieldError`] for `error`, a failure to read an id from the resource
/// name a request carries, such as a wildcard where an id is needed. The field
/// is the request's own: its `name`, or its `parent` where it has no `name`
/// (AIP-131 to AIP-135). As aip-go's `NameRequestError`.
///
/// # Panics
///
/// If `M` has neither field: a handler naming such a request is a bug.
#[must_use]
pub fn name_request_error<M: FieldResolver, E>(error: E) -> FieldError<E> {
    let path = M::field_path("name")
        .or_else(|| M::field_path("parent"))
        .unwrap_or_else(|| {
            panic!(
                "`{}` has neither a name nor a parent field",
                core::any::type_name::<M>()
            )
        });
    FieldError::new(path, error)
}

/// The `google.rpc.RequestInfo` detail of an error that answers the request
/// with the id `request_id`: the id the logs carry, for a client to quote.
#[must_use]
pub fn request_info(request_id: &str) -> ErrorDetail {
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
/// [`ErrorInterceptor`] logs it and answers without it, and
/// [`status`] leaves it out of a failed batch entry.
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

/// The `M` detail `error` carries, if it carries one: how a client reads
/// [`error_info`] and the others back, and how [`ErrorInterceptor`] reads the
/// [`debug_info`] it logs.
#[must_use]
pub fn find_detail<M: buffa::Message + MessageName>(error: &ConnectError) -> Option<M> {
    let detail = error
        .details
        .iter()
        .find(|detail| detail.type_url == M::FULL_NAME)?;
    M::decode_from_slice(&decode_value(detail)?).ok()
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
/// request failed: the code, numbered as gRPC numbers it, the message, and the
/// details a unary error would carry -- as `Any` messages, which Connect
/// carries as base64 text -- less its `google.rpc.DebugInfo` (AIP-193). As
/// aip-go's `NewStatus`, which also drops the `DebugInfo`, in the interceptor.
#[must_use]
pub fn status(error: &ConnectError) -> Status {
    build_status(error, Some)
}

/// [`status`] for the entry at `index` of a batch request's `requests`, with
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

/// `error` with an `ErrorInfo` in `domain`, its code as the reason --
/// `INVALID_ARGUMENT`, `NOT_FOUND` -- first among its details, unless it has
/// one already. AIP-193 asks for an `ErrorInfo` on every error; a service
/// builds its own where it knows a reason more specific than the code, and this
/// covers the errors it does not build. What aip-go's `WithDefaultErrorInfo`
/// does to every error the interceptor answers, for one that never reaches it.
#[must_use]
pub fn with_default_error_info(mut error: ConnectError, domain: &str) -> ConnectError {
    if !error
        .details
        .iter()
        .any(|detail| detail.type_url == ErrorInfo::FULL_NAME)
    {
        let reason = error.code.as_str().to_ascii_uppercase();
        error.details.insert(0, error_info(domain, &reason));
    }
    error
}

/// What [`status`] and [`entry_status`] share: the status of `error`, each of
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

/// Logs the failed entry at `index` of a `Batch*` response, when it was the
/// server's fault -- [`log_level`] is ERROR -- with
/// what failed underneath and where. An entry that was the caller's mistake is
/// not logged: it is in the response for the caller to read.
///
/// A failed entry is not returned as an error, so [`ErrorInterceptor`] never
/// sees it; whatever builds the entry's status calls this beside it.
pub fn log_entry(error: &ConnectError, index: usize) {
    if log_level(error.code) != tracing::Level::ERROR {
        return;
    }
    let reason = error.message.as_deref().unwrap_or_default();
    let (source, stack) = causes(error);
    tracing::error!(index, code = ?error.code, reason, source, stack, "batch entry failed");
}

/// What failed underneath `error`, which the client never sees, and where: the
/// stack of the `DebugInfo` a handler attached.
fn causes(error: &ConnectError) -> (String, String) {
    let source = std::error::Error::source(error)
        .map(ToString::to_string)
        .unwrap_or_default();
    let stack = find_detail::<DebugInfo>(error)
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
/// At the level [`log_level`] gives its code: ERROR for a
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
    /// the reason: see [`with_default_error_info`](fn@with_default_error_info).
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
        if log_level(error.code) == tracing::Level::ERROR {
            tracing::error!(procedure, origin, code = ?error.code, reason, source, stack, "rpc failed");
        } else {
            tracing::warn!(procedure, origin, code = ?error.code, reason, source, stack, "rpc failed");
        }

        // The DebugInfo is for the log, not the client (AIP-193).
        error
            .details
            .retain(|detail| detail.type_url != DebugInfo::FULL_NAME);
        if let Some(domain) = &self.domain {
            error = with_default_error_info(error, domain);
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

        let info = find_detail::<ErrorInfo>(&error).expect("an ErrorInfo");
        assert_eq!(info.reason, "SHELF_NOT_FOUND");
        assert_eq!(info.domain, "example.com");
    }

    #[test]
    fn resource_info_names_the_type_and_the_name() {
        let error = ConnectError::not_found("x").with_detail(resource_info(&Shelf));

        let info = find_detail::<ResourceInfo>(&error).expect("a ResourceInfo");
        assert_eq!(info.resource_type, "example.com/Shelf");
        assert_eq!(info.resource_name, "shelves/s1");
    }

    #[test]
    fn retry_info_carries_the_delay() {
        let error =
            ConnectError::unavailable("x").with_detail(retry_info(Duration::from_millis(1500)));

        let delay = find_detail::<RetryInfo>(&error)
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

        let request = find_detail::<BadRequest>(&error).expect("a BadRequest");
        let [violation] = request.field_violations.as_slice() else {
            panic!("one violation: {request:?}");
        };
        assert_eq!(violation.field, "name");
        assert_eq!(violation.description, "value is invalid");
    }

    /// A request with the fields it is given, as the generated impls name them.
    struct Fields<const NAME: bool, const PARENT: bool>;

    impl<const NAME: bool, const PARENT: bool> FieldResolver for Fields<NAME, PARENT> {
        fn field_path(name: &str) -> Option<crate::FieldPath> {
            use crate::{FieldPath, FieldType};
            match name {
                "name" if NAME => Some(FieldPath::field("name", 1, FieldType::String)),
                "parent" if PARENT => Some(FieldPath::field("parent", 1, FieldType::String)),
                _ => None,
            }
        }
    }

    #[test]
    fn a_name_failure_is_about_the_name_or_else_the_parent() {
        let error = name_request_error::<Fields<true, true>, _>("a wildcard");
        assert_eq!(error.path().to_string(), "name");

        let error = name_request_error::<Fields<false, true>, _>("a wildcard");
        assert_eq!(error.path().to_string(), "parent");
        assert_eq!(*error.failure(), "a wildcard");
    }

    #[test]
    #[should_panic(expected = "neither a name nor a parent")]
    fn a_name_failure_of_a_request_with_no_name_is_a_bug() {
        let _ = name_request_error::<Fields<false, false>, _>("a wildcard");
    }

    #[test]
    fn debug_info_has_the_cause_and_where() {
        let cause = std::io::Error::other("connection reset");
        let error = ConnectError::internal("x").with_detail(debug_info(&cause));

        let info = find_detail::<DebugInfo>(&error).expect("a DebugInfo");
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
        let status = status(
            &ConnectError::not_found("The requested shelf was not found")
                .with_detail(error_info("example.com", "SHELF_NOT_FOUND"))
                .with_detail(resource_info(&Shelf))
                .with_detail(debug_info(&cause)),
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

    #[test]
    fn an_error_with_no_error_info_gets_its_code_as_the_reason_first() {
        let error = with_default_error_info(
            ConnectError::invalid_argument("x").with_detail(request_info("r1")),
            "example.com",
        );

        assert_eq!(error.details[0].type_url, ErrorInfo::FULL_NAME);
        let info = find_detail::<ErrorInfo>(&error).expect("an ErrorInfo");
        assert_eq!(info.reason, "INVALID_ARGUMENT");
        assert_eq!(info.domain, "example.com");
        assert_eq!(error.details.len(), 2);
    }

    #[test]
    fn an_error_with_an_error_info_keeps_it() {
        let error = with_default_error_info(
            ConnectError::not_found("x").with_detail(error_info("example.com", "SHELF_NOT_FOUND")),
            "example.com",
        );

        assert_eq!(error.details.len(), 1);
        let info = find_detail::<ErrorInfo>(&error).expect("an ErrorInfo");
        assert_eq!(info.reason, "SHELF_NOT_FOUND");
    }
}
