//! AIP field behavior applied at the RPC boundary of a Connect server: the
//! [`FieldBehaviorInterceptor`].
//!
//! The interceptor is the one opinionated part of this crate. It applies the
//! policy AIP-133, AIP-134 and AIP-203 describe for a server, with nothing to
//! configure:
//!
//! | Each message received | Each message sent |
//! |---|---|
//! | clear `OUTPUT_ONLY`; fill in a create's omitted `<resource>_id` (a UUIDv7) and an update's omitted `update_mask`, and drop the `OUTPUT_ONLY` paths of an explicit one; validate `REQUIRED` with protovalidate's rules, into one `InvalidArgument` | clear `INPUT_ONLY` |
//!
//! It acts on every message of every RPC, unary or streamed, as aip-go's
//! `NewFieldBehaviorInterceptor` does. A message that is not a standard-method
//! request -- a resource streamed on its own -- has nothing filled in or
//! checked for `REQUIRED`; only clearing and its protovalidate rules apply.
//!
//! # Names are the server's
//!
//! AIP-133 has the server build a created resource's `name` from the request's
//! `parent` and `<resource>_id`. The interceptor fills in only an omitted id;
//! it writes no name and no parent. A handler reads both off the request --
//! the generated `parse_<resource>_id` and `parse_parent` give them typed --
//! stores the resource under the name they make, and returns the name storage
//! holds:
//!
//! ```ignore
//! let parent = request.parse_parent()?;
//! let id = request.parse_book_id()?; // never empty here
//! let book = store.insert(BookName::new(parent, id), request.book).await?;
//! Ok(Response::new(book))
//! ```
//!
//! A service with another id scheme -- a slug, a sequence -- fills the id in
//! itself, with its own interceptor ahead of this one, through
//! [`ResourceWrites`]; an id that is already set is kept.
//!
//! # Wiring
//!
//! Interceptors see bytes, not types, so the interceptor is handed a table of
//! [`Procedure`]s naming each procedure's message types, which
//! protoc-gen-rust-aip generates per package as `PROCEDURES`.
//!
//! Behind the `connect` feature, on by default; turn it off with
//! `default-features = false` to use the crate without connectrpc,
//! protovalidate-buffa and uuid.

use buffa::{Message as _, MessageName as _};
use buffa_protos::google::rpc::{BadRequest, bad_request::FieldViolation};
use connectrpc::interceptor::{StreamRequest, StreamResponse, UnaryRequest, UnaryResponse};
use connectrpc::{
    ConnectError, ErrorCode, ErrorDetail, Interceptor, JsonDeserialize, JsonSerialize, Next,
    NextStream, Payload, PayloadStream, RequestContext, SpecOrigin,
};
use futures_util::StreamExt as _;
use protovalidate_buffa::{
    FieldPath as ViolationPath, FieldPathElement, FieldType as ViolationType, Subscript, Validate,
    ValidationError, Violation,
};

use super::writes::{ResourceWrites, WriteMethod};
use crate::{FieldBehavior, FieldBehaviorKind, FieldPath, FieldScope, FieldType, Step};

/// One procedure's message types, as the steps [`FieldBehaviorInterceptor`]
/// applies to it: `inbound` to each message the server receives, `outbound` to
/// each message it sends. Each step decodes the message from the payload as
/// its own type and puts it back.
///
/// Built with [`Procedure::new`] or [`Procedure::clear`]; the fields are
/// public so a caller can apply exactly what the interceptor would, e.g.
/// `(procedure.inbound)(&mut payload)`.
#[derive(Debug, Clone, Copy)]
pub struct Procedure {
    /// The procedure's path, e.g. `/acme.v1.Books/CreateBook`.
    pub path: &'static str,
    /// Applied to each message the server receives.
    pub inbound: fn(&mut Payload) -> Result<(), ConnectError>,
    /// Applied to each message the server sends.
    pub outbound: fn(&mut Payload) -> Result<(), ConnectError>,
}

impl Procedure {
    /// A procedure of any cardinality: [`validate`] on each message received
    /// -- `OUTPUT_ONLY` cleared, what AIP leaves to the server filled in,
    /// `REQUIRED` and protovalidate's rules checked -- and `INPUT_ONLY`
    /// cleared on each message sent.
    #[must_use]
    pub const fn new<Req, Resp>(path: &'static str) -> Self
    where
        Req: buffa::Message
            + JsonSerialize
            + JsonDeserialize
            + Clone
            + FieldBehavior
            + ResourceWrites
            + Validate
            + 'static,
        Resp: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
    {
        Self {
            path,
            inbound: validate::<Req>,
            outbound: clear_input_only::<Resp>,
        }
    }

    /// A procedure whose request is a well-known type, such as
    /// `google.protobuf.Empty`: `OUTPUT_ONLY` cleared on each message received
    /// and `INPUT_ONLY` on each message sent. Such a request writes no
    /// resource and carries no protovalidate rules, so clearing is all
    /// [`validate`] would do -- and protovalidate-buffa does not implement
    /// `Validate` for it.
    #[must_use]
    pub const fn clear<Req, Resp>(path: &'static str) -> Self
    where
        Req: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
        Resp: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
    {
        Self {
            path,
            inbound: clear_output_only::<Req>,
            outbound: clear_input_only::<Resp>,
        }
    }
}

/// Applies AIP field behavior on a Connect server, as aip-go's
/// `NewFieldBehaviorInterceptor` does: the full policy on every message of
/// every RPC, unary or streamed (see the [module documentation](self)).
///
/// It edits the messages it passes on, the outbound ones included: a handler
/// that keeps a message it sent sees its `INPUT_ONLY` fields cleared. Calls a
/// client makes, and procedures `procedures` does not list, pass through
/// untouched.
///
/// ```ignore
/// let service = ConnectRpcService::new(router)
///     .with_interceptor(aip::FieldBehaviorInterceptor::new(sdk::v1::PROCEDURES));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FieldBehaviorInterceptor {
    procedures: &'static [Procedure],
}

impl FieldBehaviorInterceptor {
    /// An interceptor for the procedures `procedures` names.
    #[must_use]
    pub const fn new(procedures: &'static [Procedure]) -> Self {
        Self { procedures }
    }

    /// The procedure a server is serving in `ctx`, if `procedures` lists it.
    fn procedure(&self, ctx: &RequestContext) -> Option<&'static Procedure> {
        let spec = ctx.spec()?;
        if spec.origin != SpecOrigin::Server {
            return None;
        }
        self.procedures
            .iter()
            .find(|procedure| procedure.path == spec.procedure)
    }
}

