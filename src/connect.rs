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
use buffa_types::FieldMask;
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

use crate::{FieldBehavior, FieldBehaviorKind, FieldPath, FieldScope, FieldType, Resource, Step};

/// One procedure's message types, as the steps [`FieldBehaviorInterceptor`]
/// applies to it: `inbound` to each message the server receives, `outbound` to
/// each message it sends. Each step decodes the message from the payload as
/// its own type and puts it back.
///
/// Built with [`Procedure::new`] or [`Procedure::well_known`]; the fields are
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
    /// A procedure of any cardinality: [`prepare`] on each message received
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
            inbound: prepare::<Req>,
            outbound: clear_input_only::<Resp>,
        }
    }

    /// A procedure whose request is a well-known type, such as
    /// `google.protobuf.Empty`: `OUTPUT_ONLY` cleared on each message received
    /// and `INPUT_ONLY` on each message sent. Such a request writes no
    /// resource and carries no protovalidate rules, so clearing is all
    /// [`prepare`] would do -- and protovalidate-buffa does not implement
    /// `Validate` for it.
    #[must_use]
    pub const fn well_known<Req, Resp>(path: &'static str) -> Self
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
fn apply(
    step: fn(&mut Payload) -> Result<(), ConnectError>,
    item: Result<Payload, ConnectError>,
) -> Result<Payload, ConnectError> {
    let mut payload = item?;
    step(&mut payload)?;
    Ok(payload)
}

/// [`Procedure::well_known`]'s inbound step: `OUTPUT_ONLY` cleared on a
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

fn clear<M>(payload: &mut Payload, kind: FieldBehaviorKind) -> Result<(), ConnectError>
where
    M: buffa::Message + JsonSerialize + JsonDeserialize + Clone + FieldBehavior + 'static,
{
    let mut message = payload.message::<M>()?.clone();
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
///    ignore rather than reject ([`Resource::clear_field_mask`]) -- a
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
pub fn prepare<M>(payload: &mut Payload) -> Result<(), ConnectError>
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
    let mut request = payload.message::<M>()?.clone();
    request.clear_fields(FieldBehaviorKind::OutputOnly);
    imply(&mut request, &mut || uuid::Uuid::now_v7().to_string());
    validate(&mut request).map_err(connect_error)?;
    payload.set_message(request);
    Ok(())
}

/// `error` as [`prepare`] answers it: protovalidate-buffa's rendering, and for
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

/// Step 2 of [`prepare`], with `new_id` minting a create's omitted id.
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

/// Step 3 of [`prepare`]: protovalidate's violations, then a `required` one
/// per path [`required_paths`] finds. A rule that failed to compile or run is
/// returned as it is, which `into_connect_error` makes an `Internal`.
fn validate<M>(request: &mut M) -> Result<(), ValidationError>
where
    M: ResourceWrites + Validate,
{
    let mut error = match request.validate() {
        Ok(()) => ValidationError::default(),
        Err(error) if error.compile_error.is_some() || error.runtime_error.is_some() => {
            return Err(error);
        }
        Err(error) => error,
    };
    error
        .violations
        .extend(required_paths(request).iter().map(required_violation));
    if error.violations.is_empty() {
        Ok(())
    } else {
        Err(error)
    }
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

/// A request for an AIP standard method, by the resources it writes.
///
/// protoc-gen-rust-aip implements it for every message a service takes, and
/// for the entries of a batch one. Three shapes are recognized, as aip-go
/// recognizes them by reflection:
///
/// - a create (AIP-133): a singular field whose type is a resource, and no
///   `update_mask`;
/// - an update (AIP-134): that resource field, and a `google.protobuf.FieldMask
///   update_mask` beside it -- the schema, not the value, makes it an update;
/// - a batch (AIP-233, AIP-234): a repeated `requests` of creates or updates,
///   each written under `requests[i]`.
///
/// Every other request writes nothing, which the default says.
///
/// [`prepare`] is built on it; so is a service's own policy, such as filling
/// in ids its own way.
pub trait ResourceWrites {
    /// Calls `f` with each resource this request writes: its own, or one per
    /// entry of a batch. A request whose resource is unset writes none.
    fn for_each_write(&mut self, _f: &mut dyn FnMut(ResourceWrite<'_>)) {}
}

/// One resource a standard-method request writes.
pub struct ResourceWrite<'a> {
    /// Where the resource is in the request: `item`, or `requests[1].item`.
    pub path: FieldPath,
    /// The resource.
    pub resource: &'a mut dyn DynResource,
    /// What the method needs of it.
    pub method: WriteMethod<'a>,
}

impl ResourceWrite<'_> {
    /// The write with its path prefixed with `prefix`: how a batch reports an
    /// entry's write, under `requests[i]`.
    #[must_use]
    pub fn prefixed(self, prefix: &FieldPath) -> Self {
        Self {
            path: prefix.clone().join(self.path),
            ..self
        }
    }
}

/// Which standard method a [`ResourceWrite`] is for, with the request fields
/// it needs.
pub enum WriteMethod<'a> {
    /// A create (AIP-133).
    Create {
        /// The request's `<resource>_id`, empty when the client left the id to
        /// the server; `None` when the request has no such field.
        id: Option<&'a mut String>,
    },
    /// An update (AIP-134).
    Update {
        /// The request's `update_mask`, with no paths when the client omitted
        /// it. Reaching it makes the mask present, which an implied mask then
        /// fills.
        mask: &'a mut FieldMask,
    },
}

/// What a [`ResourceWrite`] needs of its resource, in a form a `&mut dyn` can
/// hold: every [`Resource`] is one.
pub trait DynResource: FieldBehavior {
    /// [`Resource::implied_field_mask`].
    fn implied_field_mask(&self) -> FieldMask;

    /// [`Resource::clear_field_mask`].
    fn clear_field_mask(&self, kind: FieldBehaviorKind, mask: &mut FieldMask);
}

impl<T: Resource> DynResource for T {
    fn implied_field_mask(&self) -> FieldMask {
        Resource::implied_field_mask(self)
    }

    fn clear_field_mask(&self, kind: FieldBehaviorKind, mask: &mut FieldMask) {
        Resource::clear_field_mask(self, kind, mask);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldBehaviorError, ResourceName};

    /// `books/{book}`, enough of a typed name for [`Book`] to be a resource.
    struct BookName(String);

    impl core::fmt::Display for BookName {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl core::str::FromStr for BookName {
        type Err = core::convert::Infallible;

        fn from_str(name: &str) -> Result<Self, Self::Err> {
            Ok(Self(name.to_owned()))
        }
    }

    impl ResourceName for BookName {
        fn resource_type(&self) -> &str {
            "example.com/Book"
        }

        fn pattern(&self) -> &str {
            "books/{book}"
        }

        fn validate(&self) -> Result<(), crate::resource::InvalidResourceIdError> {
            Ok(())
        }

        fn contains_wildcard(&self) -> bool {
            false
        }
    }

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
        type Name = BookName;

        fn parse_name(&self) -> Result<BookName, core::convert::Infallible> {
            self.name.parse()
        }

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
    fn a_create_missing_a_required_field_is_refused_with_its_violations() {
        let mut request = CreateBook {
            book: Some(Book {
                create_time: "injected".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        };
        // prepare's steps; the interceptor tests run prepare itself
        request.clear_fields(FieldBehaviorKind::OutputOnly);
        imply(&mut request, &mut || uuid::Uuid::now_v7().to_string());
        let error = connect_error(validate(&mut request).expect_err("title is unset"));
        assert_eq!(error.code, connectrpc::ErrorCode::InvalidArgument);
        assert_eq!(
            error.message.as_deref(),
            Some("The request contains invalid parameters")
        );
        assert!(
            !error.details.is_empty(),
            "the violations travel as a detail"
        );
        assert!(
            error
                .details
                .iter()
                .any(|detail| detail.type_url == BadRequest::FULL_NAME),
            "and as a google.rpc.BadRequest: {:?}",
            error.details
        );
        assert_eq!(request.book.expect("the book stays").create_time, "");
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
    fn a_complete_create_passes_with_a_uuid_v7_id() {
        let mut request = CreateBook {
            book: Some(Book {
                title: "Dune".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        };
        // prepare's steps; the interceptor tests run prepare itself
        request.clear_fields(FieldBehaviorKind::OutputOnly);
        imply(&mut request, &mut || uuid::Uuid::now_v7().to_string());
        validate(&mut request).expect("nothing is missing");
        let id = uuid::Uuid::parse_str(&request.book_id).expect("a UUID");
        assert_eq!(id.get_version_num(), 7);
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
