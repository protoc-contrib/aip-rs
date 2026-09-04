//! Unpadded base64url, the outer encoding of a page token.
//!
//! Equivalent to Go's `base64.RawURLEncoding` on the encoding side. The
//! decoder is deliberately stricter: see [`decode`].

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Why a string is not valid unpadded base64url.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
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
pub(crate) fn encode(src: &[u8]) -> String {
    let mut out = String::with_capacity((src.len() * 4).div_ceil(3));
    for chunk in src.chunks(3) {
        let packed = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        // A 1-byte tail spells 2 characters and a 2-byte tail spells 3; the
        // padding a full quantum would need is simply not written.
        for i in 0..(chunk.len() * 8).div_ceil(6) {
            out.push(ALPHABET[(packed >> (18 - 6 * i) & 0x3f) as usize] as char);
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
pub(crate) fn decode(src: &str) -> Result<Vec<u8>, Error> {
    let src = src.as_bytes();
    if src.len() % 4 == 1 {
        return Err(Error::InvalidLength(src.len()));
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
            _ => return Err(Error::InvalidByte { index, byte }),
        };
        accumulator = accumulator << 6 | u32::from(six);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    if accumulator & ((1 << bits) - 1) != 0 {
        return Err(Error::NonCanonical);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_go_raw_url_encoding() {
        // The empty-token vector from docs/page-token.md, with the Rust
        // version byte.
        assert_eq!(encode(&[0x02, 0, 0, 0, 0, 0, 0]), "AgAAAAAAAA");
        assert_eq!(decode("AgAAAAAAAA"), Ok(vec![0x02, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn round_trips_every_tail_length() {
        for length in 0..16 {
            let bytes: Vec<u8> = (0..length).map(|i: u8| i.wrapping_mul(37)).collect();
            assert_eq!(decode(&encode(&bytes)), Ok(bytes.clone()), "{length}");
        }
    }

    #[test]
    fn uses_the_url_safe_alphabet() {
        // 0xfb 0xff would be "+/" in standard base64.
        assert_eq!(encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(decode("-_8"), Ok(vec![0xfb, 0xff]));
    }

    #[test]
    fn rejects_padding() {
        assert_eq!(
            decode("AgAAAAAAAA=="),
            Err(Error::InvalidByte {
                index: 10,
                byte: b'='
            })
        );
    }

    #[test]
    fn rejects_impossible_length() {
        assert_eq!(decode("AAAAA"), Err(Error::InvalidLength(5)));
    }

    #[test]
    fn rejects_non_canonical_trailing_bits() {
        // "AB" carries 12 bits for one byte; the low 4 must be zero, and "AB"
        // ends in 0b000001. "AA" is the canonical spelling of [0x00].
        assert_eq!(decode("AA"), Ok(vec![0x00]));
        assert_eq!(decode("AB"), Err(Error::NonCanonical));
    }
}
