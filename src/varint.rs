//! LEB128 varints, in the two flavours the page token wire format uses.
//!
//! These match Go's `encoding/binary` byte for byte — `PutUvarint`/`Uvarint`
//! for the unsigned form and `PutVarint`/`Varint` for the zigzag-signed form —
//! because the wire format in `docs/page-token.md` was specified against them.
//!
//! Hand-rolled rather than pulled from a crate: the `leb128` crates on
//! crates.io implement DWARF's sign-extended signed encoding, which is *not*
//! zigzag and would silently produce different bytes for negative offsets.

/// Why a varint could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    /// The buffer ended in the middle of the varint.
    Truncated,
    /// The varint encodes a value wider than 64 bits.
    Overflow,
}

/// Appends `value` to `dst` as an unsigned LEB128 varint.
pub(crate) fn put_uvarint(dst: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        dst.push(value as u8 | 0x80);
        value >>= 7;
    }
    dst.push(value as u8);
}

/// Appends `value` to `dst` as a zigzag-encoded signed LEB128 varint.
pub(crate) fn put_varint(dst: &mut Vec<u8>, value: i64) {
    // Shift as u64 and fold in the sign via an arithmetic shift, so that
    // `i64::MIN` does not overflow the shift the way `value << 1` would.
    put_uvarint(dst, ((value as u64) << 1) ^ ((value >> 63) as u64));
}

/// Reads an unsigned LEB128 varint from the front of `src`, returning it with
/// the number of bytes consumed.
pub(crate) fn uvarint(src: &[u8]) -> Result<(u64, usize), Error> {
    let mut value: u64 = 0;
    for (i, &byte) in src.iter().enumerate() {
        // The tenth byte contributes bit 63 and nothing above it, so anything
        // but 0 or 1 there is a value too wide for u64.
        if i == 9 {
            if byte > 1 {
                return Err(Error::Overflow);
            }
            return Ok((value | (u64::from(byte) << 63), 10));
        }
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte < 0x80 {
            return Ok((value, i + 1));
        }
    }
    Err(Error::Truncated)
}

/// Reads a zigzag-encoded signed LEB128 varint from the front of `src`,
/// returning it with the number of bytes consumed.
pub(crate) fn varint(src: &[u8]) -> Result<(i64, usize), Error> {
    let (encoded, n) = uvarint(src)?;
    let value = (encoded >> 1) as i64;
    Ok((if encoded & 1 != 0 { !value } else { value }, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_signed(value: i64) {
        let mut buffer = Vec::new();
        put_varint(&mut buffer, value);
        assert_eq!(varint(&buffer), Ok((value, buffer.len())), "{value}");
    }

    fn round_trip_unsigned(value: u64) {
        let mut buffer = Vec::new();
        put_uvarint(&mut buffer, value);
        assert_eq!(uvarint(&buffer), Ok((value, buffer.len())), "{value}");
    }

    #[test]
    fn round_trips_signed_edges() {
        for value in [0, 1, -1, 2, -2, 100, -100, i64::MAX, i64::MIN] {
            round_trip_signed(value);
        }
    }

    #[test]
    fn round_trips_unsigned_edges() {
        for value in [0, 1, 127, 128, u64::MAX] {
            round_trip_unsigned(value);
        }
    }

    #[test]
    fn matches_go_encoding() {
        // Byte sequences lifted from the test vectors in docs/page-token.md.
        let mut buffer = Vec::new();
        put_varint(&mut buffer, 100);
        assert_eq!(buffer, [0xc8, 0x01]);

        buffer.clear();
        put_varint(&mut buffer, -2);
        assert_eq!(buffer, [0x03]);

        buffer.clear();
        put_varint(&mut buffer, 1_700_000_000);
        assert_eq!(buffer, [0x80, 0xc4, 0x9f, 0xd5, 0x0c]);

        buffer.clear();
        put_uvarint(&mut buffer, 7);
        assert_eq!(buffer, [0x07]);
    }

    #[test]
    fn rejects_truncated() {
        assert_eq!(uvarint(&[]), Err(Error::Truncated));
        assert_eq!(uvarint(&[0x80]), Err(Error::Truncated));
        assert_eq!(uvarint(&[0x80; 9]), Err(Error::Truncated));
    }

    #[test]
    fn rejects_overflow() {
        // Ten continuation bytes: the tenth carries more than bit 63.
        assert_eq!(uvarint(&[0x80; 10]), Err(Error::Overflow));
        assert_eq!(
            uvarint(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02]),
            Err(Error::Overflow)
        );
        // ...but exactly bit 63 is fine: this is u64::MAX.
        assert_eq!(
            uvarint(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]),
            Ok((u64::MAX, 10))
        );
    }
}