#[connectrpc::async_trait]
impl Interceptor for FieldBehaviorInterceptor {
    async fn intercept_unary(
        &self,
        mut request: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        let Some(procedure) = self.procedure(&request.ctx) else {
            return next.run(request).await;
        };
        (procedure.inbound)(&mut request.payload)?;
        let mut response = next.run(request).await?;
        (procedure.outbound)(&mut response.body)?;
        Ok(response)
    }

    async fn intercept_streaming(
        &self,
        request: StreamRequest,
        inbound: PayloadStream,
        next: NextStream<'_>,
    ) -> Result<StreamResponse, ConnectError> {
        let Some(procedure) = self.procedure(&request.ctx) else {
            return next.run(request, inbound).await;
        };
        let (on_inbound, on_outbound) = (procedure.inbound, procedure.outbound);
        let inbound: PayloadStream = Box::pin(inbound.map(move |item| apply(on_inbound, item)));
        let response = next.run(request, inbound).await?;
        Ok(response.map_body(move |body| -> PayloadStream {
            Box::pin(body.map(move |item| apply(on_outbound, item)))
        }))
    }
}

/// `step` applied to one streamed message, an error passed on as it is.
pub(super) fn apply(
    step: impl FnOnce(&mut Payload) -> Result<(), ConnectError>,
    item: Result<Payload, ConnectError>,
) -> Result<Payload, ConnectError> {
    let mut payload = item?;
    step(&mut payload)?;
    Ok(payload)
}

/// [`Procedure::clear`]'s inbound step: `OUTPUT_ONLY` cleared on a
/// payload holding an `M`.
fn clear_output_only<M>(payload: &mut Payload) -> Result<(), ConnectError>
where
    M: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
{
    clear::<M>(payload, FieldBehaviorKind::OutputOnly)
}

/// Every [`Procedure`]'s outbound step: `INPUT_ONLY` cleared on a payload
/// holding an `M`.
fn clear_input_only<M>(payload: &mut Payload) -> Result<(), ConnectError>
where
    M: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
{
    clear::<M>(payload, FieldBehaviorKind::InputOnly)
}

/// `payload` read as an `M`. A body that does not decode is answered with a
/// fixed message, the decoder's text kept as the error's source for the log, as
/// aip-go does: the first read of a request is the one that fails, and it is
/// this interceptor's, so an `InvalidArgument` from it is that failure and no
/// other. Any other error, such as a payload that is not an `M`, is passed on.
pub(super) fn decode<M>(payload: &Payload) -> Result<&M, ConnectError>
where
    M: buffa::Message + JsonSerialize + JsonDeserialize + 'static,
{
    payload.message::<M>().map_err(|error| {
        if error.code == ErrorCode::InvalidArgument {
            // Fixed, since connectrpc's own text carries the decoder's, which for
            // JSON quotes the value that did not parse.
            ConnectError::invalid_argument(
                "The request could not be processed due to malformed data",
            )
            .with_source(error)
        } else {
            error
        }
    })
}

fn clear<M>(payload: &mut Payload, kind: FieldBehaviorKind) -> Result<(), ConnectError>
where
    M: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
{
    let mut message = decode::<M>(payload)?.clone();
    message.clear_fields(kind);
    payload.set_message(message);
    Ok(())
}

/// What [`FieldBehaviorInterceptor`] does to each request message before its
/// handler sees it -- [`Procedure::new`]'s inbound step -- to the `M` `payload`
/// holds, for a caller to apply exactly that, such as a handler test:
///
/// 1. clears `OUTPUT_ONLY` fields, at any depth, so a client cannot write
///    server-owned values;
/// 2. fills in what AIP leaves to the server on each resource the request
///    writes ([`ResourceWrites::for_each_write`]): a create's omitted
///    `<resource>_id` becomes a UUIDv7, so server-assigned ids sort by
///    creation time (AIP-133; the name is the handler's to build, see the
///    [module documentation](self)); an update's omitted `update_mask`
///    becomes the mask the resource implies (AIP-134), and an explicit one
///    loses the paths of its `OUTPUT_ONLY` fields, which AIP-203 has a server
///    ignore rather than reject ([`Resource::clear_field_mask`](crate::Resource::clear_field_mask)) -- a
///    mask left empty by that writes nothing, and is not implied instead;
/// 3. validates it: protovalidate's rules, and a `required` violation for each
///    `REQUIRED` field of a resource the method leaves unset -- every one on a
///    create, only those the mask names on an update (AIP-203) -- shaped as
///    protovalidate reports its own `required` rule, field numbers and types
///    included.
///
/// # Errors
///
/// Every violation at once: `InvalidArgument` with the fixed message "The
/// request contains invalid parameters", as aip-go answers, and the violations
/// in the details -- protovalidate-buffa's `buf.validate.Violations`, and
/// AIP-193's `google.rpc.BadRequest` beside it, a field violation per
/// violation, its field path and message, within the same 4 KiB. The message
/// names no field: AIP-193 keeps what varies in the details. `Internal` if a
/// rule failed to compile or run.
///
/// Also if `payload` does not hold an `M`.
pub fn validate<M>(payload: &mut Payload) -> Result<(), ConnectError>
where
    M: buffa::Message
        + JsonSerialize
        + JsonDeserialize
        + Clone
        + FieldBehavior
        + ResourceWrites
        + Validate
        + 'static,
{
    let mut request = decode::<M>(payload)?.clone();
    request.clear_fields(FieldBehaviorKind::OutputOnly);
    imply(&mut request, &mut || uuid::Uuid::now_v7().to_string());

    // Step 3: protovalidate's violations, then a `required` one per path
    // `required_paths` finds. A rule that failed to compile or run is answered
    // as it is, which `into_connect_error` makes an `Internal`.
    let mut error = match request.validate() {
        Ok(()) => ValidationError::default(),
        Err(error) if error.compile_error.is_some() || error.runtime_error.is_some() => {
            return Err(connect_error(error));
        }
        Err(error) => error,
    };
    error
        .violations
        .extend(required_paths(&mut request).iter().map(required_violation));
    if !error.violations.is_empty() {
        return Err(connect_error(error));
    }

    payload.set_message(request);
    Ok(())
}

/// `error` as [`validate`] answers it: protovalidate-buffa's rendering, and for
/// an `InvalidArgument` the fixed message and a [`bad_request`] detail.
fn connect_error(error: ValidationError) -> ConnectError {
    let details = bad_request(&error.violations);
    let mut error = error.into_connect_error();
    if error.code == ErrorCode::InvalidArgument {
        error.message = Some("The request contains invalid parameters".to_owned());
        if !details.field_violations.is_empty() {
            error = error.with_detail(ErrorDetail::from_message(BadRequest::FULL_NAME, &details));
        }
    }
    error
}

