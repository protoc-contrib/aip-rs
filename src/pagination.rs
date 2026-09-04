//! AIP-158 pagination: the opaque token a server hands a client so the client
//! can ask for the next page of a List call.
//!
//! The wire format is specified in `docs/page-token.md`; that document is
//! normative and this module implements it.
//!
//! See <https://google.aip.dev/158>.

use core::fmt;
use core::str::FromStr;

use crate::{base64url, crc32, varint};

/// The leading byte of every encoded page token.
///
/// `0x02` rather than the Go implementation's `0x01`, deliberately: the two
/// are not token-compatible, and a distinct version byte makes a token fed to
/// the wrong implementation fail immediately as an unsupported version instead
/// of decoding structurally and then failing its checksum. See "Why not
/// compatible" in `docs/page-token.md`.
///
/// Bump it for any change that would make an already-issued token decode to
/// something different. Tokens written by a previous version are then rejected
/// outright rather than misread.
pub const VERSION: u8 = 0x02;

/// Mixed into the request checksum by [`request_checksum`], so that a token
/// issued by a different token type over the same request does not validate
/// here.
pub const CHECKSUM_MASK: u32 = 0x9acb_0442;

/// Cursor value type tags. Wire format — never renumber. Adding a type means
/// appending a tag and bumping [`VERSION`].
mod tag {
    pub(super) const NULL: u8 = 0x00;
    pub(super) const FALSE: u8 = 0x01;
    pub(super) const TRUE: u8 = 0x02;
    pub(super) const STRING: u8 = 0x03;
    pub(super) const BYTES: u8 = 0x04;
    pub(super) const INT: u8 = 0x05;
    pub(super) const UINT: u8 = 0x06;
    pub(super) const FLOAT: u8 = 0x07;
    pub(super) const TIMESTAMP: u8 = 0x08;
    pub(super) const DURATION: u8 = 0x09;
}

/// A nanosecond in nanoseconds, as it were.
const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// One value of a key-set cursor.
///
/// The set of representable values *is* the set the wire format supports, so
/// unlike the Go predecessor — which took `any` and had to reject unsupported
/// types at encode time — there is no way to build a cursor that cannot be
/// encoded. That is why [`PageToken::encode`] is infallible.
///
/// Sized integers widen to [`i64`]/[`u64`] and [`f32`] widens to [`f64`] via
/// the [`From`] impls, so a decoded cursor is not always of the same Rust type
/// as the value that produced it. The widening is lossless, keeps the wire
/// format small, and does not affect comparison against a database column.
///
/// [`Null`](CursorValue::Null) is what an unset message field or an unset
/// `optional` field contributes, so the query layer can compare it as SQL
/// `NULL` rather than as a zero value. A presence-less proto3 scalar set to
/// zero is a real value, not a null.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorValue {
    /// An unset field. Compare as SQL `NULL`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A UTF-8 string.
    String(String),
    /// An uninterpreted byte string.
    Bytes(Vec<u8>),
    /// A signed integer, and what enum numbers and all signed proto integer
    /// kinds widen to.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A float, and what `f32` and proto `float` widen to.
    Float(f64),
    /// A UTC instant, shaped like `google.protobuf.Timestamp`.
    ///
    /// Build it with [`CursorValue::timestamp`], which normalizes; the encoder
    /// normalizes too, so a hand-built out-of-range `nanos` still produces a
    /// valid token, just one that decodes to the normalized value.
    Timestamp {
        /// Seconds since the Unix epoch.
        seconds: i64,
        /// Nanoseconds within the second, normalized to `0..1_000_000_000`.
        nanos: i32,
    },
    /// A signed duration in nanoseconds, shaped like
    /// `google.protobuf.Duration` flattened the way Go's `time.Duration` is.
    Duration {
        /// The duration in nanoseconds. May be negative.
        nanos: i64,
    },
}

impl CursorValue {
    /// Returns a [`CursorValue::Timestamp`], carrying any excess `nanos` into
    /// `seconds` so that the result is in the range
    /// `google.protobuf.Timestamp` requires.
    #[must_use]
    pub fn timestamp(seconds: i64, nanos: i32) -> Self {
        let (seconds, nanos) = normalize_timestamp(seconds, nanos);
        Self::Timestamp { seconds, nanos }
    }

    /// Returns a [`CursorValue::Duration`] of `nanos` nanoseconds.
    #[must_use]
    pub fn duration_nanos(nanos: i64) -> Self {
        Self::Duration { nanos }
    }

