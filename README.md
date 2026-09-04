# aip-rs

Runtime primitives for the [Google API Improvement Proposals](https://google.aip.dev),
in Rust.

The runtime companion to
[protoc-gen-rust-aip](https://github.com/protoc-contrib/protoc-gen-rust-aip),
and the Rust counterpart of
[aip-go](https://github.com/protoc-contrib/aip-go).

## Status

Nothing implemented yet. The design decisions are settled and recorded — see
[docs/page-token.md](docs/page-token.md) for the wire format, which is the
piece worth building first.

| AIP | Concept | Where it lives |
| --- | --- | --- |
| [122](https://google.aip.dev/122) | resource names | here (pattern scan/format) + generated types |
| [132](https://google.aip.dev/132#ordering) | `order_by` | here |
| [158](https://google.aip.dev/158) | page tokens | here |
| [134](https://google.aip.dev/134) | `update_mask` validation | **generated** |
| [203](https://google.aip.dev/203) | field behavior | **generated** |
| [160](https://google.aip.dev/160) | `filter` | neither — see below |

## Scope: what is deliberately not here

**Filtering.** Filters are plain CEL, not AIP-160, so the runtime is the
[`cel`](https://crates.io/crates/cel) crate and this crate contributes
nothing. The Go predecessor carried ~3,000 lines to parse AIP-160's
CEL-*like* grammar; that grammar was dropped rather than ported.

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

Reads are a different matter. Validating `REQUIRED` fields, extracting a
cursor and checking `update_mask` paths are all read-only, so they can live
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
generated code reads `aip::PageToken`:

```toml
[dependencies]
aip = { package = "aip-rs", git = "https://github.com/protoc-contrib/aip-rs", tag = "v0.1.0" }
```

**This crate is not published to crates.io**, and does not need to be: no
consumer of it is published either. Git is the source of truth. That keeps
the API free to change without version churn, and publishing stays available
later if it ever matters — it is purely additive.

One cost to know about, if you build with Nix: a git dependency needs a
`cargoLock.outputHashes` entry, and the hash changes with every revision you
bump to. Registry dependencies do not. Pin to a tag rather than a branch so
the hash only moves when you decide it does.

## License

MIT. See [LICENSE](LICENSE).
