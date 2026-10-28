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

use crate::error::{ConnectErrorExt as _, error_info, log_level, request_info};

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
        if log_level(error.code) == tracing::Level::ERROR {
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
mod tests;