    fn encode_into(&self, buffer: &mut Vec<u8>) {
        match self {
            Self::Null => buffer.push(tag::NULL),
            Self::Bool(false) => buffer.push(tag::FALSE),
            Self::Bool(true) => buffer.push(tag::TRUE),
            Self::String(value) => {
                buffer.push(tag::STRING);
                varint::put_uvarint(buffer, value.len() as u64);
                buffer.extend_from_slice(value.as_bytes());
            }
            Self::Bytes(value) => {
                buffer.push(tag::BYTES);
                varint::put_uvarint(buffer, value.len() as u64);
                buffer.extend_from_slice(value);
            }
            Self::Int(value) => {
                buffer.push(tag::INT);
                varint::put_varint(buffer, *value);
            }
            Self::Uint(value) => {
                buffer.push(tag::UINT);
                varint::put_uvarint(buffer, *value);
            }
            Self::Float(value) => {
                buffer.push(tag::FLOAT);
                buffer.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            Self::Timestamp { seconds, nanos } => {
                let (seconds, nanos) = normalize_timestamp(*seconds, *nanos);
                buffer.push(tag::TIMESTAMP);
                varint::put_varint(buffer, seconds);
                varint::put_varint(buffer, i64::from(nanos));
            }
            Self::Duration { nanos } => {
                buffer.push(tag::DURATION);
                varint::put_varint(buffer, *nanos);
            }
        }
    }

    /// Reads one tagged value off the front of `src`, returning it with the
    /// rest of the buffer.
    fn decode_from(src: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let (&tag, src) = src.split_first().ok_or(DecodeError::Truncated)?;
        match tag {
            tag::NULL => Ok((Self::Null, src)),
            tag::FALSE => Ok((Self::Bool(false), src)),
            tag::TRUE => Ok((Self::Bool(true), src)),
            tag::STRING | tag::BYTES => {
                let (length, read) = varint::uvarint(src)?;
                let src = &src[read..];
                let length = usize::try_from(length).map_err(|_| DecodeError::Truncated)?;
                if src.len() < length {
                    return Err(DecodeError::Truncated);
                }
                let (payload, rest) = src.split_at(length);
                if tag == tag::STRING {
                    let value =
                        core::str::from_utf8(payload).map_err(|_| DecodeError::InvalidUtf8)?;
                    Ok((Self::String(value.to_owned()), rest))
                } else {
                    Ok((Self::Bytes(payload.to_vec()), rest))
                }
            }
            tag::INT => {
                let (value, read) = varint::varint(src)?;
                Ok((Self::Int(value), &src[read..]))
            }
            tag::UINT => {
                let (value, read) = varint::uvarint(src)?;
                Ok((Self::Uint(value), &src[read..]))
            }
            tag::FLOAT => {
                let bits: [u8; 8] = src
                    .get(..8)
                    .ok_or(DecodeError::Truncated)?
                    .try_into()
                    .expect("an 8-byte slice is an 8-byte array");
                Ok((
                    Self::Float(f64::from_bits(u64::from_le_bytes(bits))),
                    &src[8..],
                ))
            }
            tag::TIMESTAMP => {
                let (seconds, read) = varint::varint(src)?;
                let src = &src[read..];
                let (nanos, read) = varint::varint(src)?;
                let nanos = i32::try_from(nanos).map_err(|_| DecodeError::NanosOutOfRange)?;
                Ok((Self::timestamp(seconds, nanos), &src[read..]))
            }
            tag::DURATION => {
                let (nanos, read) = varint::varint(src)?;
                Ok((Self::Duration { nanos }, &src[read..]))
            }
            unknown => Err(DecodeError::UnknownTag(unknown)),
        }
    }
}

/// Carries excess nanoseconds into seconds, so `nanos` lands in
/// `0..1_000_000_000` as `google.protobuf.Timestamp` requires.
fn normalize_timestamp(seconds: i64, nanos: i32) -> (i64, i32) {
    let carry = i64::from(nanos).div_euclid(NANOS_PER_SECOND);
    let nanos = i64::from(nanos).rem_euclid(NANOS_PER_SECOND);
    (seconds.saturating_add(carry), nanos as i32)
}

impl From<bool> for CursorValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<String> for CursorValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for CursorValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<Vec<u8>> for CursorValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for CursorValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<T: Into<CursorValue>> From<Option<T>> for CursorValue {
    /// `None` — an unset field — becomes [`CursorValue::Null`].
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

/// Widens the sized integer and float types onto the four the wire format
/// carries, matching the Go implementation's type table.
macro_rules! from_widening {
    ($($source:ty => $variant:ident as $target:ty),* $(,)?) => {
        $(
            impl From<$source> for CursorValue {
                fn from(value: $source) -> Self {
                    Self::$variant(<$target>::from(value))
                }
            }
        )*
    };
}

from_widening! {
    i8 => Int as i64,
    i16 => Int as i64,
    i32 => Int as i64,
    i64 => Int as i64,
    u8 => Uint as u64,
    u16 => Uint as u64,
    u32 => Uint as u64,
    u64 => Uint as u64,
    f32 => Float as f64,
    f64 => Float as f64,
}

/// The opaque state a server hands a client so the client can ask for the next
/// page of a List call.
///
/// A token supports both AIP-158 pagination styles, and which field is
/// meaningful depends on how the page was served:
///
/// - [`offset`](Self::offset) counts rows already returned. Advance it with
///   [`next_offset`](Self::next_offset). Simple, but the cost of skipping rows
///   grows with the page number, and concurrent writes shift rows across page
///   boundaries.
/// - [`cursor`](Self::cursor) holds the sort-key values of the last row
///   returned. Advance it with [`next_cursor`](Self::next_cursor). Cost is
///   constant per page and results stay stable under concurrent writes, at the
///   price of requiring an `order_by` whose trailing field is unique.
///
/// Use one or the other; a token carrying both is not itself invalid, but no
/// query should consult both.
///
/// Every token carries a checksum of the request fields that must not change
/// between pages, so a client cannot page with one filter and then swap in
/// another. Mismatches surface from [`parse`](Self::parse) as
/// [`ParseError::ChecksumMismatch`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageToken {
    /// The number of rows preceding this page.
    pub offset: i64,
    /// The sort-key tuple of the last row of the previous page.
    ///
    /// A cursor is only meaningful alongside the `order_by` that produced it:
    /// the i'th value is the i'th ordering field's value on the last row, so
    /// its length must equal the number of ordering fields. Serving the next
    /// page means asking for rows that sort strictly after the tuple.
    pub cursor: Vec<CursorValue>,
    /// The checksum of the request that produced this token, as returned by
    /// [`request_checksum`].
    pub request_checksum: u32,
}

impl PageToken {
    /// Decodes and validates `token` against `checksum`, the checksum of the
    /// request being served.
    ///
    /// An empty `token` — the first page — yields the zero-offset token
    /// carrying `checksum`, so the result is always safe to advance and hand
    /// back to the client.
    ///
    /// `skip` (AIP-158's "skipping results") is not handled here, because
    /// reading it needs the request message this crate deliberately does not
    /// depend on. Add it to [`offset`](Self::offset) at the call site:
    ///
    /// ```
    /// # use aip::PageToken;
    /// # let (encoded, checksum, skip) = ("", 0u32, 25i64);
    /// let mut token = PageToken::parse(encoded, checksum)?;
    /// token.offset += skip;
    /// # Ok::<_, aip::pagination::ParseError>(())
    /// ```
    ///
    /// Both error variants map to `InvalidArgument` at the RPC boundary, but
    /// they are worth distinguishing in logs: a mismatch means the client
    /// changed `filter` or `order_by` mid-page, while a malformed token means
    /// it sent something that was never a token at all.
    pub fn parse(token: &str, checksum: u32) -> Result<Self, ParseError> {
        if token.is_empty() {
            return Ok(Self {
                request_checksum: checksum,
                ..Self::default()
            });
        }
        let token = Self::decode(token)?;
        if token.request_checksum != checksum {
            return Err(ParseError::ChecksumMismatch {
                token: token.request_checksum,
                request: checksum,
            });
        }
        Ok(token)
    }

