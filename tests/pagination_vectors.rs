//! The page token test vectors from `docs/page-token.md`.
//!
//! These are the Go implementation's vectors with the version byte changed
//! from `0x01` to `0x02`; everything after that byte is identical. That is the
//! whole point of keeping them: the checksums differ between implementations
//! and the cursor bytes do not, so the vectors still pin down varint, tag and
//! base64 handling exactly.
//!
//! A conforming encoder must reproduce these and a conforming decoder must
//! accept them.

use aip::pagination::CHECKSUM_MASK;
use aip::{CursorValue, PageToken};

fn check(token: PageToken, encoded: &str) {
    assert_eq!(token.encode(), encoded, "encoding {token:?}");
    assert_eq!(
        PageToken::decode(encoded),
        Ok(token),
        "decoding {encoded:?}"
    );
}

#[test]
fn empty() {
    check(PageToken::default(), "AgAAAAAAAA");
}

#[test]
fn offset_only() {
    check(
        PageToken {
            offset: 100,
            request_checksum: 0xdead_beef,
            ..PageToken::default()
        },
        "AsgB776t3gA",
    );
}

#[test]
fn cursor_of_strings() {
    check(
        PageToken {
            offset: 0,
            request_checksum: 0xdead_beef,
            cursor: vec!["Alice".into(), "uuid-7".into()],
        },
        "AgDvvq3eAgMFQWxpY2UDBnV1aWQtNw",
    );
}

#[test]
fn every_cursor_value_tag() {
    check(
        PageToken {
            offset: 3,
            request_checksum: 0x0102_0304,
            cursor: vec![
                CursorValue::Null,
                true.into(),
                false.into(),
                "hi".into(),
                vec![0x01u8, 0xff].into(),
                (-2i64).into(),
                7u64.into(),
                1.5f64.into(),
                // 2023-11-14T22:13:20.123456789Z
                CursorValue::timestamp(1_700_000_000, 123_456_789),
                // 90 minutes
                CursorValue::duration_nanos(90 * 60 * 1_000_000_000),
            ],
        },
        "AgYEAwIBCgACAQMCaGkEAgH_BQMGBwcAAAAAAAD4PwiAxJ_VDKq03nUJgMCnkam6Ag",
    );
}

/// The version byte is the whole reason a Go token fails fast here rather than
/// decoding structurally and then failing its checksum.
#[test]
fn go_tokens_are_rejected_on_the_version_byte() {
    for go_vector in [
        "AQAAAAAAAA",
        "AcgB776t3gA",
        "AQDvvq3eAgMFQWxpY2UDBnV1aWQtNw",
        "AQYEAwIBCgACAQMCaGkEAgH_BQMGBwcAAAAAAAD4PwiAxJ_VDKq03nUJgMCnkam6Ag",
    ] {
        assert_eq!(
            PageToken::decode(go_vector),
            Err(aip::pagination::DecodeError::UnsupportedVersion(0x01)),
            "{go_vector}"
        );
    }
}

/// The mask is what keeps a token issued by a different token type over the
/// same request from validating here.
#[test]
fn the_request_checksum_is_the_masked_crc32() {
    // CRC-32(IEEE) of "123456789" is the CRC catalogue's check value.
    assert_eq!(
        aip::pagination::request_checksum(b"123456789"),
        0xcbf4_3926 ^ CHECKSUM_MASK
    );
}
