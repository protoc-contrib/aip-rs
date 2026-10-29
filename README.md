# aip-rs

[![CI](https://github.com/protoc-contrib/aip-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/protoc-contrib/aip-rs/actions/workflows/ci.yml)
[![Rust (edition 2024)](https://img.shields.io/badge/Rust-2024-black?logo=rust)](https://www.rust-lang.org/)
[![Nix Flake](https://img.shields.io/badge/Nix-Flake-5277C3?logo=nixos&logoColor=white)](https://nixos.wiki/wiki/Flakes)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Runtime primitives for the [Google API Improvement Proposals](https://google.aip.dev),
in Rust.

The runtime companion to
[protoc-gen-rust-aip](https://github.com/protoc-contrib/protoc-gen-rust-aip),
and the Rust counterpart of
[aip-go](https://github.com/protoc-contrib/aip-go).

## Status

The one module that is pure data manipulation is implemented, along with the
traits generated code implements and, behind the default `connect` feature,
the Connect interceptor that applies them at an RPC boundary. Without
`connect` the only dependency is `buffa-types`, for the `FieldMask` a
resource implies. The optional `descriptor` feature adds buffa-descriptor, to
read `google.api.field_behavior` off a descriptor pool's fields as a
`FieldBehaviorSet`:

```toml
aip = { package = "aip-rs", git = "https://github.com/protoc-contrib/aip-rs", tag = "v0.11.0" }                                  # with the interceptor x-release-please-version
aip = { package = "aip-rs", git = "https://github.com/protoc-contrib/aip-rs", tag = "v0.11.0", default-features = false }        # primitives only x-release-please-version
aip = { package = "aip-rs", git = "https://github.com/protoc-contrib/aip-rs", tag = "v0.11.0", features = ["descriptor"] } # x-release-please-version
```

| AIP | Concept | Where it lives | |
| --- | --- | --- | --- |
| [122](https://google.aip.dev/122) | resource names | `ResourceName` and `ResourceNameError` + **generated** types, parsers and `validate` | ✅ |
| [123](https://google.aip.dev/123) | a resource and its typed name (the generated `parse_name`) | **generated** impls | ✅ |
| [159](https://google.aip.dev/159) | wildcard segments | `resource` | ✅ |
| [134](https://google.aip.dev/134) | the field mask an omitted `update_mask` implies (`implied_field_mask`) | `Resource` trait, **generated** impls | ✅ |
| [203](https://google.aip.dev/203#output-only) | `OUTPUT_ONLY` paths in an explicit `update_mask`, ignored (`clear_field_mask`: removes the paths that name, or pass through, a field of a behavior) | `Resource` trait, built on the generated `validate_fields` | ✅ |
| [134](https://google.aip.dev/134) | `update_mask` validation | protovalidate (`field_mask.in`) | — |
| [203](https://google.aip.dev/203) | clearing the fields of a behavior (`clear_fields`), e.g. `OUTPUT_ONLY` on input, `INPUT_ONLY` on output | `FieldBehavior` trait, **generated** impls | ✅ |
| [203](https://google.aip.dev/203) | checking the fields of a behavior within a scope (`validate_fields`), e.g. `REQUIRED` — all on a create, those the mask names on an update | `FieldBehavior` trait, **generated** impls; `FieldBehaviorError`, `FieldPath` (each field with its number and `FieldType`, as protovalidate reports a violation's path); `FieldError`, a failure and the `FieldPath` of the field it was of, for a request field that does not parse | ✅ |
| [203](https://google.aip.dev/203) | a behavior's `google.api.FieldBehavior` number (`TryFrom<i32>`/`From<FieldBehaviorKind> for i32`, `UnknownFieldBehavior`); a set of behaviors (`FieldBehaviorSet`); a descriptor field's behaviors (`FieldDescriptorExt`, `descriptor` feature) | `field_behavior`, `descriptor` | ✅ |
| [203](https://google.aip.dev/203) | `REQUIRED` on a request's own fields, value rules | protovalidate (`required`, `min_len`, …) | — |
| [133](https://google.aip.dev/133), [134](https://google.aip.dev/134), [233](https://google.aip.dev/233), [234](https://google.aip.dev/234) | a standard-method request's shape — the resources a create, update or batch writes (`ResourceWrites`, `ResourceWrite`, `WriteMethod`, `DynResource`) | `connect`, **generated** impls | ✅ |
| [133](https://google.aip.dev/133), [134](https://google.aip.dev/134), [203](https://google.aip.dev/203) | the server's policy at the RPC boundary — clear `OUTPUT_ONLY`/`INPUT_ONLY`, fill in a create's omitted id, imply an update's mask, check `REQUIRED` per method with protovalidate's rules (`FieldBehaviorInterceptor`, `Procedure`, `validate`) | `connect` (default feature); `PROCEDURES` **generated** | ✅ |
| [160](https://google.aip.dev/160) | `filter` environment and syntax check | **generated** (CEL) | — |
| [132](https://google.aip.dev/132#ordering), [158](https://google.aip.dev/158) | `order_by`, page tokens | the query layer — see below | — |

## Layout

One module per AIP concept, everything re-exported at the crate root:

```
src/
├── lib.rs
├── resource.rs            names (AIP-122, 159)
├── field/                 request fields
│   ├── path.rs            FieldPath, FieldResolver
│   ├── error.rs           FieldError
│   ├── behavior.rs        field behavior (AIP-203)
│   └── descriptor.rs      feature `descriptor`
├── error/                 errors a client sees (AIP-193, 194)
│   ├── mod.rs             details, status, error_log_level
│   └── interceptor.rs     ErrorInterceptor
└── request/               what a server does with a request (AIP-133, 134, 233, 234)
    ├── writes.rs          ResourceWrites
    ├── behavior.rs        FieldBehaviorInterceptor, Procedure
    └── prepare.rs         RequestPreparer, PrepareInterceptor
```

## The interceptor

`FieldBehaviorInterceptor` applies AIP field behavior on a Connect server, with
nothing to configure — the Rust counterpart of aip-go's
`NewFieldBehaviorInterceptor`:

| Each message received | Each message sent |
| --- | --- |
| clear `OUTPUT_ONLY`; fill in a create's omitted `<resource>_id` (a UUIDv7) and an update's omitted `update_mask`, and drop the `OUTPUT_ONLY` paths of an explicit one (`clear_field_mask`); validate `REQUIRED` — every field on a create, those the mask names on an update — with protovalidate's rules, into one `InvalidArgument`, "The request contains invalid parameters", with the violations in its details | clear `INPUT_ONLY` |

It acts on every message of every RPC, unary or streamed. A message that is not
a standard-method request — a resource streamed on its own — has nothing filled
in or checked for `REQUIRED`; only clearing and its protovalidate rules apply.

```rust
let service = ConnectRpcService::new(router)
    .with_interceptor(aip::FieldBehaviorInterceptor::new(sdk::v1::PROCEDURES));
```

An interceptor sees bytes, not types, so it is handed the `PROCEDURES` table
protoc-gen-rust-aip generates per package: one `Procedure` per method, whose
`inbound` and `outbound` steps decode each message as its own type. It edits
the messages it passes on, the outbound ones included, and leaves a client's
calls alone. Every message buffa and protoc-gen-rust-aip generate qualifies,
the well-known types included (aip-rs gives them empty `FieldBehavior`
impls), so a method taking `google.protobuf.Empty` needs nothing special.
`aip::request::behavior::validate::<M>` applies the inbound steps to a payload holding an
`M`, exactly as the interceptor does, for a caller such as a handler test.

**Names are the server's.** AIP-133 has the server build a created resource's
`name` from the request's `parent` and `<resource>_id`. The interceptor fills
in only an omitted id; it writes no name and no parent. A handler reads both
off the request — the generated `parse_<resource>_id` and `parse_parent` give
them typed — stores the resource under the name they make, and returns the
name storage holds. A service with another id scheme fills the id in with its
own interceptor ahead of this one, through `ResourceWrites::for_each_write`:
an id that is already set is kept.

The public surface of `aip::request` is that and no more:
`FieldBehaviorInterceptor`, `Procedure` (`path`, `inbound`, `outbound`;
`Procedure::new`, `Procedure::clear` for a well-known request type),
`ResourceWrites`, `ResourceWrite`,
`WriteMethod`, `DynResource` (every `Resource` is one) and `validate`.

## Preparing a request

`RequestPreparer` is a request that readies itself for its handler, and
`PrepareInterceptor` calls it, after `FieldBehaviorInterceptor`: where a service
does what AIP leaves to the server and no library decides for it, such as
building a create's name from its `parent` and id (AIP-133) or filling a List
call's `page_size` (AIP-158). Like the field behavior interceptor it is handed a
table, `Preparer::new::<Request>(path)` per procedure; a request it refuses is
`InvalidArgument` with the field it names as a `BadRequest`, or what
`with_reject` answers.

## The error model

`aip::error` (the `connect` feature) is the AIP-193 error model of a Connect
server, the Rust counterpart of aip-go's `errors.go` and `error_interceptor.go`.
Nothing in it knows a service: the domain of an `ErrorInfo`, its reasons and the
public messages are the service's, and it builds its errors from these parts.

| | |
| --- | --- |
| details | `error_info(domain, reason)`, `resource_info(name)`, `retry_info(delay)`, `bad_request(field_error)` -- with a fixed description, since the error's own text can carry the value --, and `debug_info(cause)`; `error.detail::<M>()` (the one-method `ConnectErrorExt`) reads one back |
| a batch entry | `status(&error)` is the `google.rpc.Status` of a failed entry of a `Batch*` response, less its `DebugInfo`; the service names the fields of its `BadRequest` under `requests[i]`, where it builds the batch |
| `ErrorInterceptor` | logs each failed unary call once, at ERROR for a fault the request did not cause (`Internal`, `Unknown`, `DataLoss`, `Unavailable`) and WARN for a caller's mistake, with the cause and the stack of its `DebugInfo`, then answers without that `DebugInfo`. `with_default_error_info(domain)` gives an error with no `ErrorInfo` one, its code as the reason; `with_request_id(f)` sends the request's id as a `RequestInfo` |

A body that does not decode is answered by `FieldBehaviorInterceptor` with a
fixed message, "The request could not be processed due to malformed data": the
decoder's text can quote the value, so it is kept as the error's source, for the
log.

Register `ErrorInterceptor` first, so what the interceptors after it refuse is
logged and answered the same way.

## Parity with aip-go

aip-rs keeps aip-go's names and behaviour, in Rust's casing, and departs only
where the language makes it: the table lists every exported aip-go identifier
and what stands for it. Anything not in it, in either direction, is a bug.

| aip-go | aip-rs | |
| --- | --- | --- |
| `NewErrorInfo`, `NewResourceInfo`, `NewRetryInfo`, `NewBadRequest`, `NewDebugInfo` | `error::error_info`, `resource_info`, `retry_info`, `bad_request`, `debug_info` | `bad_request` takes a `&FieldError`, where Go panics on an error that is not one |
| `NewErrorDetail` | `ErrorDetail::from_message` | connectrpc's |
| none | `ConnectErrorExt::detail::<M>()` | a Rust-only reader: Go reads a detail with `connectproto.UnmarshalErrorDetail` |
| `NewStatus` | `error::status` | for a `ConnectError`: Rust has no other error to convert; it is less its `DebugInfo`, which Go's interceptor takes out later and nothing here wraps the response to do |
| `ErrorLogLevel` | `error::error_log_level` | |
| `NewErrorInterceptor`, `WithDefaultErrorInfo`, `WithErrorRequestID` | `error::ErrorInterceptor`, `with_default_error_info`, `with_request_id` | |
| `WithErrorLogger` | none | the logs are `tracing` events |
| the interceptor's `Send` over a batch response | `error::status` and `error_log_level`, called where the entry's status is built | an interceptor has no reflection to find the entries in, so the caller logs them |
| `FieldError`, `NewFieldError`, `FieldPath` | `FieldError`, `FieldError::of::<M>`, `FieldPath::of::<M>` | the failure is any boxed error, as Go's `Err error`: `failure()` is `Unwrap`, and `downcast_ref` asks for its type; `Field()` is the path's `Display` |
| `FieldBehaviorKind`, `FieldBehaviorSet` (`Has`, `HasAny`, `All`), `FieldBehaviorsOf` | `FieldBehaviorKind`, `FieldBehaviorSet` (`contains`, `intersects`, `iter`), `FieldDescriptorExt` | |
| `ClearFields`, `ValidateFields`, `FieldBehaviorError` | `FieldBehavior::clear_fields`, `validate_fields`, `FieldBehaviorError` | generated, as there is no reflection |
| `ImpliedFieldMask`, `ClearFieldMask` | `Resource::implied_field_mask`, `clear_field_mask` | |
| `NewFieldBehaviorInterceptor` | `FieldBehaviorInterceptor::new(PROCEDURES)` | the table stands for reflection |
| `ResourceWrites`, `ResourceWrite`, `ResourceCreate`, `ResourceUpdate` | `ResourceWrites::for_each_write`, `ResourceWrite`, `WriteMethod::{Create, Update}` | an enum where Go has two optional pointers |
| `ResourceWildcard` | `resource::RESOURCE_WILDCARD` | a name holds a segment as a string, which can be `-`; whether it does is for the consumer to compare |
| none | none | the names and their parsers, `Validate` and `<segment>_id()` are generated by `protoc-gen-rust-aip`, as `protoc-gen-go-aip`'s are: nothing of the parsing lives here |
| `ResourceName` | `ResourceName` | `Type` is `resource_type`, `type` being a keyword |
| the `error` a parse and a `Validate` return | `ResourceNameError`, a message | one type for every generated name, where Go has `fmt.Errorf`; the messages are Go's |
| none | `FieldScope`, `DynResource`, `UnknownFieldBehavior`, `FieldBehaviorSet::iter`'s `Iter` | Rust-only: what the table standing for reflection and the type system need, which Go reads from a descriptor or an `error` |
| `RequestPreparer`, `NewPrepareInterceptor(reject)` | `RequestPreparer`, `PrepareInterceptor::new(PREPARERS).with_reject(..)` | a table of `Preparer`s stands for reflection; `prepare` returns any boxed error, and the default reject names the field a `FieldError` in it is about, as Go's `rejectPrepare` does |

## Scope: what is deliberately not here

**Ordering and paging.** `order_by` and page tokens are the query layer's: how
a List request sorts is the columns the query orders by, and a page token is
whatever the pager encodes in it. Decided anywhere else, they could only
disagree with the query that runs —
[sqlx-query](https://github.com/sqlx-contrib/sqlx-query) parses both, for SQL.
A `filter` is plain CEL: `protoc-gen-rust-aip` generates its environment and a
syntax check onto each List request, and needs nothing from this crate. The ~3,000
lines the Go predecessor carried to parse AIP-160's CEL-*like* grammar have no
counterpart here.

**Mutating a protobuf message.** Clearing fields is *generated*, because buffa
0.9 offers no reflective path to mutation at all:

```rust
fn reflect(&self) -> ReflectCow<'_>;

// `reflect_mut(&mut self) -> ReflectCowMut<'_>` is part of the design but
// deferred to the MergeSink work [...]
```

`Reflectable` hands out an immutable handle in either reflect mode.
`ReflectMessage::clear()` exists, but reaching it needs a
`&mut dyn ReflectMessage` that nothing produces. So the generator emits the
walk instead — which it can do well, knowing the field list at codegen time —
as an implementation of this crate's `FieldBehavior` trait. The read-only
check and a resource's implied field mask are generated alongside rather than
read through `Reflectable` here, so field behavior has one mechanism and works
with reflection off:

```rust
impl aip::FieldBehavior for Item {
    fn clear_fields(&mut self, kind: aip::FieldBehaviorKind) {
        if kind == aip::FieldBehaviorKind::OutputOnly {
            self.parent.clear();
            self.create_time = Default::default();
        }
    }

    fn validate_fields(&self, kind: aip::FieldBehaviorKind, scope: aip::FieldScope<'_>)
        -> Result<(), aip::FieldBehaviorError>
    {
        let mut paths = Vec::new();
        if kind == aip::FieldBehaviorKind::Required && scope.contains("display_name")
            && self.display_name.is_empty()
        {
            paths.push(aip::FieldPath::field("display_name", 3, aip::FieldType::String));
        }
        aip::FieldBehaviorError::check(kind, paths)
    }
}

impl aip::Resource for Item {
    fn implied_field_mask(&self) -> buffa_types::FieldMask { /* the writable fields set */ }
}
```

The plugin implements `FieldBehavior` for every message it generates, and the
defaults do nothing, so generic code over `T: FieldBehavior` works for any of
them; `Resource` only for resources. Bring a trait into scope with
`use aip::FieldBehavior;` where its methods are called.

**Policy.** Every operation works on one message, with whatever
`FieldBehaviorKind` the caller passes. Which a service applies — `OUTPUT_ONLY`
cleared on requests, `INPUT_ONLY` on responses, `REQUIRED` checked with
`FieldScope::All` on a create and the update's mask on an update, `IMMUTABLE`
changes dropped or refused — and to which message of a request, is the
caller's, as is walking a batch's entries:

```rust
use aip::{FieldBehavior, FieldBehaviorKind, FieldPath, FieldScope, FieldType, Resource};

request.clear_fields(FieldBehaviorKind::OutputOnly);
if let Some(item) = request.item.as_option() {
    // an explicit mask ignores the OUTPUT_ONLY fields it names (AIP-203)
    if let Some(mask) = request.update_mask.as_option_mut() {
        item.clear_field_mask(FieldBehaviorKind::OutputOnly, mask);
    }
    let mask = request.update_mask.as_option().map_or(&[][..], |mask| &mask.paths[..]);
    item.validate_fields(FieldBehaviorKind::Required, FieldScope::Mask(mask))
        .map_err(|error| error.prefixed(&FieldPath::field("item", 2, FieldType::Message)))?;
}
```

`REQUIRED` on a resource's fields is checked here rather than by protovalidate:
a resource is shared by its create and update requests, and a `buf.validate`
rule on it cannot tell which request it is in. protovalidate keeps a request's
own fields, value rules and `update_mask` paths (`field_mask.in`).

aip-go answers the same by reflection, with `ClearFields`, `ValidateFields`,
`ImpliedFieldMask` and `ClearFieldMask`. `clear_field_mask` needs nothing
generated of its own: it runs the generated `validate_fields` on the
resource's `Default`, where every field is unset, so the check reports the
fields of a behavior a path names. Revisit the split if a buffa release lands
`reflect_mut` — then all of it could move here behind a `buffa` feature, the way
aip-go does it with `protoreflect`.

## Reflect mode is not this crate's concern

buffa generates reflection in three modes — `Off`, `Bridge` (round-trips
through a `DynamicMessage`; smaller code, an allocation and re-encode per
call) and `VTable` (`impl ReflectMessage` directly; larger code, near-free
access).

Per buffa's own documentation, *"the call site is `foo.reflect().get(fd)`
regardless of mode"*. So write against `Reflectable` and let the consuming
template choose, by measuring generated-code size against per-request cost.
It is one line in `buf.gen.yaml` and nothing here has to change.

## Consuming it

`aip` is taken on crates.io, so the package is `aip-rs`. Rename it back and
generated code reads `aip::ResourceName`:

```toml
[dependencies]
aip = { package = "aip-rs", git = "https://github.com/protoc-contrib/aip-rs", tag = "v0.11.0" } # x-release-please-version
```

Releases from 0.8.0 are git tags, not crates.io versions: the interceptor's
`google.rpc.BadRequest` comes from [buffa-protos], which is not published, and
crates.io takes no git dependency. 0.7.0 and earlier stay on crates.io.

[buffa-protos]: https://github.com/protoc-contrib/buffa-protos

The minimum supported Rust version is 1.85, the first with edition 2024.

The crate is pre-1.0: a breaking change bumps the minor version, and a tag
pins one release, so nothing changes unasked.

## License

MIT. See [LICENSE](LICENSE).