    /// Returns the opaque string form of the token, to be returned to the
    /// client as `next_page_token`.
    ///
    /// This cannot fail. The Go predecessor's encoder could, because its
    /// cursor held `any` and a value it could not represent produced a
    /// silently truncated token that failed to decode a round-trip later, far
    /// from the cause. [`CursorValue`] makes that value unrepresentable, which
    /// removes the failure rather than reporting it.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut buffer = Vec::with_capacity(32);
        buffer.push(VERSION);
        varint::put_varint(&mut buffer, self.offset);
        buffer.extend_from_slice(&self.request_checksum.to_le_bytes());
        varint::put_uvarint(&mut buffer, self.cursor.len() as u64);
        for value in &self.cursor {
            value.encode_into(&mut buffer);
        }
        base64url::encode(&buffer)
    }

    /// Decodes an encoded page token.
    ///
    /// This does not check the token against a request; [`parse`](Self::parse)
    /// does that.
    pub fn decode(token: &str) -> Result<Self, DecodeError> {
        let raw = base64url::decode(token)?;
        let (&version, raw) = raw.split_first().ok_or(DecodeError::Truncated)?;
        if version != VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let (offset, read) = varint::varint(raw)?;
        let raw = &raw[read..];
        let checksum: [u8; 4] = raw
            .get(..4)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .expect("a 4-byte slice is a 4-byte array");
        let mut raw = &raw[4..];
        let (length, read) = varint::uvarint(raw)?;
        raw = &raw[read..];
        // Guard against a hostile length driving a huge allocation: every
        // cursor value costs at least one byte on the wire, so a length past
        // the remaining bytes cannot be honest. Narrowing to usize first also
        // makes the guard hold on a 32-bit target, where a u64 length can
        // exceed anything addressable.
        let length = usize::try_from(length).map_err(|_| DecodeError::Truncated)?;
        if length > raw.len() {
            return Err(DecodeError::Truncated);
        }
        let mut cursor = Vec::with_capacity(length);
        for _ in 0..length {
            let (value, rest) = CursorValue::decode_from(raw)?;
            cursor.push(value);
            raw = rest;
        }
        if !raw.is_empty() {
            return Err(DecodeError::TrailingBytes(raw.len()));
        }
        Ok(Self {
            offset,
            cursor,
            request_checksum: u32::from_le_bytes(checksum),
        })
    }