/// The most a [`bad_request`] detail takes, as protovalidate-buffa allows its
/// own `buf.validate.Violations`: a request with a great many violations is not
/// answered with an error as large.
const MAX_DETAIL_BYTES: usize = 4096;

/// `violations` as AIP-193's `google.rpc.BadRequest`, the standard detail for
/// the fields a request got wrong: a field violation per violation, its field
/// path and message, until the next would take the detail past
/// [`MAX_DETAIL_BYTES`].
fn bad_request(violations: &[Violation]) -> BadRequest {
    let mut request = BadRequest::default();
    for violation in violations {
        request.field_violations.push(FieldViolation {
            field: violation.field.to_string(),
            description: violation.message.to_string(),
            ..Default::default()
        });
        if request.encoded_len() as usize > MAX_DETAIL_BYTES {
            request.field_violations.pop();
            break;
        }
    }
    request
}

/// Step 2 of [`validate`], with `new_id` minting a create's omitted id.
fn imply<M: ResourceWrites + ?Sized>(request: &mut M, new_id: &mut dyn FnMut() -> String) {
    request.for_each_write(&mut |write| match write.method {
        WriteMethod::Create { id: Some(id) } => {
            if id.is_empty() {
                *id = new_id();
            }
        }
        WriteMethod::Create { id: None } => {}
        WriteMethod::Update { mask } => {
            if mask.paths.is_empty() {
                *mask = write.resource.implied_field_mask();
            } else {
                write
                    .resource
                    .clear_field_mask(FieldBehaviorKind::OutputOnly, mask);
            }
        }
    });
}

/// The `REQUIRED` fields the method leaves unset, on each resource `request`
/// writes, as paths from `request`: every one on a create, only those the
/// mask names on an update.
fn required_paths<M: ResourceWrites + ?Sized>(request: &mut M) -> Vec<FieldPath> {
    let mut paths = Vec::new();
    request.for_each_write(&mut |write| {
        let scope = match &write.method {
            WriteMethod::Create { .. } => FieldScope::All,
            WriteMethod::Update { mask } => FieldScope::Mask(&mask.paths),
        };
        if let Err(error) = write
            .resource
            .validate_fields(FieldBehaviorKind::Required, scope)
        {
            paths.extend(error.prefixed(&write.path).paths);
        }
    });
    paths
}

/// The violation protovalidate reports for `(buf.validate.field).required` --
/// rule id `required`, message `value is required` -- for the field at
/// `path`, each element with the field's name, number and type.
fn required_violation(path: &FieldPath) -> Violation {
    let mut elements: Vec<FieldPathElement> = Vec::new();
    for step in path.steps() {
        match step {
            Step::Field { name, number, ty } => elements.push(FieldPathElement {
                field_number: i32::try_from(number).ok(),
                field_name: Some(name.into()),
                field_type: Some(ty.into()),
                ..Default::default()
            }),
            Step::Index(index) => {
                if let Some(element) = elements.last_mut() {
                    element.subscript = Some(Subscript::Index(index as u64));
                }
            }
        }
    }
    Violation {
        field: ViolationPath { elements },
        rule: ViolationPath {
            elements: vec![FieldPathElement {
                field_number: Some(25),
                field_name: Some("required".into()),
                field_type: Some(ViolationType::Bool),
                ..Default::default()
            }],
        },
        rule_id: "required".into(),
        message: "value is required".into(),
        for_key: false,
    }
}

