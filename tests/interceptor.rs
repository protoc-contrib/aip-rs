//! [`FieldBehaviorInterceptor`] on the path a request takes through a
//! connectrpc server -- HTTP in, the router, the interceptor chain, the
//! handler, and back -- as aip-go's `interceptor_test.go` drives its own,
//! rather than through [`aip::connect::validate`] alone.
//!
//! The messages are hand-written `buffa::Message`s, so the crate needs no
//! codegen to test itself: a `Book` resource and the requests that write it,
//! each with the `FieldBehavior`, `Resource`, `ResourceWrites` and `Validate`
//! impls protoc-gen-rust-aip and protoc-gen-protovalidate-buffa would emit.
#![cfg(feature = "connect")]

use std::sync::{Arc, Mutex};

use aip::{
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
        if kind == FieldBehaviorKind::Required && scope.contains("title") && self.title.is_empty() {
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
    let (code, _, _) = seen.error.clone().expect("an error");
    assert_eq!(code, ErrorCode::InvalidArgument);
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
