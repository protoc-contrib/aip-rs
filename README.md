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
| [134](https://google.aip.dev/134) | `update_mask` validation | **generated** | — |
| [203](https://google.aip.dev/203) | field behavior | **generated** | — |
| [132](https://google.aip.dev/132#ordering), [158](https://google.aip.dev/158), [160](https://google.aip.dev/160) | `order_by`, page tokens, `filter` | the query layer — see below | — |

## Scope: what is deliberately not here

**List queries.** `order_by`, page tokens and `filter` are the query layer's.
Which fields a List request may name is the mapping from AIP paths to columns,
a filter is only as good as the query it becomes, and a page token is whatever
the pager resumes from — a keyset cursor, not an offset. Decided anywhere else,
they could only disagree with the query that runs, so they live where it runs:
[sqlx-query](https://github.com/sqlx-contrib/sqlx-query) parses all three, for
SQL. The Go predecessor's ordering and pagination, and the ~3,000 lines it
carried to parse AIP-160's CEL-*like* grammar, have no counterpart here.

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
    fn clear_output_only(&mut self) { self.create_time = None; /* ... */ }
}
```

Reads are a different matter. Validating `REQUIRED` fields and checking
`update_mask` paths are both read-only, so they can live
here and use `Reflectable` the way aip-go uses `protoreflect`. Revisit this
if a buffa release lands `reflect_mut`.

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
