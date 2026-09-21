//! Undoing the scanline filters (PNG spec, section 9).
//!
//! Each scanline starts with a filter-type byte. The filters work on
//! bytes, not pixels: byte `x` is predicted from `a`, the byte one pixel
//! to the left (`stride` bytes back), `b`, the byte above, and `c`, the
//! byte above and to the left. All arithmetic is modulo 256.

use crate::error::Error;

/// The five filter types of filter method 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    None = 0,
    Sub = 1,
    Up = 2,
    Average = 3,
    Paeth = 4,
}

impl Filter {
    pub(crate) fn from_byte(b: u8) -> Option<Filter> {
        Some(match b {
            0 => Filter::None,
            1 => Filter::Sub,
            2 => Filter::Up,
            3 => Filter::Average,
            4 => Filter::Paeth,
            _ => return None,
        })
    }
}

/// The Paeth predictor, exactly as the spec writes it (section 9.4).
///
/// It estimates `p = a + b - c` and returns whichever neighbour is
/// closest to it, preferring `a`, then `b`, then `c` on ties. The
/// computation must be done without overflow, which is why it widens to
/// `i16` before subtracting.
pub(crate) fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let pa = (p - ia).abs();
    let pb = (p - ib).abs();
    let pc = (p - ic).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Reverses one filter in place. `prev` is the previous scanline after
/// its own unfiltering, or all zeros for the first row of a pass.
pub(crate) fn unfilter_row(filter: Filter, stride: usize, prev: &[u8], row: &mut [u8]) {
    debug_assert_eq!(prev.len(), row.len());
    match filter {
        Filter::None => {}
        Filter::Sub => {
            for i in stride..row.len() {
                row[i] = row[i].wrapping_add(row[i - stride]);
            }
        }
        Filter::Up => {
            for (x, &b) in row.iter_mut().zip(prev) {
                *x = x.wrapping_add(b);
            }
        }
        Filter::Average => {
            for i in 0..row.len() {
                let a = if i >= stride { row[i - stride] } else { 0 };
                // The sum needs nine bits; do it in u16 so it cannot wrap.
                let avg = ((u16::from(a) + u16::from(prev[i])) / 2) as u8;
                row[i] = row[i].wrapping_add(avg);
            }
        }
        Filter::Paeth => {
            for i in 0..row.len() {
                let (a, c) = if i >= stride {
                    (row[i - stride], prev[i - stride])
                } else {
                    (0, 0)
                };
                row[i] = row[i].wrapping_add(paeth_predictor(a, prev[i], c));
            }
        }
    }
}

