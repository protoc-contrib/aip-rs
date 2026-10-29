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
//!     fn prepare(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

use std::error::Error;

use connectrpc::interceptor::{StreamRequest, StreamResponse, UnaryRequest, UnaryResponse};
use connectrpc::{
    ConnectError, Interceptor, JsonDeserialize, JsonSerialize, Next, NextStream, Payload,
    PayloadStream, RequestContext, SpecOrigin,
};
use futures_util::StreamExt as _;

use super::behavior::{apply, decode};
use crate::{
    FieldError,
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
    /// What the request cannot be prepared from: the caller's mistake, which
    /// [`PrepareInterceptor`] answers as `InvalidArgument`. A [`FieldError`] names
    /// the field it is about, as a `*FieldError` does in aip-go, where `Prepare`
    /// returns any `error`.
    fn prepare(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
}

/// Answers a request [`RequestPreparer::prepare`] refused, with the error to
/// return: where a service adds the details that are its own, such as an
/// `ErrorInfo`. See [`PrepareInterceptor::with_reject`].
type Reject = fn(Box<dyn Error + Send + Sync>) -> ConnectError;

/// The default [`Reject`]: `InvalidArgument` with the fixed message "The
/// request contains invalid parameters", the field `error` names, if it is a
/// [`FieldError`], as a `google.rpc.BadRequest`, and `error` as the cause, with
/// its `google.rpc.DebugInfo`. As aip-go's default.
fn reject_prepare(error: Box<dyn Error + Send + Sync>) -> ConnectError {
    let mut rejected = ConnectError::invalid_argument("The request contains invalid parameters");
    if let Some(field) = error.downcast_ref::<FieldError>() {
        rejected = rejected.with_detail(bad_request(field));
    }
    rejected.with_detail(debug_info(&*error)).with_source(error)
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
/// through. A refusal is answered with `InvalidArgument`, naming the field a
/// [`FieldError`] is about, or by what [`with_reject`](Self::with_reject) sets.
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

    /// Answers a request `prepare` refused with `reject`, in place of the
    /// default. As aip-go's `reject` argument.
    #[must_use]
    pub const fn with_reject(
        mut self,
        reject: fn(Box<dyn Error + Send + Sync>) -> ConnectError,
    ) -> Self {
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
mod tests {
    use buffa_types::FieldMask;
    use bytes::Bytes;
    use connectrpc::handler::handler_fn;
    use connectrpc::{ConnectRpcService, Router, Spec, StreamType};
    use http_body_util::{BodyExt as _, Full};
    use tower::ServiceExt as _;

    use super::*;
    use crate::error::ConnectErrorExt as _;

    /// A request that upper-cases its paths, and refuses one that is empty, naming
    /// it, or one that is `boom`, as any error that names no field.
    impl RequestPreparer for FieldMask {
        fn prepare(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
            for (index, path) in self.paths.iter_mut().enumerate() {
                if path.is_empty() {
                    return Err(FieldError::new(
                        crate::FieldPath::index("paths", 1, crate::FieldType::String, index),
                        "it is empty",
                    )
                    .into());
                }
                if path == "boom" {
                    return Err("it does not like that".into());
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
    async fn an_error_that_names_no_field_is_refused_without_one() {
        let service = service(PrepareInterceptor::new(PREPARERS));

        let (message, field) = call(&service, ECHO, &["boom"]).await.unwrap_err();

        assert_eq!(message, "The request contains invalid parameters");
        assert_eq!(field, "");
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
}
