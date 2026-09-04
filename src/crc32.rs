//! CRC-32 (IEEE), the hash behind the page token's request checksum.
//!
//! Equivalent to Go's `hash/crc32.ChecksumIEEE`. Hand-rolled to keep the crate
//! dependency-free: this is the reflected polynomial 0xedb88320 with the
//! standard pre- and post-inversion, and it is fifteen lines.

/// One byte's worth of the reflected IEEE polynomial, precomputed at compile
/// time.
const TABLE: [u32; 256] = {
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
pub(crate) fn checksum_ieee(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc = crc >> 8 ^ TABLE[((crc ^ u32::from(byte)) & 0xff) as usize];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::checksum_ieee;

    #[test]
    fn matches_the_published_check_values() {
        assert_eq!(checksum_ieee(b""), 0x0000_0000);
        assert_eq!(checksum_ieee(b"a"), 0xe8b7_be43);
        // The CRC catalogue's check value: the CRC-32 of "123456789".
        assert_eq!(checksum_ieee(b"123456789"), 0xcbf4_3926);
        assert_eq!(
            checksum_ieee(b"The quick brown fox jumps over the lazy dog"),
            0x414f_a339
        );
    }
}
