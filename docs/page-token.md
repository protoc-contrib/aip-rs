<!-- Moved here from grpc-go-template/grpc-rust-template: this is the
normative specification, so it lives with the implementation. -->

# Page token wire format

**Decided: Rust and Go tokens are not interchangeable.** This document is a
design to copy, not a compatibility contract. See
[Why not compatible](#why-not-compatible) for the reasoning and for the
cheap path back if that changes.

Use version byte `0x02` in Rust rather than `0x01`, so a token fed to the
wrong implementation is rejected immediately and unmistakably instead of
decoding structurally and then failing its checksum.

The shape below is still worth copying exactly: every part of it came from
fixing a real bug, and the test vectors will verify your varint and tag
handling even though the checksums will differ.

A page token supports both AIP-158 styles. Use one per List method:

- **Offset** — the number of rows already returned. Cheap to implement; the
  cost of skipping grows with the page number and concurrent writes shift rows
  across page boundaries.
- **Key-set cursor** — the sort-key values of the last row returned. Constant
  cost per page and stable under concurrent writes, provided the trailing
  `order_by` field is unique. The cursor tuple must line up positionally with
  the ordering fields.

## Encoding

`base64url`, **no padding**. The decoded bytes are:

| Field | Encoding |
| --- | --- |
| version | 1 byte, `0x02` here and `0x01` in Go — see above |
| offset | signed LEB128 varint (zigzag), as Go's `binary.PutVarint` |
| request checksum | `u32`, **little-endian**, fixed 4 bytes |
| cursor length | unsigned LEB128 varint |
| cursor values | *length* × (tag byte + payload) |

Trailing bytes after the last cursor value are an error, as is an unknown
version byte. Bump the version for any change that would make an
already-issued token decode differently.

## Cursor value tags

Wire format — never renumber. Adding a type means appending a tag and bumping
the version.

| Tag | Type | Payload |
| --- | --- | --- |
| `0x00` | null | none |
| `0x01` | false | none |
| `0x02` | true | none |
| `0x03` | string | uvarint length, then UTF-8 bytes |
| `0x04` | bytes | uvarint length, then bytes |
| `0x05` | int | signed varint (zigzag), 64-bit |
| `0x06` | uint | unsigned varint, 64-bit |
| `0x07` | float | 8 bytes, IEEE-754 little-endian |
| `0x08` | timestamp | signed varint seconds, then signed varint nanoseconds; UTC |
| `0x09` | duration | signed varint nanoseconds |

Sized integers widen to 64-bit and `f32` widens to `f64`, so a decoded cursor
is not always identical in type to what produced it. The widening is lossless.

An **unset** message or `optional` field contributes a null (`0x00`), so the
query layer can compare it as SQL `NULL` rather than as a zero value. A
presence-less proto3 scalar set to zero is a real value, not a null.

## Request checksum

CRC-32 (IEEE) over the request, deterministically marshalled, with
`page_token`, `page_size` and `skip` cleared, then XORed with `0x9acb0442`.

Two details that were bugs before they were requirements:

- **Marshalling must be deterministic.** Protobuf map field ordering is
  explicitly unspecified; a non-deterministic encoder makes the checksum of a
  request carrying a map flap between calls, which rejects valid page tokens at
  random.
- **The mask** keeps a token issued by a different token type over the same
  request from validating.

A mismatch means the client changed `filter` or `order_by` mid-page. Return
`InvalidArgument`, distinguishable from a malformed token.

## Test vectors

Originally produced by the Go implementation, then reissued with version byte
`0x02`; every byte after the first is unchanged. Checksums are supplied here
rather than computed from a request, so the vectors stay meaningful across
implementations even though real checksums do not — see
[Why not compatible](#why-not-compatible).

A conforming encoder must reproduce these exactly, and a conforming decoder
must accept them.

| Token | Encoded |
| --- | --- |
| empty | `AgAAAAAAAA` |
| offset 100, checksum `0xdeadbeef` | `AsgB776t3gA` |
| cursor `["Alice", "uuid-7"]`, checksum `0xdeadbeef` | `AgDvvq3eAgMFQWxpY2UDBnV1aWQtNw` |
| offset 3, checksum `0x01020304`, cursor `[null, true, false, "hi", 0x01ff, -2, 7u, 1.5, 2023-11-14T22:13:20.123456789Z, 90m]` | `AgYEAwIBCgACAQMCaGkEAgH_BQMGBwcAAAAAAAD4PwiAxJ_VDKq03nUJgMCnkam6Ag` |

The Go spellings — the same rows with a leading `0x01` — are checked here too,
as inputs that must be *rejected* on the version byte. That is the guarantee
the distinct version buys, so it is worth a test rather than a comment. See
`tests/pagination_vectors.rs`.

## An unencodable cursor must never reach the wire

The Go predecessor returned a bare `String()` and discarded the encoding
error. A cursor holding a value the encoder could not represent produced a
*silently truncated* token that failed to decode on the client's next request,
surfacing as a confusing error one round-trip away from the cause. So in Go,
encoding returns a result: serving a page with an unencodable cursor is an
internal error, not something to paper over.

**In Rust the failure is unrepresentable instead of reported.** A cursor value
is the enum `CursorValue`, whose variants are exactly the tags in the table
above, so there is no way to build the cursor that produced the Go bug and
`PageToken::encode` is infallible. This is the stronger form of the same
requirement, not a relaxation of it — the check moves from run time to the
type, and to the `From` conversions that widen `i32`, `u8`, `f32` and friends
onto the four numeric tags the wire format has.

The other half of the rule survives as a run-time check, because no type can
carry it: a key-set cursor with **no** ordering fields is an error, not an
empty cursor. Without a sort key there is nothing to seek on, and an empty
cursor yields a token that cannot page. `next_cursor` rejects it; when a
request carries no `order_by`, advance the offset instead.

## Where the Rust decoder is stricter

Two inputs the Go implementation accepts and this one rejects. Both are
narrowing, so every token Rust issues still decodes; they only reject tokens
Rust would never have written.

- **A string cursor value must be UTF-8.** A Go `string` may hold arbitrary
  bytes and a Rust `String` may not, so a non-UTF-8 payload under tag `0x03`
  is a decode error. Sort keys that are not text belong under tag `0x04`.
- **The base64 must be canonical.** The unused low bits of the final character
  are discarded on decode, so accepting non-zero ones would give a single
  token several valid spellings — which in turn would make a token usable as a
  cache or dedupe key only by accident. Go's decoder does not check them.

## Why not compatible

The checksum makes cross-implementation tokens impractical. It is a CRC-32
over the deterministically-marshalled request, and protobuf's own
documentation rules that out as a cross-language primitive:

> Note that the deterministic serialization is NOT canonical across
> languages. It is not guaranteed to remain stable over time. […] Users who
> need canonical serialization (e.g., persistent storage in a canonical form,
> **fingerprinting**, etc.) must define their own canonicalization
> specification and implement their own serializer rather than relying on
> this API.

Fingerprinting is precisely what this checksum does. Porting the wire format
byte-for-byte would reproduce the cursor bytes and still not reproduce the
checksum; real compatibility would mean hand-writing a canonical serializer
in both languages and freezing it forever.

Set against that, the scenario needing it is small and self-healing. A stale
token already returns `InvalidArgument`, so at a Go-to-Rust cutover in-flight
clients restart pagination from page one — one page-lifetime of degradation,
versus a permanent constraint on two codebases.

**Note this also bounds the Go implementation.** "Not guaranteed to remain
stable over time" applies within a single language: a protobuf library
upgrade could in principle change the serialization and invalidate in-flight
tokens. The failure is the same benign one — `InvalidArgument`, restart from
page one — but the checksum is stable by convention, not by contract.

### The cheap path back

If tokens ever do need to interoperate, do not write a canonical serializer.
Checksum the query-defining strings instead of the marshalled message:

    crc32(parent + "\0" + filter + "\0" + order_by)

Those are plain UTF-8 and canonical by construction, so any language computes
the same value. The checksum only has to detect "the client changed the query
mid-page", and it does not need to cover the whole message to do that — it is
in fact *more* stable this way, being immune to protobuf version drift.

The trade is that it stops detecting changes to other request fields. For
these templates that means only `parent`, which the expression above already
covers.

## Implementations

`src/pagination.rs` in this crate is the implementation this document
specifies. Where the two disagree, **this document wins** — it is normative
here, which is the point of moving it next to the code.

[protoc-contrib/aip-go](https://github.com/protoc-contrib/aip-go), file
`pagination.go`, is where the design came from and remains the authority on
its own tokens. It is no longer a compatibility target; the two now differ
deliberately, in the version byte and in the two decoder checks above.

One piece of the specification lives outside this crate. The request checksum
needs the request message cleared of `page_token`, `page_size` and `skip` and
then marshalled deterministically, and neither is possible without protobuf
reflection — which is why `aip::pagination::request_checksum` takes the
already-marshalled bytes and generated code supplies them. See the scope note
in the README.
