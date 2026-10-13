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
traits generated code implements. The only dependency is `buffa-types`, for
the `FieldMask` a resource implies. The optional `descriptor` feature adds
buffa-descriptor, to read `google.api.field_behavior` off a descriptor pool's
fields as a `FieldBehaviorSet`:

```toml
aip = { package = "aip-rs", version = "0.5", features = ["descriptor"] }
```

| AIP | Concept | Where it lives | |
| --- | --- | --- | --- |
| [122](https://google.aip.dev/122) | resource names | `resource` (pattern scan/format) + generated types (`ResourceName`) | ✅ |
| [123](https://google.aip.dev/123) | a resource and its typed name (`Resource::Name`, `parse_name`) | `Resource` trait, **generated** impls | ✅ |
| [159](https://google.aip.dev/159) | wildcard segments | `resource` | ✅ |
| [134](https://google.aip.dev/134) | the field mask an omitted `update_mask` implies (`implied_field_mask`) | `Resource` trait, **generated** impls | ✅ |
| [134](https://google.aip.dev/134) | `update_mask` validation | protovalidate (`field_mask.in`) | — |
| [203](https://google.aip.dev/203) | clearing the fields of a behavior (`clear_fields`), e.g. `OUTPUT_ONLY` on input, `INPUT_ONLY` on output | `FieldBehavior` trait, **generated** impls | ✅ |
| [203](https://google.aip.dev/203) | checking the fields of a behavior within a scope (`validate_fields`), e.g. `REQUIRED` — all on a create, those the mask names on an update | `FieldBehavior` trait, **generated** impls; `FieldBehaviorError`, `FieldPath` (each field with its number and `FieldType`, as protovalidate reports a violation's path) | ✅ |
| [203](https://google.aip.dev/203) | a behavior's `google.api.FieldBehavior` number (`TryFrom<i32>`/`From<FieldBehaviorKind> for i32`, `UnknownFieldBehavior`); a set of behaviors (`FieldBehaviorSet`); a descriptor field's behaviors (`FieldDescriptorExt`, `descriptor` feature) | `field_behavior`, `descriptor` | ✅ |
| [203](https://google.aip.dev/203) | `REQUIRED` on a request's own fields, value rules | protovalidate (`required`, `min_len`, …) | — |
| [133](https://google.aip.dev/133), [134](https://google.aip.dev/134) | handling a whole request — naming a create's resource, implying an update's mask, checking each batch entry | the caller, built on these traits | — |
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
            paths.push(aip::FieldPath::field("display_name"));
        }
        aip::FieldBehaviorError::check(kind, paths)
    }
}

impl aip::Resource for Item {
    type Name = ItemName;
    fn parse_name(&self) -> Result<ItemName, aip::resource::ParseError> { self.name.parse() }
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
    let mask = request.update_mask.as_option().map_or(&[][..], |mask| &mask.paths[..]);
    item.validate_fields(FieldBehaviorKind::Required, FieldScope::Mask(mask))
        .map_err(|error| error.prefixed(&FieldPath::field("item", 2, FieldType::Message)))?;
}
```

`REQUIRED` on a resource's fields is checked here rather than by protovalidate:
a resource is shared by its create and update requests, and a `buf.validate`
rule on it cannot tell which request it is in. protovalidate keeps a request's
own fields, value rules and `update_mask` paths (`field_mask.in`).

aip-go answers the same by reflection, with `ClearFields`, `ValidateFields`
and `ImpliedFieldMask`. Revisit the split if a buffa release lands
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
aip = { package = "aip-rs", version = "0.5" }
```

The minimum supported Rust version is 1.85, the first with edition 2024.

The crate is pre-1.0: a breaking change bumps the minor version, so `0.5`
above will not pick one up unasked.

## License

MIT. See [LICENSE](LICENSE).
