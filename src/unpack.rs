//! Reading samples out of an unfiltered scanline (PNG spec, section 7.2).
//!
//! Depths below 8 pack several samples into each byte, leftmost pixel in
//! the most significant bits. A row that does not fill its last byte is
//! padded, and the padding bits mean nothing. 16-bit samples are
//! big-endian.

/// Unpacks `count` samples of 1, 2 or 4 bits each, most significant
/// bits first, ignoring any padding at the end of the row.
pub(crate) fn unpack_bits(
    row: &[u8],
    bit_depth: u8,
    count: usize,
) -> impl Iterator<Item = u8> + '_ {
    debug_assert!(matches!(bit_depth, 1 | 2 | 4));
    let per_byte = 8 / usize::from(bit_depth);
    let mask = (1u8 << bit_depth) - 1;
    (0..count).map(move |i| {
        let byte = row[i / per_byte];
        // Sample 0 of each byte sits in the top bits.
        let shift = 8 - bit_depth * (1 + (i % per_byte) as u8);
        (byte >> shift) & mask
    })
}

/// Reads `count` samples of any legal depth into `out`, replacing its
/// contents. Samples come out at their stored precision, not rescaled.
pub(crate) fn read_samples(row: &[u8], bit_depth: u8, count: usize, out: &mut Vec<u16>) {
    out.clear();
    match bit_depth {
        1 | 2 | 4 => out.extend(unpack_bits(row, bit_depth, count).map(u16::from)),
        8 => out.extend(row[..count].iter().map(|&b| u16::from(b))),
        16 => out.extend(
            row[..count * 2]
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
        ),
        _ => unreachable!("bit depth {bit_depth} was rejected when IHDR was parsed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpack(row: &[u8], depth: u8, count: usize) -> Vec<u8> {
        unpack_bits(row, depth, count).collect()
    }

    #[test]
    fn one_bit_is_msb_first() {
        assert_eq!(unpack(&[0b1011_0001], 1, 8), [1, 0, 1, 1, 0, 0, 0, 1]);
    }

    #[test]
    fn two_bit() {
        assert_eq!(unpack(&[0b11_10_01_00], 2, 4), [3, 2, 1, 0]);
    }

    #[test]
    fn four_bit() {
        assert_eq!(unpack(&[0xA5, 0x3C], 4, 4), [0xA, 0x5, 0x3, 0xC]);
    }

    #[test]
    fn samples_continue_across_bytes() {
        assert_eq!(
            unpack(&[0b0000_0001, 0b1000_0000], 1, 9),
            [0, 0, 0, 0, 0, 0, 0, 1, 1]
        );
        assert_eq!(unpack(&[0x12, 0x34, 0x56], 4, 5), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn padding_bits_are_ignored() {
        // Three 1-bit pixels use the top three bits; the other five are
        // padding and may hold anything.
        assert_eq!(unpack(&[0b101_11111], 1, 3), [1, 0, 1]);
        assert_eq!(unpack(&[0b101_00000], 1, 3), [1, 0, 1]);
        // One 4-bit pixel: the low nibble is padding.
        assert_eq!(unpack(&[0x7F], 4, 1), [7]);
        // Three 2-bit pixels: the last pair is padding.
        assert_eq!(unpack(&[0b01_10_11_11], 2, 3), [1, 2, 3]);
    }

    #[test]
    fn against_a_bit_string() {
        // Independent check: spell every row out as a string of bits and
        // slice it, for every depth, every width and a spread of bytes.
        let row: Vec<u8> = (0..16u32).map(|i| (i * 37 + 11) as u8).collect();
        let bits: String = row.iter().map(|b| format!("{b:08b}")).collect();
        for depth in [1u8, 2, 4] {
            let d = usize::from(depth);
            for count in 0..=row.len() * 8 / d {
                let expected: Vec<u8> = (0..count)
                    .map(|i| u8::from_str_radix(&bits[i * d..(i + 1) * d], 2).unwrap())
                    .collect();
                assert_eq!(
                    unpack(&row, depth, count),
                    expected,
                    "depth {depth}, {count} samples"
                );
            }
        }
    }

    #[test]
    fn eight_and_sixteen_bit() {
        let mut out = Vec::new();
        read_samples(&[1, 2, 3, 250], 8, 3, &mut out);
        assert_eq!(out, [1, 2, 3]);
        read_samples(&[0x12, 0x34, 0xFF, 0x00], 16, 2, &mut out);
        assert_eq!(out, [0x1234, 0xFF00]);
    }

    #[test]
    fn read_samples_replaces_previous_contents() {
        let mut out = vec![9, 9, 9, 9];
        read_samples(&[0b1100_0000], 1, 2, &mut out);
        assert_eq!(out, [1, 1]);
    }
}
