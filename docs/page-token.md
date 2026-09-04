<!-- Moved here from grpc-go-template/grpc-rust-template: this is the
normative specification, so it lives with the implementation. -->

# Page token wire format

**Decide first: do Rust and Go services need to exchange tokens?** If a client
can be served by either implementation, or a service migrates between them,
then a token issued by one must decode in the other and this section is a
binding specification. If not, reimplement freely — but the shape below is
worth keeping regardless, because the failure modes it avoids were real.

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
| version | 1 byte, currently `0x01` |
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

Produced by the Go implementation. A conforming encoder must reproduce these
exactly, and a conforming decoder must accept them.

| Token | Encoded |
| --- | --- |
| empty | `AQAAAAAAAA` |
| offset 100, checksum `0xdeadbeef` | `AcgB776t3gA` |
| cursor `["Alice", "uuid-7"]`, checksum `0xdeadbeef` | `AQDvvq3eAgMFQWxpY2UDBnV1aWQtNw` |
| offset 3, checksum `0x01020304`, cursor `[null, true, false, "hi", 0x01ff, -2, 7u, 1.5, 2023-11-14T22:13:20.123456789Z, 90m]` | `AQYEAwIBCgACAQMCaGkEAgH_BQMGBwcAAAAAAAD4PwiAxJ_VDKq03nUJgMCnkam6Ag` |

## Encoding a token must be able to fail

The Go predecessor returned a bare `String()` and discarded the encoding
error. A cursor holding a value the encoder could not represent produced a
*silently truncated* token that failed to decode on the client's next request,
surfacing as a confusing error one round-trip away from the cause.

Encoding returns a result. Serving a page with an unencodable cursor is an
internal error, not something to paper over.

For the same reason: a key-set cursor with **no** ordering fields is an error,
not an empty cursor. Without a sort key there is nothing to seek on, and an
empty cursor yields a token that cannot page. When a request carries no
`order_by`, advance the offset instead.

## Reference implementation

[protoc-contrib/aip-go](https://github.com/protoc-contrib/aip-go), file
`pagination.go`. Where this document and that file disagree, the file wins
until the difference is resolved deliberately — the test vectors above are
generated from it.
