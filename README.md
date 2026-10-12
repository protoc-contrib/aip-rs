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

The one module that is pure data manipulation is implemented, with no
dependencies — not even a protobuf runtime — along with the traits generated
code implements.

| AIP | Concept | Where it lives | |
| --- | --- | --- | --- |
| [122](https://google.aip.dev/122) | resource names | `resource` (pattern scan/format) + generated types | ✅ |
| [159](https://google.aip.dev/159) | wildcard segments | `resource` | ✅ |
| [134](https://google.aip.dev/134) | `update_mask` expansion (`MUTABLE_PATHS`, `implied_update_mask`) | `FieldBehavior` trait, **generated** impls | ✅ |
| [133](https://google.aip.dev/133) | naming a create's resource from its id, minting the id when omitted (`imply_resource_name`) | `FieldBehavior` trait, **generated** impls | ✅ |
| [134](https://google.aip.dev/134) | implying an omitted `update_mask` on an update or batch request (`imply_update_mask`; both via `imply_request`) | `FieldBehavior` trait, **generated** impls | ✅ |
| [134](https://google.aip.dev/134) | `update_mask` validation | protovalidate (`field_mask.in`) | — |
| [203](https://google.aip.dev/203) | `REQUIRED` on a resource's fields (`validate_required_fields`) | `FieldBehavior` trait, **generated** impls; `RequiredError` | ✅ |
| [203](https://google.aip.dev/203) | `REQUIRED` across a create, update or batch request (`validate_required`, each field a `FieldPath`) | `FieldBehavior` trait, **generated** impls; `RequiredError`, `FieldPath` | ✅ |
| [203](https://google.aip.dev/203) | `IMMUTABLE` changes (`validate_immutable`), `OUTPUT_ONLY` / `INPUT_ONLY` clearing | `FieldBehavior` trait, **generated** impls; `ImmutableError` | ✅ |
| [203](https://google.aip.dev/203) | `REQUIRED` on a request's own fields, value rules | protovalidate (`required`, `min_len`, …) | — |
| [160](https://google.aip.dev/160) | `filter` environment and syntax check | **generated** (CEL) | — |
| [132](https://google.aip.dev/132#ordering), [158](https://google.aip.dev/158) | `order_by`, page tokens | the query layer — see below | — |

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

**Mutating a protobuf message.** Clearing `OUTPUT_ONLY` fields is *generated*,
because buffa 0.9.1 offers no reflective path to mutation at all:

```rust
fn reflect(&self) -> ReflectCow<'_>;

// `reflect_mut(&mut self) -> ReflectCowMut<'_>` is part of the design but
// deferred to the MergeSink work [...]
```

`Reflectable` hands out an immutable handle in either reflect mode.
`ReflectMessage::clear()` exists, but reaching it needs a
`&mut dyn ReflectMessage` that nothing produces. So the generator emits the
walk instead — which it can do well, knowing the field list at codegen time —
as an implementation of this crate's `FieldBehavior` trait:

```rust
impl aip::FieldBehavior for Collection {
    fn clear_output_only(&mut self) { self.create_time = Default::default(); /* ... */ }
}
```

The read-only halves — the writable paths, the implied update mask and the
`IMMUTABLE` and `REQUIRED` checks — are generated into the same impl rather
than read through `Reflectable` here, so that field behavior has one mechanism,
not two, and works with reflection off. The plugin implements the trait for
every message it generates, and the trait's defaults are the "nothing to do"
answers, so one server layer over `T: FieldBehavior` handles every request;
bring it into scope with `use aip::FieldBehavior;` where its methods are
called. The items are named after what they do: `validate_*` returns `Ok(())`
or an error listing the fields that fail, `imply_*` fills in what a client may
leave to the server, `clear_*` drops what one side of the wire may not see.

`REQUIRED` on a resource's fields is checked by the generated
`validate_required_fields` rather than by protovalidate:
a resource is shared by its create and update requests, a create must set a
`REQUIRED` field and an update only when its mask names it, and a
`buf.validate` rule on the resource cannot tell which request it is in.
protovalidate keeps a request's own fields, value rules and `update_mask`
paths (`field_mask.in`).

A server reads both per request rather than per resource. On every create,
update and batch request the plugin also generates `validate_required`, which
applies the resource's check the way the method asks — every `REQUIRED` field
on a create, only those the `update_mask` names on an update, each
`requests[i]` entry of a batch on its own — and reports each unset field as a
`FieldPath` from the request (`requests[1].item.display_name`) in a
`RequiredError`. It also generates what AIP leaves to the server:
`imply_resource_name` on a create, which mints an omitted `<resource>_id` and
builds the resource's `name` from its pattern, `parent` and that id, and
`imply_update_mask` on an update, which fills an omitted mask from the
resource's `implied_update_mask`; `imply_request` runs both. One server layer
then covers every request:

```rust
use aip::FieldBehavior;

request.clear_output_only();
request.imply_request(&mut || uuid::Uuid::now_v7().to_string());
if let Err(error) = request.validate_required() {
    for path in &error.paths {
        // a `required` violation at `path`
    }
}
```

aip-go's `ImplyRequest` and `ValidateRequired` answer the same, by
reflection. Revisit the split if a buffa release lands
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
generated code reads `aip::ResourcePattern`:

```toml
[dependencies]
aip = { package = "aip-rs", version = "0.1" }
```

The minimum supported Rust version is 1.85, the first with edition 2024.

The crate is pre-1.0: a breaking change bumps the minor version, so `0.1`
above will not pick one up unasked.

## License

MIT. See [LICENSE](LICENSE).