    /// Returns the token for the page following this one, under offset
    /// pagination, by advancing the offset past the page just served.
    ///
    /// Pair it with [`offset`](Self::offset); the counterpart for key-set
    /// pagination is [`next_cursor`](Self::next_cursor). Advancing the wrong
    /// one yields a token the query layer cannot serve correctly, so choose
    /// per List method and stay with it.
    #[must_use]
    pub fn next_offset(&self, page_size: i32) -> Self {
        Self {
            offset: self.offset + i64::from(page_size),
            ..self.clone()
        }
    }

    /// Returns the token for the page following this one, under key-set
    /// pagination, from the sort-key values of the last row served.
    ///
    /// `cursor` must line up positionally with the ordering fields of the
    /// request — [`OrderBy::paths`](crate::OrderBy::paths) returns them in the
    /// right order.
    ///
    /// An empty `cursor` is an error rather than an empty tuple: without a
    /// sort key there is nothing to seek on, and the resulting token could not
    /// page. When a request carries no `order_by`, use
    /// [`next_offset`](Self::next_offset) instead.
    pub fn next_cursor(
        &self,
        cursor: impl Into<Vec<CursorValue>>,
    ) -> Result<Self, EmptyCursorError> {
        let cursor = cursor.into();
        if cursor.is_empty() {
            return Err(EmptyCursorError);
        }
        Ok(Self {
            cursor,
            ..self.clone()
        })
    }
}

impl fmt::Display for PageToken {
    /// Writes the encoded form; see [`PageToken::encode`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl FromStr for PageToken {
    type Err = DecodeError;

    /// Decodes the token; see [`PageToken::decode`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::decode(s)
    }
}

/// Returns the checksum to carry in a page token issued for the request whose
/// deterministic protobuf encoding is `marshalled`.
///
/// Two things the caller must get right, both of which were bugs before they
/// were requirements:
///
/// - **`marshalled` must come from a deterministic encoder.** Protobuf map
///   field ordering is explicitly unspecified, so a non-deterministic encoder
///   makes the checksum of a request carrying a map flap between calls, which
///   rejects valid page tokens at random.
/// - **`page_token`, `page_size` and `skip` must be cleared first.** They are
///   expected to change from one page to the next, so including them would
///   invalidate every token as soon as it was used.
///
/// Clearing fields and encoding deterministically both need to walk the
/// request message, which is why they are the caller's job here and generated
/// code's job in practice — see the scope note in the README. This function is
/// the part that does not: CRC-32 (IEEE) over the bytes, XORed with
/// [`CHECKSUM_MASK`] so that a token issued by a different token type over the
/// same request does not validate here.
#[must_use]
pub fn request_checksum(marshalled: &[u8]) -> u32 {
    crc32::checksum_ieee(marshalled) ^ CHECKSUM_MASK
}

