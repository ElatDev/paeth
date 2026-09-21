//! CRC-32 as PNG uses it: ISO 3309 / ITU-T V.42, reflected polynomial
//! `0xEDB88320`, register preset to all ones and inverted at the end
//! (PNG spec, Annex D).

/// One entry per possible byte value, computed at compile time.
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

/// A running CRC-32, for checksumming a chunk's type and data without
/// copying them into one buffer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Crc32(u32);

impl Crc32 {
    pub(crate) fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }

    pub(crate) fn update(mut self, bytes: &[u8]) -> Self {
        for &b in bytes {
            self.0 = TABLE[((self.0 ^ u32::from(b)) & 0xFF) as usize] ^ (self.0 >> 8);
        }
        self
    }

    pub(crate) fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

/// CRC-32 of a byte slice in one call.
#[cfg(test)]
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    Crc32::new().update(bytes).finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_check_value() {
        // The "check" value every CRC-32 catalogue lists for this polynomial.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn empty_input() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn iend_chunk() {
        // Every PNG ends with IEND, whose CRC (type only, no data) is
        // the familiar AE 42 60 82.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data = b"IHDR\x00\x00\x00\x20\x00\x00\x00\x20\x08\x06\x00\x00\x00";
        for split in 0..=data.len() {
            let (a, b) = data.split_at(split);
            assert_eq!(Crc32::new().update(a).update(b).finish(), crc32(data));
        }
    }
}
