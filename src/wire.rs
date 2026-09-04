//! The byte-level primitives the page token wire format is built from.
//!
//! Three unrelated things live here — zigzag LEB128 varints, unpadded
//! base64url and CRC-32 — grouped not because they share a category but
//! because they share a reason: `docs/page-token.md` specifies the token
//! against Go's `encoding/binary`, `encoding/base64` and `hash/crc32`, and
//! this crate reproduces them rather than take a dependency.
//!
//! Nothing here is public. Names are prefixed by which primitive they belong
//! to, since a flat `encode`/`decode` in a module this broad would say
//! nothing.

// ---------------------------------------------------------------------------
// LEB128 varints
// ---------------------------------------------------------------------------

/// Why a varint could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VarintError {
    /// The buffer ended in the middle of the varint.
    Truncated,
    /// The varint encodes a value wider than 64 bits.
    Overflow,
}

/// Appends `value` to `dst` as an unsigned LEB128 varint.
///
/// Go's `binary.PutUvarint`.
pub(crate) fn write_uvarint(dst: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        dst.push(value as u8 | 0x80);
        value >>= 7;
    }
    dst.push(value as u8);
}

/// Appends `value` to `dst` as a zigzag-encoded signed LEB128 varint.
///
/// Go's `binary.PutVarint`. Hand-rolled rather than pulled from a crate: the
/// `leb128` crates on crates.io implement DWARF's sign-extended signed
/// encoding, which is *not* zigzag and would silently produce different bytes
/// for a negative offset.
pub(crate) fn write_varint(dst: &mut Vec<u8>, value: i64) {
    // Shift as u64 and fold in the sign via an arithmetic shift, so that
    // `i64::MIN` does not overflow the shift the way `value << 1` would.
    write_uvarint(dst, ((value as u64) << 1) ^ ((value >> 63) as u64));
}

/// Reads an unsigned LEB128 varint from the front of `src`, returning it with
/// the number of bytes consumed.
///
/// Go's `binary.Uvarint`.
pub(crate) fn read_uvarint(src: &[u8]) -> Result<(u64, usize), VarintError> {
    let mut value: u64 = 0;
    for (i, &byte) in src.iter().enumerate() {
        // The tenth byte contributes bit 63 and nothing above it, so anything
        // but 0 or 1 there is a value too wide for u64.
        if i == 9 {
            if byte > 1 {
                return Err(VarintError::Overflow);
            }
            return Ok((value | (u64::from(byte) << 63), 10));
        }
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte < 0x80 {
            return Ok((value, i + 1));
        }
    }
    Err(VarintError::Truncated)
}

/// Reads a zigzag-encoded signed LEB128 varint from the front of `src`,
/// returning it with the number of bytes consumed.
///
/// Go's `binary.Varint`.
pub(crate) fn read_varint(src: &[u8]) -> Result<(i64, usize), VarintError> {
    let (encoded, n) = read_uvarint(src)?;
    let value = (encoded >> 1) as i64;
    Ok((if encoded & 1 != 0 { !value } else { value }, n))
}

#[cfg(test)]
mod varint_tests {
    use super::*;

    fn round_trip_signed(value: i64) {
        let mut buffer = Vec::new();
        write_varint(&mut buffer, value);
        assert_eq!(read_varint(&buffer), Ok((value, buffer.len())), "{value}");
    }

    fn round_trip_unsigned(value: u64) {
        let mut buffer = Vec::new();
        write_uvarint(&mut buffer, value);
        assert_eq!(read_uvarint(&buffer), Ok((value, buffer.len())), "{value}");
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
        write_varint(&mut buffer, 100);
        assert_eq!(buffer, [0xc8, 0x01]);

        buffer.clear();
        write_varint(&mut buffer, -2);
        assert_eq!(buffer, [0x03]);

        buffer.clear();
        write_varint(&mut buffer, 1_700_000_000);
        assert_eq!(buffer, [0x80, 0xc4, 0x9f, 0xd5, 0x0c]);

        buffer.clear();
        write_uvarint(&mut buffer, 7);
        assert_eq!(buffer, [0x07]);
    }

    #[test]
    fn rejects_truncated() {
        assert_eq!(read_uvarint(&[]), Err(VarintError::Truncated));
        assert_eq!(read_uvarint(&[0x80]), Err(VarintError::Truncated));
        assert_eq!(read_uvarint(&[0x80; 9]), Err(VarintError::Truncated));
    }

    #[test]
    fn rejects_overflow() {
        // Ten continuation bytes: the tenth carries more than bit 63.
        assert_eq!(read_uvarint(&[0x80; 10]), Err(VarintError::Overflow));
        assert_eq!(
            read_uvarint(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02]),
            Err(VarintError::Overflow)
        );
        // ...but exactly bit 63 is fine: this is u64::MAX.
        assert_eq!(
            read_uvarint(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]),
            Ok((u64::MAX, 10))
        );
    }
}

