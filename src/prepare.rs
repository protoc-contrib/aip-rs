//! Readying a request for its handler: the [`RequestPreparer`] a request type
//! implements, and the [`PrepareInterceptor`] that calls it.
//!
//! The Rust counterpart of aip-go's `RequestPreparer` and
//! `NewPrepareInterceptor`. AIP leaves some things to the server that no
//! library decides for it -- a create's resource name, built from the request's
//! `parent` and its id (AIP-133); a List call's unspecified `page_size`
//! (AIP-158) -- and this is where a service does them: on the request, in a
//! hand-written file next to the generated code, before any handler runs.
//!
//! ```ignore
//! impl aip::RequestPreparer for ListBooksRequest {
//!     fn prepare(&mut self) -> Result<(), aip::PrepareError> {
//!         if self.page_size == 0 {
//!             self.page_size = 50;
//!         }
//!         Ok(())
//!     }
//! }
//!
//! static PREPARERS: &[aip::Preparer] = &[aip::Preparer::new::<ListBooksRequest>(
//!     "/acme.v1.Books/ListBooks",
//! )];
//!
//! let service = ConnectRpcService::new(router)
//!     .with_interceptor(aip::FieldBehaviorInterceptor::new(sdk::v1::PROCEDURES))
//!     .with_interceptor(aip::PrepareInterceptor::new(PREPARERS));
//! ```

use std::{error::Error, fmt};

use connectrpc::interceptor::{StreamRequest, StreamResponse, UnaryRequest, UnaryResponse};
use connectrpc::{
    ConnectError, Interceptor, JsonDeserialize, JsonSerialize, Next, NextStream, Payload,
    PayloadStream, RequestContext, SpecOrigin,
};
use futures_util::StreamExt as _;

use crate::{
    FieldError,
    connect::{apply, decode},
    error::{bad_request, debug_info},
};

/// A request that readies itself for its handler, filling in what AIP leaves to
/// the server, such as a create's resource name (AIP-133). Every request a
/// service takes implements it, most with the default, which leaves it as it
/// is: what lets one table name each of them alike.
pub trait RequestPreparer {
    /// Fills in the request.
    ///
    /// # Errors
    ///
    /// What the request cannot be prepared from, with the field it names: the
    /// caller's mistake, which [`PrepareInterceptor`] answers as
    /// `InvalidArgument`.
    fn prepare(&mut self) -> Result<(), PrepareError> {
        Ok(())
    }
}

/// Why [`RequestPreparer::prepare`] could not prepare a request: the field it
/// could not read, and why -- as aip-go's `Prepare` returns a `*FieldError`.
pub type PrepareError = FieldError<PrepareFailure>;

/// What was wrong with the field of a [`PrepareError`]: any error, or a text,
/// made one with [`new`](Self::new). Its `Display` is the cause's, and the
/// field is the [`FieldError`]'s.
#[derive(Debug)]
pub struct PrepareFailure(Box<dyn Error + Send + Sync>);