/// Unfilters one pass: `rows` scanlines of `1 + row_bytes` bytes each
/// (filter byte first). Returns the rows back to back, filter bytes
/// removed.
///
/// `first_row` is the index of this pass's first scanline in the whole
/// stream, so errors can say which row was bad.
pub(crate) fn unfilter_pass(
    data: &[u8],
    row_bytes: usize,
    stride: usize,
    first_row: usize,
) -> Result<Vec<u8>, Error> {
    debug_assert_eq!(data.len() % (row_bytes + 1), 0);
    let rows = data.len() / (row_bytes + 1);
    let mut out = vec![0u8; rows * row_bytes];
    let zeros = vec![0u8; row_bytes];
    for (y, line) in data.chunks_exact(row_bytes + 1).enumerate() {
        let filter = Filter::from_byte(line[0]).ok_or(Error::InvalidFilterType {
            filter: line[0],
            row: first_row + y,
        })?;
        let (done, rest) = out.split_at_mut(y * row_bytes);
        let prev = if y == 0 {
            &zeros[..]
        } else {
            &done[(y - 1) * row_bytes..]
        };
        let row = &mut rest[..row_bytes];
        row.copy_from_slice(&line[1..]);
        unfilter_row(filter, stride, prev, row);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second, independent formulation of the predictor: of the three
    /// neighbours, take the one nearest `a + b - c`, breaking ties in the
    /// order a, b, c.
    fn nearest_neighbour(a: u8, b: u8, c: u8) -> u8 {
        let p = i32::from(a) + i32::from(b) - i32::from(c);
        [a, b, c]
            .into_iter()
            .enumerate()
            .min_by_key(|&(order, v)| ((p - i32::from(v)).abs(), order))
            .map(|(_, v)| v)
            .unwrap()
    }

    #[test]
    fn paeth_agrees_with_the_definition_for_every_input() {
        // All 2^24 combinations of (a, b, c).
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                for c in 0..=255u8 {
                    assert_eq!(
                        paeth_predictor(a, b, c),
                        nearest_neighbour(a, b, c),
                        "a={a} b={b} c={c}"
                    );
                }
            }
        }
    }

    #[test]
    fn paeth_hand_worked_cases() {
        // p = 10 + 20 - 5 = 25: pa = 15, pb = 5, pc = 20, so b.
        assert_eq!(paeth_predictor(10, 20, 5), 20);
        // p = 20 + 10 - 5 = 25: pa = 5, pb = 15, pc = 20, so a.
        assert_eq!(paeth_predictor(20, 10, 5), 20);
        // p = 100 + 0 - 100 = 0: pa = 100, pb = 0, pc = 100, so b.
        assert_eq!(paeth_predictor(100, 0, 100), 0);
        // p = 5 + 5 - 250 = -240 is below zero; pa = pb = 245, pc = 490.
        assert_eq!(paeth_predictor(5, 5, 250), 5);
        // p = 250 + 250 - 5 = 495 is above 255; the i16 maths must not wrap.
        assert_eq!(paeth_predictor(250, 250, 5), 250);
        // Flat region: every distance is zero and a wins.
        assert_eq!(paeth_predictor(7, 7, 7), 7);
    }

    #[test]
    fn paeth_tie_breaks_prefer_a_then_b_then_c() {
        // p = 8 + 11 - 10 = 9: pa = 1, pb = 2, pc = 1. a and c tie; a wins.
        assert_eq!(paeth_predictor(8, 11, 10), 8);
        // p = 11 + 8 - 10 = 9: pa = 2, pb = 1, pc = 1. b and c tie; b wins.
        assert_eq!(paeth_predictor(11, 8, 10), 8);
        // An a/b tie can only happen when a == b (otherwise a + b = 2c and
        // c is exactly on target), so its order is invisible in the output.
    }

    /// The encoder side, written independently from the spec's
    /// filtering equations, so the unfilter can be tested by round trip.
    fn filter_row(filter: Filter, stride: usize, prev: &[u8], raw: &[u8]) -> Vec<u8> {
        let left = |i: usize| if i >= stride { raw[i - stride] } else { 0 };
        let upleft = |i: usize| if i >= stride { prev[i - stride] } else { 0 };
        (0..raw.len())
            .map(|i| {
                let predicted = match filter {
                    Filter::None => 0,
                    Filter::Sub => left(i),
                    Filter::Up => prev[i],
                    Filter::Average => ((u16::from(left(i)) + u16::from(prev[i])) >> 1) as u8,
                    Filter::Paeth => nearest_neighbour(left(i), prev[i], upleft(i)),
                };
                raw[i].wrapping_sub(predicted)
            })
            .collect()
    }

    /// xorshift64*, so the tests are random but reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn byte(&mut self) -> u8 {
            (self.next() >> 56) as u8
        }
    }

    const FILTERS: [Filter; 5] = [
        Filter::None,
        Filter::Sub,
        Filter::Up,
        Filter::Average,
        Filter::Paeth,
    ];

    #[test]
    fn round_trips_every_filter_and_stride() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        // Every stride a real PNG can have: 1 (all sub-byte depths and
        // 8-bit gray/palette) up to 8 (16-bit RGBA).
        for stride in [1, 2, 3, 4, 6, 8] {
            for pixels in [1, 2, 3, 7, 33] {
                let row_bytes = stride * pixels;
                for _ in 0..20 {
                    let rows = 6;
                    let raw: Vec<u8> = (0..rows * row_bytes).map(|_| rng.byte()).collect();
                    // Pick a filter per row at random, as encoders do.
                    let mut encoded = Vec::new();
                    let zeros = vec![0; row_bytes];
                    for y in 0..rows {
                        let prev = if y == 0 {
                            &zeros[..]
                        } else {
                            &raw[(y - 1) * row_bytes..y * row_bytes]
                        };
                        let f = FILTERS[(rng.next() % 5) as usize];
                        encoded.push(f as u8);
                        encoded.extend(filter_row(
                            f,
                            stride,
                            prev,
                            &raw[y * row_bytes..(y + 1) * row_bytes],
                        ));
                    }
                    let decoded = unfilter_pass(&encoded, row_bytes, stride, 0).unwrap();
                    assert_eq!(decoded, raw, "stride {stride}, {pixels} px");
                }
            }
        }
    }

    #[test]
    fn sub_uses_the_stride_not_the_previous_byte() {
        // 8-bit RGB, stride 3: each channel adds to the same channel of
        // the pixel to its left.
        let mut row = [10, 20, 30, 1, 2, 3];
        unfilter_row(Filter::Sub, 3, &[0; 6], &mut row);
        assert_eq!(row, [10, 20, 30, 11, 22, 33]);
    }

    #[test]
    fn sub_with_sub_byte_depth_uses_whole_bytes() {
        // 1-bit gray packs eight pixels per byte, but the filter still
        // steps one byte at a time.
        let mut row = [0b1000_0000, 0b0000_0001];
        unfilter_row(Filter::Sub, 1, &[0; 2], &mut row);
        assert_eq!(row, [0b1000_0000, 0b1000_0001]);
    }

    #[test]
    fn arithmetic_wraps_modulo_256() {
        let mut row = [200, 100];
        unfilter_row(Filter::Sub, 1, &[0; 2], &mut row);
        assert_eq!(row, [200, 44]);
        let mut row = [200];
        unfilter_row(Filter::Up, 1, &[100], &mut row);
        assert_eq!(row, [44]);
    }

    #[test]
    fn average_does_not_overflow_before_halving() {
        // Byte 1 sees a = 255 and b = 255. The average is 255; summing in
        // a u8 would give (510 mod 256) / 2 = 127.
        let mut row = [255, 0];
        unfilter_row(Filter::Average, 1, &[0, 255], &mut row);
        assert_eq!(row, [255, 255]);
    }

    #[test]
    fn first_row_treats_the_row_above_as_zero() {
        // Up on the first row is a no-op.
        assert_eq!(unfilter_pass(&[2, 5, 6], 2, 1, 0).unwrap(), [5, 6]);
        // Paeth with b = c = 0 always picks a, so it degenerates to Sub.
        assert_eq!(unfilter_pass(&[4, 5, 6], 2, 1, 0).unwrap(), [5, 11]);
        // Average halves the left byte alone.
        assert_eq!(unfilter_pass(&[3, 5, 6], 2, 1, 0).unwrap(), [5, 8]);
    }

    #[test]
    fn invalid_filter_type_reports_the_row() {
        let data = [0, 1, 1, 0, 2, 2, 5, 3, 3];
        assert_eq!(
            unfilter_pass(&data, 2, 1, 10),
            Err(Error::InvalidFilterType { filter: 5, row: 12 })
        );
    }
}