/// `ty` as protovalidate names it.
impl From<FieldType> for ViolationType {
    fn from(ty: FieldType) -> Self {
        match ty {
            FieldType::Double => Self::Double,
            FieldType::Float => Self::Float,
            FieldType::Int64 => Self::Int64,
            FieldType::Uint64 => Self::Uint64,
            FieldType::Int32 => Self::Int32,
            FieldType::Fixed64 => Self::Fixed64,
            FieldType::Fixed32 => Self::Fixed32,
            FieldType::Bool => Self::Bool,
            FieldType::String => Self::String,
            FieldType::Group => Self::Group,
            FieldType::Message => Self::Message,
            FieldType::Bytes => Self::Bytes,
            FieldType::Uint32 => Self::Uint32,
            FieldType::Enum => Self::Enum,
            FieldType::Sfixed32 => Self::Sfixed32,
            FieldType::Sfixed64 => Self::Sfixed64,
            FieldType::Sint32 => Self::Sint32,
            FieldType::Sint64 => Self::Sint64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buffa_types::FieldMask;

    use crate::{DynResource, FieldBehaviorError, Resource, ResourceWrite};

    /// A resource with one `REQUIRED` field, `title` (3), an `OUTPUT_ONLY`
    /// `create_time` (4) and an `INPUT_ONLY` `secret`.
    #[derive(Default)]
    struct Book {
        name: String,
        title: String,
        create_time: String,
        secret: String,
    }

    impl FieldBehavior for Book {
        fn clear_fields(&mut self, kind: FieldBehaviorKind) {
            if kind == FieldBehaviorKind::OutputOnly {
                self.create_time.clear();
            }
            if kind == FieldBehaviorKind::InputOnly {
                self.secret.clear();
            }
        }

        fn validate_fields(
            &self,
            kind: FieldBehaviorKind,
            scope: FieldScope<'_>,
        ) -> Result<(), FieldBehaviorError> {
            let mut paths = Vec::new();
            if kind == FieldBehaviorKind::Required
                && scope.contains("title")
                && self.title.is_empty()
            {
                paths.push(FieldPath::field("title", 3, FieldType::String));
            }
            if kind == FieldBehaviorKind::OutputOnly
                && scope.contains("create_time")
                && self.create_time.is_empty()
            {
                paths.push(FieldPath::field("create_time", 4, FieldType::String));
            }
            FieldBehaviorError::check(kind, paths)
        }
    }

    impl Resource for Book {
        fn implied_field_mask(&self) -> FieldMask {
            let mut paths = Vec::new();
            if !self.title.is_empty() {
                paths.push("title".to_owned());
            }
            FieldMask {
                paths,
                ..Default::default()
            }
        }
    }

    fn book_path() -> FieldPath {
        FieldPath::field("book", 2, FieldType::Message)
    }

    #[derive(Default)]
    struct CreateBook {
        book_id: String,
        book: Option<Book>,
    }

    impl FieldBehavior for CreateBook {
        fn clear_fields(&mut self, kind: FieldBehaviorKind) {
            if let Some(book) = &mut self.book {
                book.clear_fields(kind);
            }
        }
    }

    impl ResourceWrites for CreateBook {
        fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
            if let Some(book) = &mut self.book {
                f(ResourceWrite {
                    path: book_path(),
                    resource: book,
                    method: WriteMethod::Create {
                        id: Some(&mut self.book_id),
                    },
                });
            }
        }
    }

    impl Validate for CreateBook {
        fn validate(&self) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct UpdateBook {
        book: Option<Book>,
        update_mask: Option<FieldMask>,
    }

    impl FieldBehavior for UpdateBook {}

    impl ResourceWrites for UpdateBook {
        fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
            if let Some(book) = &mut self.book {
                f(ResourceWrite {
                    path: book_path(),
                    resource: book,
                    method: WriteMethod::Update {
                        mask: self.update_mask.get_or_insert_default(),
                    },
                });
            }
        }
    }

    impl Validate for UpdateBook {
        fn validate(&self) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct BatchCreateBooks {
        requests: Vec<CreateBook>,
    }

    impl FieldBehavior for BatchCreateBooks {}

    impl ResourceWrites for BatchCreateBooks {
        fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
            for (index, request) in self.requests.iter_mut().enumerate() {
                request.for_each_write(&mut |write| {
                    f(write.prefixed(&FieldPath::index("requests", 1, FieldType::Message, index)));
                });
            }
        }
    }

    impl Validate for BatchCreateBooks {
        fn validate(&self) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    fn mask(paths: &[&str]) -> Option<FieldMask> {
        Some(FieldMask {
            paths: paths.iter().map(|&path| path.to_owned()).collect(),
            ..Default::default()
        })
    }

    #[test]
    fn a_create_mints_an_omitted_id_and_leaves_the_name_alone() {
        let mut request = CreateBook {
            book: Some(Book::default()),
            ..Default::default()
        };
        imply(&mut request, &mut || "minted".to_owned());
        assert_eq!(request.book_id, "minted");
        assert_eq!(request.book.expect("the book stays").name, "");
    }

    #[test]
    fn a_create_keeps_the_id_the_client_sent() {
        let mut request = CreateBook {
            book_id: "sent".to_owned(),
            book: Some(Book::default()),
        };
        imply(&mut request, &mut || unreachable!("an id was sent"));
        assert_eq!(request.book_id, "sent");
    }

    #[test]
    fn an_update_implies_an_omitted_mask_and_keeps_a_sent_one() {
        let mut request = UpdateBook {
            book: Some(Book {
                title: "Dune".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        };
        imply(&mut request, &mut String::new);
        assert_eq!(
            request.update_mask.map(|mask| mask.paths),
            Some(vec!["title".to_owned()])
        );

        let mut sent = UpdateBook {
            book: Some(Book::default()),
            update_mask: mask(&["other"]),
        };
        imply(&mut sent, &mut String::new);
        assert_eq!(
            sent.update_mask.map(|mask| mask.paths),
            Some(vec!["other".to_owned()])
        );
    }

    #[test]
    fn an_update_ignores_the_output_only_fields_an_explicit_mask_names() {
        let mut request = UpdateBook {
            book: Some(Book {
                title: "Dune".to_owned(),
                ..Default::default()
            }),
            update_mask: mask(&["create_time", "title", "other"]),
        };
        imply(&mut request, &mut String::new);
        assert_eq!(
            request.update_mask.map(|mask| mask.paths),
            Some(vec!["title".to_owned(), "other".to_owned()])
        );
    }

    #[test]
    fn an_update_leaves_a_mask_of_only_output_only_fields_empty_rather_than_implied() {
        let mut request = UpdateBook {
            book: Some(Book {
                title: "Dune".to_owned(),
                ..Default::default()
            }),
            update_mask: mask(&["create_time"]),
        };
        imply(&mut request, &mut String::new);
        assert_eq!(request.update_mask.map(|mask| mask.paths), Some(Vec::new()));
    }

    #[test]
    fn a_create_requires_every_required_field() {
        let mut request = CreateBook {
            book: Some(Book::default()),
            ..Default::default()
        };
        assert_eq!(
            required_paths(&mut request),
            [book_path().join(FieldPath::field("title", 3, FieldType::String))]
        );
    }

    #[test]
    fn an_update_requires_only_the_fields_its_mask_names() {
        let mut other = UpdateBook {
            book: Some(Book::default()),
            update_mask: mask(&["other"]),
        };
        assert!(required_paths(&mut other).is_empty());

        let mut named = UpdateBook {
            book: Some(Book::default()),
            update_mask: mask(&["title"]),
        };
        assert_eq!(required_paths(&mut named).len(), 1);
    }

    #[test]
    fn a_batch_reports_each_entry_under_its_index() {
        let mut request = BatchCreateBooks {
            requests: vec![
                CreateBook {
                    book: Some(Book {
                        title: "Dune".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                CreateBook {
                    book: Some(Book::default()),
                    ..Default::default()
                },
            ],
        };
        let paths = required_paths(&mut request);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].to_string(), "requests[1].book.title");
    }

    #[test]
    fn a_bad_request_names_each_violation_s_field_and_message() {
        let title = FieldPath::index("requests", 1, FieldType::Message, 2)
            .join(book_path())
            .join(FieldPath::field("title", 3, FieldType::String));
        let request = bad_request(&[required_violation(&title)]);
        let violations: Vec<_> = request
            .field_violations
            .iter()
            .map(|violation| (violation.field.as_str(), violation.description.as_str()))
            .collect();
        assert_eq!(
            violations,
            [("requests[2].book.title", "value is required")]
        );
    }

    #[test]
    fn a_bad_request_stops_short_of_the_detail_budget() {
        let title = FieldPath::field("title", 3, FieldType::String);
        let violations = vec![required_violation(&title); 1000];
        let request = bad_request(&violations);
        assert!(!request.field_violations.is_empty());
        assert!(request.field_violations.len() < violations.len());
        assert!(request.encoded_len() as usize <= MAX_DETAIL_BYTES);
    }

    #[test]
    fn a_required_violation_reads_like_protovalidate_s_own() {
        let path = FieldPath::index("requests", 1, FieldType::Message, 2)
            .join(book_path())
            .join(FieldPath::field("title", 3, FieldType::String));
        let violation = required_violation(&path);
        assert_eq!(violation.rule_id, "required");
        assert_eq!(violation.message, "value is required");
        assert_eq!(violation.field.to_string(), "requests[2].book.title");
        let elements: Vec<_> = violation
            .field
            .elements
            .iter()
            .map(|element| {
                (
                    element.field_name.as_deref().unwrap_or_default().to_owned(),
                    element.field_number,
                    element.field_type,
                )
            })
            .collect();
        assert_eq!(
            elements,
            [
                ("requests".to_owned(), Some(1), Some(ViolationType::Message)),
                ("book".to_owned(), Some(2), Some(ViolationType::Message)),
                ("title".to_owned(), Some(3), Some(ViolationType::String)),
            ]
        );
        assert!(matches!(
            violation.field.elements[0].subscript,
            Some(Subscript::Index(2))
        ));
    }

    #[test]
    fn a_field_type_converts_to_protovalidate_s() {
        assert_eq!(
            ViolationType::from(FieldType::Sint64),
            ViolationType::Sint64
        );
        assert_eq!(ViolationType::from(FieldType::Group), ViolationType::Group);
    }

    #[test]
    fn every_resource_is_a_dyn_resource() {
        let book = Book {
            title: "Dune".to_owned(),
            ..Default::default()
        };
        let resource: &dyn DynResource = &book;
        assert_eq!(resource.implied_field_mask().paths, ["title"]);
    }
}

#[cfg(test)]
mod interceptor_tests {
    //! [`FieldBehaviorInterceptor`] on the path a request takes through a
    //! connectrpc server -- HTTP in, the router, the interceptor chain, the
    //! handler, and back -- as aip-go's `interceptor_test.go` drives its own,
    //! rather than through [`crate::request::interceptor::validate`] alone.
    //!
    //! The messages are hand-written `buffa::Message`s, so the crate needs no
    //! codegen to test itself: a `Book` resource and the requests that write it,
    //! each with the `FieldBehavior`, `Resource`, `ResourceWrites` and `Validate`
    //! impls protoc-gen-rust-aip and protoc-gen-protovalidate-buffa would emit.
    use std::sync::{Arc, Mutex};

    use crate::{
        FieldBehavior, FieldBehaviorError, FieldBehaviorInterceptor, FieldBehaviorKind, FieldPath,
        FieldScope, FieldType, Procedure, Resource, ResourceWrite, ResourceWrites, WriteMethod,
    };
    use buffa::bytes::{Buf, Bytes};
    use buffa::encoding::{Tag, WireType, encode_varint, skip_field, varint_len};
    use buffa::{DecodeContext, DecodeError, EncodeSink, Message, SizeCache};
    use buffa_types::FieldMask;
    use connectrpc::handler::{handler_fn, streaming_handler_fn};
    use connectrpc::interceptor::{StreamRequest, StreamResponse, UnaryRequest, UnaryResponse};
    use connectrpc::{
        ConnectError, ConnectRpcService, ErrorCode, Interceptor, Next, NextStream, PayloadStream,
        Router, Spec, StreamType,
    };
    use http_body_util::{BodyExt as _, Full};
    use protovalidate_buffa::{Validate, ValidationError};
    use tower::ServiceExt as _;

    // -- Messages ---------------------------------------------------------------

    /// A field of a hand-written message, and how it goes on and off the wire.
    /// Every field here is length-delimited, with a number below 16, so its tag
    /// is one byte.
    trait Field {
        fn size(&self) -> u32;
        fn put(&self, number: u32, buf: &mut impl EncodeSink);
        fn merge(&mut self, buf: &mut impl Buf, ctx: DecodeContext<'_>) -> Result<(), DecodeError>;
    }

    fn delimited(len: usize) -> u32 {
        (1 + varint_len(len as u64) + len) as u32
    }

    fn put_delimited(number: u32, bytes: &[u8], buf: &mut impl EncodeSink) {
        Tag::new(number, WireType::LengthDelimited).encode(buf);
        encode_varint(bytes.len() as u64, buf);
        buf.put_slice(bytes);
    }

    impl Field for String {
        fn size(&self) -> u32 {
            if self.is_empty() {
                0
            } else {
                delimited(self.len())
            }
        }

        fn put(&self, number: u32, buf: &mut impl EncodeSink) {
            if !self.is_empty() {
                put_delimited(number, self.as_bytes(), buf);
            }
        }

        fn merge(&mut self, buf: &mut impl Buf, _: DecodeContext<'_>) -> Result<(), DecodeError> {
            buffa::types::merge_string(self, buf)
        }
    }

    impl<M: Message> Field for Option<M> {
        fn size(&self) -> u32 {
            self.as_ref()
                .map_or(0, |message| delimited(message.encoded_len() as usize))
        }

        fn put(&self, number: u32, buf: &mut impl EncodeSink) {
            if let Some(message) = self {
                put_delimited(number, &message.encode_to_vec(), buf);
            }
        }

        fn merge(&mut self, buf: &mut impl Buf, ctx: DecodeContext<'_>) -> Result<(), DecodeError> {
            self.get_or_insert_default()
                .merge_length_delimited(buf, ctx)
        }
    }

    impl<M: Message> Field for Vec<M> {
        fn size(&self) -> u32 {
            self.iter()
                .map(|message| delimited(message.encoded_len() as usize))
                .sum()
        }

        fn put(&self, number: u32, buf: &mut impl EncodeSink) {
            for message in self {
                put_delimited(number, &message.encode_to_vec(), buf);
            }
        }

        fn merge(&mut self, buf: &mut impl Buf, ctx: DecodeContext<'_>) -> Result<(), DecodeError> {
            let mut message = M::default();
            message.merge_length_delimited(buf, ctx)?;
            self.push(message);
            Ok(())
        }
    }

    /// A struct of [`Field`]s, numbered, and its `buffa::Message` impl.
    macro_rules! message {
        ($name:ident { $($number:literal => $field:ident: $ty:ty),* $(,)? }) => {
            #[derive(Debug, Clone, Default, PartialEq)]
            struct $name {
                $($field: $ty,)*
            }

            buffa::impl_default_instance!($name);

            impl Message for $name {
                fn compute_size(&self, _: &mut SizeCache) -> u32 {
                    0 $(+ self.$field.size())*
                }

                fn write_to(&self, _: &mut SizeCache, buf: &mut impl EncodeSink) {
                    $(self.$field.put($number, buf);)*
                }

                fn merge_field(
                    &mut self,
                    tag: Tag,
                    buf: &mut impl Buf,
                    ctx: DecodeContext<'_>,
                ) -> Result<(), DecodeError> {
                    match tag.field_number() {
                        $($number => self.$field.merge(buf, ctx),)*
                        _ => skip_field(tag, buf),
                    }
                }

                fn clear(&mut self) {
                    *self = Self::default();
                }
            }
        };
    }

    message!(Book {
        1 => name: String,
        // REQUIRED
        2 => title: String,
        // OUTPUT_ONLY
        3 => create_time: String,
        // INPUT_ONLY
        4 => secret: String,
    });

    message!(CreateBookRequest {
        1 => book_id: String,
        2 => book: Option<Book>,
    });

    message!(BatchCreateBooksRequest {
        1 => requests: Vec<CreateBookRequest>,
    });

    message!(BatchCreateBooksResponse {
        1 => books: Vec<Book>,
    });

    message!(GetBookRequest {
        1 => name: String,
    });

    // -- What the plugins would generate ----------------------------------------

    impl FieldBehavior for Book {
        fn clear_fields(&mut self, kind: FieldBehaviorKind) {
            match kind {
                FieldBehaviorKind::OutputOnly => self.create_time.clear(),
                FieldBehaviorKind::InputOnly => self.secret.clear(),
                _ => {}
            }
        }

        fn validate_fields(
            &self,
            kind: FieldBehaviorKind,
            scope: FieldScope<'_>,
        ) -> Result<(), FieldBehaviorError> {
            let mut paths = Vec::new();
            if kind == FieldBehaviorKind::Required
                && scope.contains("title")
                && self.title.is_empty()
            {
                paths.push(FieldPath::field("title", 2, FieldType::String));
            }
            FieldBehaviorError::check(kind, paths)
        }
    }

    impl Resource for Book {
        fn implied_field_mask(&self) -> FieldMask {
            FieldMask {
                paths: if self.title.is_empty() {
                    Vec::new()
                } else {
                    vec!["title".to_owned()]
                },
                ..Default::default()
            }
        }
    }

    impl FieldBehavior for CreateBookRequest {
        fn clear_fields(&mut self, kind: FieldBehaviorKind) {
            if let Some(book) = &mut self.book {
                book.clear_fields(kind);
            }
        }
    }

    impl ResourceWrites for CreateBookRequest {
        fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
            if let Some(book) = &mut self.book {
                f(ResourceWrite {
                    path: FieldPath::field("book", 2, FieldType::Message),
                    resource: book,
                    method: WriteMethod::Create {
                        id: Some(&mut self.book_id),
                    },
                });
            }
        }
    }

    impl FieldBehavior for BatchCreateBooksRequest {
        fn clear_fields(&mut self, kind: FieldBehaviorKind) {
            for request in &mut self.requests {
                request.clear_fields(kind);
            }
        }
    }

    impl ResourceWrites for BatchCreateBooksRequest {
        fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
            for (index, request) in self.requests.iter_mut().enumerate() {
                request.for_each_write(&mut |write| {
                    f(write.prefixed(&FieldPath::index("requests", 1, FieldType::Message, index)));
                });
            }
        }
    }

    impl FieldBehavior for BatchCreateBooksResponse {
        fn clear_fields(&mut self, kind: FieldBehaviorKind) {
            for book in &mut self.books {
                book.clear_fields(kind);
            }
        }
    }

    impl FieldBehavior for GetBookRequest {}
    impl ResourceWrites for GetBookRequest {}
    /// A resource on its own writes nothing: streamed, it is only cleared.
    impl ResourceWrites for Book {}

    /// No protovalidate rules on these.
    macro_rules! valid {
        ($($ty:ty),*) => {
            $(impl Validate for $ty {
                fn validate(&self) -> Result<(), ValidationError> {
                    Ok(())
                }
            })*
        };
    }

    valid!(Book, CreateBookRequest, BatchCreateBooksRequest);

    /// A `GetBookRequest` whose rules fail to compile unless it names a book:
    /// how protovalidate-buffa reports a rule it cannot run.
    impl Validate for GetBookRequest {
        fn validate(&self) -> Result<(), ValidationError> {
            if !self.name.is_empty() {
                return Ok(());
            }
            Err(ValidationError {
                compile_error: Some("no such function: is_book".to_owned()),
                ..Default::default()
            })
        }
    }

    const CREATE: &str = "/example.v1.Books/CreateBook";
    const BATCH_CREATE: &str = "/example.v1.Books/BatchCreateBooks";
    const GET: &str = "/example.v1.Books/GetBook";
    const WATCH: &str = "/example.v1.Books/WatchBooks";
    /// A stream of standard-method requests.
    const IMPORT: &str = "/example.v1.Books/ImportBooks";
    /// In the table, but served as a client would call it.
    const CLIENT: &str = "/example.v1.Books/ClientCreateBook";
    /// Served, but not in the table.
    const UNLISTED: &str = "/example.v1.Books/UnlistedCreateBook";

    static PROCEDURES: &[Procedure] = &[
        Procedure::new::<CreateBookRequest, Book>(CREATE),
        Procedure::new::<BatchCreateBooksRequest, BatchCreateBooksResponse>(BATCH_CREATE),
        Procedure::new::<GetBookRequest, Book>(GET),
        Procedure::new::<Book, Book>(WATCH),
        Procedure::new::<CreateBookRequest, Book>(IMPORT),
        Procedure::new::<CreateBookRequest, Book>(CLIENT),
    ];

    // -- The server -------------------------------------------------------------

    /// What the handlers saw, and what the chain answered with, for a test to
    /// read back.
    #[derive(Default)]
    struct Seen {
        /// Each request a handler was called with, as it received it.
        requests: Vec<String>,
        /// The error the interceptor chain answered a unary call with -- code,
        /// message and source, debug-formatted as a log would show it -- read by
        /// an interceptor outside it, since the source never reaches the wire.
        error: Option<(ErrorCode, String, String)>,
    }

    /// Outermost: records the error the rest of the chain answered with.
    struct Record(Arc<Mutex<Seen>>);

    #[connectrpc::async_trait]
    impl Interceptor for Record {
        async fn intercept_unary(
            &self,
            request: UnaryRequest,
            next: Next<'_>,
        ) -> Result<UnaryResponse, ConnectError> {
            let response = next.run(request).await;
            if let Err(error) = &response {
                let source = std::error::Error::source(error).map(|source| format!("{source:?}"));
                self.0.lock().unwrap().error = Some((
                    error.code,
                    error.message.clone().unwrap_or_default(),
                    source.unwrap_or_default(),
                ));
            }
            response
        }

        async fn intercept_streaming(
            &self,
            request: StreamRequest,
            inbound: PayloadStream,
            next: NextStream<'_>,
        ) -> Result<StreamResponse, ConnectError> {
            next.run(request, inbound).await
        }
    }

    /// A book as a handler returns it: with the server-owned `create_time` and
    /// the `INPUT_ONLY` `secret` still set, for the interceptor to clear.
    fn returned(book: Option<Book>) -> Book {
        Book {
            create_time: "2026-01-01T00:00:00Z".to_owned(),
            secret: "kept by the handler".to_owned(),
            ..book.unwrap_or_default()
        }
    }

    /// A service routing the procedures above, through `FieldBehaviorInterceptor`.
    fn service(seen: &Arc<Mutex<Seen>>) -> ConnectRpcService {
        let create = {
            let seen = Arc::clone(seen);
            move |_, request: CreateBookRequest| {
                seen.lock().unwrap().requests.push(format!("{request:?}"));
                async move { Ok(returned(request.book).into()) }
            }
        };
        let batch = {
            let seen = Arc::clone(seen);
            move |_, request: BatchCreateBooksRequest| {
                seen.lock().unwrap().requests.push(format!("{request:?}"));
                let books = request
                    .requests
                    .into_iter()
                    .map(|entry| returned(entry.book))
                    .collect();
                async move { Ok(BatchCreateBooksResponse { books }.into()) }
            }
        };
        let get = {
            let seen = Arc::clone(seen);
            move |_, request: GetBookRequest| {
                seen.lock().unwrap().requests.push(format!("{request:?}"));
                async move { Ok(returned(None).into()) }
            }
        };
        let watch = {
            let seen = Arc::clone(seen);
            move |_, request: Book| {
                seen.lock().unwrap().requests.push(format!("{request:?}"));
                let books = vec![
                    Ok(returned(Some(request.clone()))),
                    Ok(returned(Some(request))),
                ];
                let stream: connectrpc::ServiceStream<Book> =
                    Box::pin(futures_util::stream::iter(books));
                async move { Ok(stream.into()) }
            }
        };

        let import = {
            let seen = Arc::clone(seen);
            move |_, request: CreateBookRequest| {
                seen.lock().unwrap().requests.push(format!("{request:?}"));
                let books = vec![Ok(returned(request.book))];
                let stream: connectrpc::ServiceStream<Book> =
                    Box::pin(futures_util::stream::iter(books));
                async move { Ok(stream.into()) }
            }
        };

        let router = Router::new()
            .route("example.v1.Books", "CreateBook", handler_fn(create.clone()))
            .route("example.v1.Books", "BatchCreateBooks", handler_fn(batch))
            .route("example.v1.Books", "GetBook", handler_fn(get))
            .route_server_stream(
                "example.v1.Books",
                "WatchBooks",
                streaming_handler_fn(watch),
            )
            .route_server_stream(
                "example.v1.Books",
                "ImportBooks",
                streaming_handler_fn(import),
            )
            .route(
                "example.v1.Books",
                "ClientCreateBook",
                handler_fn(create.clone()),
            )
            .route("example.v1.Books", "UnlistedCreateBook", handler_fn(create))
            .with_spec(Spec::server(CREATE, StreamType::Unary))
            .with_spec(Spec::server(BATCH_CREATE, StreamType::Unary))
            .with_spec(Spec::server(GET, StreamType::Unary))
            .with_spec(Spec::server(WATCH, StreamType::ServerStream))
            .with_spec(Spec::server(IMPORT, StreamType::ServerStream))
            .with_spec(Spec::client(CLIENT, StreamType::Unary))
            .with_spec(Spec::server(UNLISTED, StreamType::Unary));

        ConnectRpcService::new(router)
            .with_interceptor(Record(Arc::clone(seen)))
            .with_interceptor(FieldBehaviorInterceptor::new(PROCEDURES))
    }

    /// Calls the unary `procedure` with `body`, over the Connect protocol: the
    /// response message, or `None` when the call failed.
    async fn unary<Res: Message>(
        service: &ConnectRpcService,
        procedure: &str,
        body: Vec<u8>,
    ) -> Option<Res> {
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
        ok.then(|| Res::decode_from_slice(&body).expect("a response message"))
    }

    /// Calls the server-streaming `procedure` with `request`, over the Connect
    /// protocol: each message streamed back, and the end-of-stream JSON, which
    /// carries the error a failed stream ends with.
    async fn server_stream(
        service: &ConnectRpcService,
        procedure: &str,
        request: &impl Message,
    ) -> (Vec<Book>, String) {
        let message = request.encode_to_vec();
        let mut body = vec![0];
        body.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
        body.extend_from_slice(&message);
        let request = http::Request::post(procedure)
            .header("content-type", "application/connect+proto")
            .body(Full::new(Bytes::from(body)))
            .expect("a request");
        let response = service.clone().oneshot(request).await.expect("infallible");
        let mut body = response
            .into_body()
            .collect()
            .await
            .expect("a body")
            .to_bytes();

        // Each envelope is a flags byte and a big-endian length; the end-of-stream
        // one, flagged 0x02, carries JSON rather than a message.
        let mut messages = Vec::new();
        let mut end = String::new();
        while body.remaining() >= 5 {
            let flags = body.get_u8();
            let len = body.get_u32() as usize;
            let frame = body.split_to(len);
            if flags & 0x02 == 0 {
                messages.push(Book::decode_from_slice(&frame).expect("a streamed message"));
            } else {
                end = String::from_utf8_lossy(&frame).into_owned();
            }
        }
        (messages, end)
    }

    fn book(title: &str) -> Book {
        Book {
            title: title.to_owned(),
            create_time: "sent by the client".to_owned(),
            secret: "sent by the client".to_owned(),
            ..Default::default()
        }
    }

    // -- Specs ------------------------------------------------------------------

    #[tokio::test]
    async fn a_create_is_cleared_and_filled_in_on_the_way_in_and_cleared_on_the_way_out() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let request = CreateBookRequest {
            book: Some(book("Dune")),
            ..Default::default()
        };

        let response: Book = unary(&service(&seen), CREATE, request.encode_to_vec())
            .await
            .expect("created");

        let seen = seen.lock().unwrap();
        let [received] = &seen.requests[..] else {
            panic!("one call: {:?}", seen.requests);
        };
        // OUTPUT_ONLY cleared and an id minted; INPUT_ONLY is the handler's.
        assert!(received.contains(r#"create_time: """#), "{received}");
        assert!(
            received.contains(r#"secret: "sent by the client""#),
            "{received}"
        );
        assert!(!received.contains(r#"book_id: """#), "{received}");
        // The id is a UUIDv7, so server-assigned ids sort by creation time.
        let id = received
            .split(r#"book_id: ""#)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("an id");
        assert_eq!(
            uuid::Uuid::parse_str(id).expect("a UUID").get_version_num(),
            7,
            "{received}"
        );
        // INPUT_ONLY cleared on the response; OUTPUT_ONLY is the server's to set.
        assert_eq!(response.secret, "");
        assert_eq!(response.create_time, "2026-01-01T00:00:00Z");
    }

    #[tokio::test]
    async fn a_batch_response_has_input_only_cleared_from_every_resource() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let entry = |title: &str| CreateBookRequest {
            book: Some(book(title)),
            ..Default::default()
        };
        let request = BatchCreateBooksRequest {
            requests: vec![entry("Dune"), entry("Emma")],
        };

        let response: BatchCreateBooksResponse =
            unary(&service(&seen), BATCH_CREATE, request.encode_to_vec())
                .await
                .expect("created");

        assert_eq!(response.books.len(), 2);
        assert!(
            response.books.iter().all(|book| book.secret.is_empty()),
            "{response:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_required_field_is_refused_before_the_handler() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let request = CreateBookRequest {
            book: Some(book("")),
            ..Default::default()
        };

        let response: Option<Book> = unary(&service(&seen), CREATE, request.encode_to_vec()).await;

        assert!(response.is_none());
        let seen = seen.lock().unwrap();
        assert!(
            seen.requests.is_empty(),
            "the handler ran: {:?}",
            seen.requests
        );
        let (code, message, _) = seen.error.clone().expect("an error");
        assert_eq!(code, ErrorCode::InvalidArgument);
        // A fixed message, as aip-go answers: the violations are in the details.
        assert_eq!(message, "The request contains invalid parameters");
    }

    #[tokio::test]
    async fn a_request_that_writes_nothing_passes_through_and_its_response_is_cleared() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let request = GetBookRequest {
            name: "books/dune".to_owned(),
        };

        let response: Book = unary(&service(&seen), GET, request.encode_to_vec())
            .await
            .expect("read");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.requests, [format!("{request:?}")]);
        assert_eq!(response.secret, "");
    }

    #[tokio::test]
    async fn a_stream_has_output_only_cleared_inbound_and_input_only_outbound() {
        let seen = Arc::new(Mutex::new(Seen::default()));

        let (responses, _) = server_stream(&service(&seen), WATCH, &book("")).await;

        let seen = seen.lock().unwrap();
        let [received] = &seen.requests[..] else {
            panic!("one call: {:?}", seen.requests);
        };
        // Cleared, and nothing else: a resource streamed on its own writes
        // nothing, so it has no REQUIRED check, and the empty title passes.
        assert!(received.contains(r#"create_time: """#), "{received}");
        assert!(
            received.contains(r#"secret: "sent by the client""#),
            "{received}"
        );
        assert_eq!(responses.len(), 2);
        assert!(
            responses.iter().all(|book| book.secret.is_empty()),
            "{responses:?}"
        );
        assert!(
            responses.iter().all(|book| !book.create_time.is_empty()),
            "{responses:?}"
        );
    }

    #[tokio::test]
    async fn a_streamed_create_is_filled_in_and_validated_like_a_unary_one() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let request = CreateBookRequest {
            book: Some(book("Dune")),
            ..Default::default()
        };

        let (responses, end) = server_stream(&service(&seen), IMPORT, &request).await;

        let seen = seen.lock().unwrap();
        let [received] = &seen.requests[..] else {
            panic!("one call: {:?}", seen.requests);
        };
        // OUTPUT_ONLY cleared and an id minted, as on a unary create.
        assert!(received.contains(r#"create_time: """#), "{received}");
        assert!(!received.contains(r#"book_id: """#), "{received}");
        assert_eq!(responses.len(), 1, "{end}");
        assert_eq!(responses[0].secret, "");
    }

    #[tokio::test]
    async fn a_streamed_create_missing_a_required_field_fails_the_stream() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let request = CreateBookRequest {
            book: Some(book("")),
            ..Default::default()
        };

        let (responses, end) = server_stream(&service(&seen), IMPORT, &request).await;

        assert!(responses.is_empty(), "{responses:?}");
        assert!(
            seen.lock().unwrap().requests.is_empty(),
            "the handler ran on a message that failed validation"
        );
        assert!(end.contains(r#""code":"invalid_argument""#), "{end}");
        assert!(
            end.contains(r#""message":"The request contains invalid parameters""#),
            "{end}"
        );
        // The violations travel as a google.rpc.BadRequest too (AIP-193).
        assert!(end.contains(r#""type":"google.rpc.BadRequest""#), "{end}");
    }

    #[tokio::test]
    async fn a_call_a_client_makes_passes_through_untouched() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        // No title: refused if the interceptor applied.
        let request = CreateBookRequest {
            book: Some(book("")),
            ..Default::default()
        };

        let response: Book = unary(&service(&seen), CLIENT, request.encode_to_vec())
            .await
            .expect("passed through");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.requests, [format!("{request:?}")]);
        assert_eq!(response.secret, "kept by the handler");
    }

    #[tokio::test]
    async fn a_procedure_the_table_does_not_list_passes_through_untouched() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let request = CreateBookRequest {
            book: Some(book("")),
            ..Default::default()
        };

        let response: Book = unary(&service(&seen), UNLISTED, request.encode_to_vec())
            .await
            .expect("passed through");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.requests, [format!("{request:?}")]);
        assert_eq!(response.secret, "kept by the handler");
    }

    #[tokio::test]
    async fn a_body_that_does_not_decode_is_refused_before_the_handler() {
        let seen = Arc::new(Mutex::new(Seen::default()));

        // Field 2, length-delimited, claiming 5 bytes and holding 1.
        let response: Option<Book> = unary(&service(&seen), CREATE, vec![0x12, 0x05, 0x00]).await;

        assert!(response.is_none());
        let seen = seen.lock().unwrap();
        assert!(
            seen.requests.is_empty(),
            "the handler ran: {:?}",
            seen.requests
        );
        let (code, message, source) = seen.error.clone().expect("an error");
        assert_eq!(code, ErrorCode::InvalidArgument);
        // Fixed, since the decoder's own text can carry the value; that text is
        // the source, for the log.
        assert_eq!(
            message,
            "The request could not be processed due to malformed data"
        );
        assert!(source.contains("failed to decode proto"), "{source}");
    }

    #[tokio::test]
    async fn a_rule_that_cannot_run_is_an_internal_error_with_its_cause_kept() {
        let seen = Arc::new(Mutex::new(Seen::default()));

        let response: Option<Book> = unary(
            &service(&seen),
            GET,
            GetBookRequest::default().encode_to_vec(),
        )
        .await;

        assert!(response.is_none());
        let seen = seen.lock().unwrap();
        assert!(
            seen.requests.is_empty(),
            "the handler ran: {:?}",
            seen.requests
        );
        let (code, message, source) = seen.error.clone().expect("an error");
        assert_eq!(code, ErrorCode::Internal);
        assert_eq!(message, "validation failed");
        assert!(source.contains("no such function: is_book"), "{source}");
    }
}