impl PrepareFailure {
    /// The failure `error`, which can be a parse error or a plain text.
    #[must_use]
    pub fn new(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}

impl fmt::Display for PrepareFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for PrepareFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

/// Answers a request [`RequestPreparer::prepare`] refused, with the error to
/// return: where a service adds the details that are its own, such as an
/// `ErrorInfo`. See [`PrepareInterceptor::with_reject`].
pub type Reject = fn(PrepareError) -> ConnectError;

/// The default [`Reject`]: `InvalidArgument` with the fixed message "The
/// request contains invalid parameters", the field `error` names as a
/// `google.rpc.BadRequest`, and `error` as the cause, with its
/// `google.rpc.DebugInfo`. As aip-go's default.
#[must_use]
pub fn reject_prepare(error: PrepareError) -> ConnectError {
    ConnectError::invalid_argument("The request contains invalid parameters")
        .with_detail(bad_request(&error))
        .with_detail(debug_info(&error))
        .with_source(error)
}

/// How one procedure's request is prepared: its `path`, and the step that
/// decodes each message as its own type, prepares it and puts it back.
///
/// Built with [`Preparer::new`]. Where aip-go finds `Prepare` by reflection, a
/// table of these names every procedure's request type.
#[derive(Debug, Clone, Copy)]
pub struct Preparer {
    /// The procedure's path, e.g. `/acme.v1.Books/CreateBook`.
    pub path: &'static str,
    /// Applied to each message the server receives, with how a refusal is
    /// answered.
    pub prepare: fn(&mut Payload, Reject) -> Result<(), ConnectError>,
}

impl Preparer {
    /// A procedure whose request is an `M`.
    #[must_use]
    pub const fn new<M>(path: &'static str) -> Self
    where
        M: buffa::Message + JsonSerialize + JsonDeserialize + Clone + RequestPreparer + 'static,
    {
        Self {
            path,
            prepare: prepare_payload::<M>,
        }
    }
}

/// [`RequestPreparer::prepare`] on a payload holding an `M`.
fn prepare_payload<M>(payload: &mut Payload, reject: Reject) -> Result<(), ConnectError>
where
    M: buffa::Message + JsonSerialize + JsonDeserialize + Clone + RequestPreparer + 'static,
{
    let mut request = decode::<M>(payload)?.clone();
    request.prepare().map_err(reject)?;
    payload.set_message(request);
    Ok(())
}

/// Calls each request's [`RequestPreparer`] before its handler, as aip-go's
/// `NewPrepareInterceptor` does.
///
/// Register it after [`FieldBehaviorInterceptor`](crate::FieldBehaviorInterceptor),
/// so an omitted id is minted and every field validated by then. Requests are
/// found by procedure in the table it is given, and decoded once as their own
/// type. Calls a client makes, and procedures the table does not list, pass
/// through. A refusal is answered by [`reject_prepare`], or by what
/// [`with_reject`](Self::with_reject) sets.
#[derive(Debug, Clone, Copy)]
pub struct PrepareInterceptor {
    preparers: &'static [Preparer],
    reject: Reject,
}

impl PrepareInterceptor {
    /// An interceptor for the procedures `preparers` names.
    #[must_use]
    pub const fn new(preparers: &'static [Preparer]) -> Self {
        Self {
            preparers,
            reject: reject_prepare,
        }
    }

    /// Answers a request `prepare` refused with `reject`, in place of
    /// [`reject_prepare`]. As aip-go's `reject` argument.
    #[must_use]
    pub const fn with_reject(mut self, reject: Reject) -> Self {
        self.reject = reject;
        self
    }

    /// The preparer of the procedure a server is serving in `ctx`, if the table
    /// lists it.
    fn preparer(&self, ctx: &RequestContext) -> Option<&'static Preparer> {
        let spec = ctx.spec()?;
        if spec.origin != SpecOrigin::Server {
            return None;
        }
        self.preparers
            .iter()
            .find(|preparer| preparer.path == spec.procedure)
    }
}

#[connectrpc::async_trait]
impl Interceptor for PrepareInterceptor {
    async fn intercept_unary(
        &self,
        mut request: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        if let Some(preparer) = self.preparer(&request.ctx) {
            (preparer.prepare)(&mut request.payload, self.reject)?;
        }
        next.run(request).await
    }

    async fn intercept_streaming(
        &self,
        request: StreamRequest,
        inbound: PayloadStream,
        next: NextStream<'_>,
    ) -> Result<StreamResponse, ConnectError> {
        let Some(preparer) = self.preparer(&request.ctx) else {
            return next.run(request, inbound).await;
        };
        let (prepare, reject) = (preparer.prepare, self.reject);
        let inbound: PayloadStream =
            Box::pin(inbound.map(move |item| apply(|payload| prepare(payload, reject), item)));
        next.run(request, inbound).await
    }
}

#[cfg(test)]
mod tests;
