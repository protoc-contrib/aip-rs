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
dependencies — not even a protobuf runtime.

| AIP | Concept | Where it lives | |
| --- | --- | --- | --- |
| [122](https://google.aip.dev/122) | resource names | `resource` (pattern scan/format) + generated types | ✅ |
| [159](https://google.aip.dev/159) | wildcard segments | `resource` | ✅ |
| [134](https://google.aip.dev/134) | `update_mask` expansion (`MUTABLE_PATHS`, `implied_update_mask`) | **generated** | — |
| [134](https://google.aip.dev/134) | `update_mask` validation | protovalidate (`field_mask.in`) | — |
| [203](https://google.aip.dev/203) | `OUTPUT_ONLY` clearing, `IMMUTABLE` changes | **generated** | — |
| [203](https://google.aip.dev/203) | `REQUIRED` | protovalidate (`required`, `min_len`) | — |
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
walk instead — which it can do well, knowing the field list at codegen time:

```rust
impl Collection {
    fn clear_output_only(&mut self) { self.create_time = Default::default(); /* ... */ }
}
```

The read-only halves — the writable paths, the implied update mask, the
`IMMUTABLE` changes — are generated too, rather than read through
`Reflectable` here, so that field behavior has one mechanism, not two, and
works with reflection off. Validating `REQUIRED` fields and `update_mask` paths
is not this crate's at all: protovalidate's `required` and `field_mask.in`
already run on every request. Revisit the split if a buffa release lands
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