// ---------------------------------------------------------------------------
// Unpadded base64url
// ---------------------------------------------------------------------------

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Why a string is not valid unpadded base64url.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Base64Error {
    /// A character outside the base64url alphabet, at this byte index. `=` is
    /// among them: this encoding is unpadded.
    InvalidByte { index: usize, byte: u8 },
    /// A length that no unpadded base64url string can have. Whole input
    /// lengths of 4n+1 are impossible, since one character carries 6 bits and
    /// a byte needs 8.
    InvalidLength(usize),
    /// The bits left over in the final character are not zero.
    NonCanonical,
}

/// Encodes `src` as unpadded base64url.
///
/// Go's `base64.RawURLEncoding`.
pub(crate) fn base64_encode(src: &[u8]) -> String {
    let mut out = String::with_capacity((src.len() * 4).div_ceil(3));
    for chunk in src.chunks(3) {
        let packed = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        // A 1-byte tail spells 2 characters and a 2-byte tail spells 3; the
        // padding a full quantum would need is simply not written.
        for i in 0..(chunk.len() * 8).div_ceil(6) {
            out.push(BASE64_ALPHABET[(packed >> (18 - 6 * i) & 0x3f) as usize] as char);
        }
    }
    out
}

/// Decodes unpadded base64url.
///
/// Unlike Go's decoder this rejects a final character whose unused low bits
/// are non-zero. Those bits are discarded on decode, so accepting them would
/// give a single page token several valid spellings — which in turn would make
/// a token usable as a cache key or a dedupe key only by accident.
pub(crate) fn base64_decode(src: &str) -> Result<Vec<u8>, Base64Error> {
    let src = src.as_bytes();
    if src.len() % 4 == 1 {
        return Err(Base64Error::InvalidLength(src.len()));
    }
    let mut out = Vec::with_capacity(src.len() * 3 / 4);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    for (index, &byte) in src.iter().enumerate() {
        let six = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(Base64Error::InvalidByte { index, byte }),
        };
        accumulator = accumulator << 6 | u32::from(six);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    if accumulator & ((1 << bits) - 1) != 0 {
        return Err(Base64Error::NonCanonical);
    }
    Ok(out)
}

#[cfg(test)]
mod base64_tests {
    use super::*;

    #[test]
    fn matches_go_raw_url_encoding() {
        // The empty-token vector from docs/page-token.md, with the Rust
        // version byte.
        assert_eq!(base64_encode(&[0x02, 0, 0, 0, 0, 0, 0]), "AgAAAAAAAA");
        assert_eq!(
            base64_decode("AgAAAAAAAA"),
            Ok(vec![0x02, 0, 0, 0, 0, 0, 0])
        );
    }

    #[test]
    fn round_trips_every_tail_length() {
        for length in 0..16 {
            let bytes: Vec<u8> = (0..length).map(|i: u8| i.wrapping_mul(37)).collect();
            assert_eq!(
                base64_decode(&base64_encode(&bytes)),
                Ok(bytes.clone()),
                "{length}"
            );
        }
    }

    #[test]
    fn uses_the_url_safe_alphabet() {
        // 0xfb 0xff would be "+/" in standard base64.
        assert_eq!(base64_encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64_decode("-_8"), Ok(vec![0xfb, 0xff]));
    }

    #[test]
    fn rejects_padding() {
        assert_eq!(
            base64_decode("AgAAAAAAAA=="),
            Err(Base64Error::InvalidByte {
                index: 10,
                byte: b'='
            })
        );
    }

    #[test]
    fn rejects_impossible_length() {
        assert_eq!(base64_decode("AAAAA"), Err(Base64Error::InvalidLength(5)));
    }

    #[test]
    fn rejects_non_canonical_trailing_bits() {
        // "AB" carries 12 bits for one byte; the low 4 must be zero, and "AB"
        // ends in 0b000001. "AA" is the canonical spelling of [0x00].
        assert_eq!(base64_decode("AA"), Ok(vec![0x00]));
        assert_eq!(base64_decode("AB"), Err(Base64Error::NonCanonical));
    }
}

// ---------------------------------------------------------------------------
// CRC-32 (IEEE)
// ---------------------------------------------------------------------------

/// One byte's worth of the reflected IEEE polynomial, precomputed at compile
/// time.
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                crc >> 1 ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

/// Returns the CRC-32 (IEEE) of `data`.
///
/// Go's `hash/crc32.ChecksumIEEE`: the reflected polynomial 0xedb88320 with
/// the standard pre- and post-inversion.
///
/// This is a checksum, not a cryptographic hash. It detects the accidental
/// mismatch the page token cares about — a client that changed `filter`
/// mid-page — and is trivially forgeable by anyone who wants to.
pub(crate) fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc = crc >> 8 ^ CRC32_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize];
    }
    !crc
}

#[cfg(test)]
mod crc32_tests {
    use super::crc32_ieee;

    #[test]
    fn matches_the_published_check_values() {
        assert_eq!(crc32_ieee(b""), 0x0000_0000);
        assert_eq!(crc32_ieee(b"a"), 0xe8b7_be43);
        // The CRC catalogue's check value: the CRC-32 of "123456789".
        assert_eq!(crc32_ieee(b"123456789"), 0xcbf4_3926);
        assert_eq!(
            crc32_ieee(b"The quick brown fox jumps over the lazy dog"),
            0x414f_a339
        );
    }
}