/// Why an encoded page token could not be decoded at all.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// A character outside the unpadded base64url alphabet, at this byte
    /// index. `=` is among them: the encoding is unpadded.
    Base64InvalidByte {
        /// The index of the offending character.
        index: usize,
        /// The offending byte.
        byte: u8,
    },
    /// A length no unpadded base64url string can have.
    Base64InvalidLength(usize),
    /// Valid base64url characters, but the unused low bits of the final one
    /// are not zero, so this is not the canonical spelling of any token.
    Base64NonCanonical,
    /// The token ended in the middle of a field.
    Truncated,
    /// A version byte this implementation does not write. `0x01` means a token
    /// issued by the Go implementation, which is not compatible — see
    /// [`VERSION`].
    UnsupportedVersion(u8),
    /// A cursor value tag from a future version of the format.
    UnknownTag(u8),
    /// A varint encoding a value wider than 64 bits.
    VarintOverflow,
    /// A string cursor value whose payload is not UTF-8.
    ///
    /// The Go implementation accepts this, because a Go `string` may hold
    /// arbitrary bytes; a Rust [`String`] may not. Encode non-UTF-8 sort keys
    /// as [`CursorValue::Bytes`].
    InvalidUtf8,
    /// A timestamp whose nanoseconds field does not fit in an [`i32`].
    NanosOutOfRange,
    /// Bytes after the last cursor value.
    TrailingBytes(usize),
}

impl From<base64url::Error> for DecodeError {
    fn from(error: base64url::Error) -> Self {
        match error {
            base64url::Error::InvalidByte { index, byte } => {
                Self::Base64InvalidByte { index, byte }
            }
            base64url::Error::InvalidLength(length) => Self::Base64InvalidLength(length),
            base64url::Error::NonCanonical => Self::Base64NonCanonical,
        }
    }
}

impl From<varint::Error> for DecodeError {
    fn from(error: varint::Error) -> Self {
        match error {
            varint::Error::Truncated => Self::Truncated,
            varint::Error::Overflow => Self::VarintOverflow,
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("malformed page token: ")?;
        match self {
            Self::Base64InvalidByte { index, byte } => {
                write!(f, "invalid base64 byte {byte:#04x} at index {index}")
            }
            Self::Base64InvalidLength(length) => {
                write!(f, "invalid base64 length {length}")
            }
            Self::Base64NonCanonical => f.write_str("non-canonical base64"),
            Self::Truncated => f.write_str("truncated"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported version {version:#04x}")
            }
            Self::UnknownTag(tag) => write!(f, "unknown cursor value tag {tag:#04x}"),
            Self::VarintOverflow => f.write_str("varint overflows 64 bits"),
            Self::InvalidUtf8 => f.write_str("string cursor value is not UTF-8"),
            Self::NanosOutOfRange => f.write_str("timestamp nanoseconds out of range"),
            Self::TrailingBytes(count) => write!(f, "{count} trailing bytes"),
        }
    }
}

impl core::error::Error for DecodeError {}

/// Why a page token could not be used for the request it arrived on.
///
/// Both variants map to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The token could not be decoded at all.
    Malformed(DecodeError),
    /// The token was issued for a materially different request — typically a
    /// client that changed `filter` or `order_by` while paging.
    ChecksumMismatch {
        /// The checksum the token carries.
        token: u32,
        /// The checksum of the request it arrived on.
        request: u32,
    },
}

impl From<DecodeError> for ParseError {
    fn from(error: DecodeError) -> Self {
        Self::Malformed(error)
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => error.fmt(f),
            Self::ChecksumMismatch { token, request } => write!(
                f,
                "page token does not match the request (token {token:#010x}, request {request:#010x})"
            ),
        }
    }
}

impl core::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Malformed(error) => Some(error),
            Self::ChecksumMismatch { .. } => None,
        }
    }
}

/// Returned by [`PageToken::next_cursor`] when handed no sort-key values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyCursorError;

impl fmt::Display for EmptyCursorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("next cursor: no ordering paths")
    }
}

impl core::error::Error for EmptyCursorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_cursor_value_kind() {
        let token = PageToken {
            offset: 3,
            request_checksum: 0x0102_0304,
            cursor: vec![
                CursorValue::Null,
                CursorValue::Bool(true),
                CursorValue::Bool(false),
                CursorValue::from("hi"),
                CursorValue::from(vec![0x01, 0xff]),
                CursorValue::from(-2i32),
                CursorValue::from(7u8),
                CursorValue::from(1.5f32),
                CursorValue::timestamp(1_700_000_000, 123_456_789),
                CursorValue::duration_nanos(5_400_000_000_000),
            ],
        };
        assert_eq!(PageToken::decode(&token.encode()), Ok(token));
    }

    #[test]
    fn widens_sized_integers_and_floats() {
        assert_eq!(CursorValue::from(7i8), CursorValue::Int(7));
        assert_eq!(CursorValue::from(7u16), CursorValue::Uint(7));
        assert_eq!(CursorValue::from(0.5f32), CursorValue::Float(0.5));
        assert_eq!(CursorValue::from(Some(1u8)), CursorValue::Uint(1));
        assert_eq!(CursorValue::from(None::<u8>), CursorValue::Null);
    }

    #[test]
    fn normalizes_timestamp_nanoseconds() {
        assert_eq!(
            CursorValue::timestamp(10, 1_500_000_000),
            CursorValue::Timestamp {
                seconds: 11,
                nanos: 500_000_000
            }
        );
        assert_eq!(
            CursorValue::timestamp(10, -1),
            CursorValue::Timestamp {
                seconds: 9,
                nanos: 999_999_999
            }
        );
    }

    #[test]
    fn parse_of_an_empty_token_is_the_first_page() {
        assert_eq!(
            PageToken::parse("", 0xdead_beef),
            Ok(PageToken {
                offset: 0,
                cursor: Vec::new(),
                request_checksum: 0xdead_beef,
            })
        );
    }

    #[test]
    fn parse_rejects_a_token_from_another_request() {
        let encoded = PageToken {
            offset: 10,
            request_checksum: 0xdead_beef,
            ..PageToken::default()
        }
        .encode();
        assert_eq!(
            PageToken::parse(&encoded, 0x0102_0304),
            Err(ParseError::ChecksumMismatch {
                token: 0xdead_beef,
                request: 0x0102_0304,
            })
        );
    }

    #[test]
    fn rejects_a_go_page_token() {
        // The Go implementation's empty-token vector, version byte 0x01.
        assert_eq!(
            PageToken::decode("AQAAAAAAAA"),
            Err(DecodeError::UnsupportedVersion(0x01))
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut raw = base64url::decode(&PageToken::default().encode()).unwrap();
        raw.push(0x00);
        assert_eq!(
            PageToken::decode(&base64url::encode(&raw)),
            Err(DecodeError::TrailingBytes(1))
        );
    }

    #[test]
    fn rejects_an_unknown_cursor_tag() {
        let mut raw = base64url::decode(&PageToken::default().encode()).unwrap();
        *raw.last_mut().unwrap() = 1; // one cursor value...
        raw.push(0x7f); // ...with a tag from a format that does not exist.
        assert_eq!(
            PageToken::decode(&base64url::encode(&raw)),
            Err(DecodeError::UnknownTag(0x7f))
        );
    }

    #[test]
    fn rejects_a_hostile_cursor_length() {
        let mut raw = base64url::decode(&PageToken::default().encode()).unwrap();
        raw.pop();
        varint::put_uvarint(&mut raw, u64::MAX);
        assert_eq!(
            PageToken::decode(&base64url::encode(&raw)),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn rejects_a_non_utf8_string_value() {
        let raw = [
            VERSION,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x01,
            tag::STRING,
            0x01,
            0xff,
        ];
        assert_eq!(
            PageToken::decode(&base64url::encode(&raw)),
            Err(DecodeError::InvalidUtf8)
        );
    }

    #[test]
    fn next_offset_advances_by_the_page_size() {
        let token = PageToken {
            offset: 20,
            request_checksum: 7,
            ..PageToken::default()
        };
        assert_eq!(token.next_offset(25).offset, 45);
        assert_eq!(token.next_offset(25).request_checksum, 7);
    }

    #[test]
    fn next_cursor_rejects_an_empty_tuple() {
        assert_eq!(
            PageToken::default().next_cursor(Vec::new()),
            Err(EmptyCursorError)
        );
    }

    #[test]
    fn next_cursor_keeps_the_checksum() {
        let token = PageToken {
            request_checksum: 0xdead_beef,
            ..PageToken::default()
        };
        let next = token.next_cursor(vec![CursorValue::from("x")]).unwrap();
        assert_eq!(next.request_checksum, 0xdead_beef);
        assert_eq!(next.cursor, vec![CursorValue::from("x")]);
    }

    #[test]
    fn request_checksum_is_masked() {
        assert_eq!(request_checksum(b""), CHECKSUM_MASK);
        assert_ne!(request_checksum(b"a"), request_checksum(b"b"));
    }
}
