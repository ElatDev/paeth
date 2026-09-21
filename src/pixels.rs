//! Turning stored samples into RGBA.
//!
//! Every color type and depth ends up as four channels at 8 or 16 bits.
//! Samples are rescaled with the spec's preferred formula (section
//! 13.12), `round(sample × MAXOUT / MAXIN)`, which is exact whenever
//! the target depth is a multiple of the source depth: 1-bit white is
//! 255 or 65535, 4-bit 0x9 is 0x99 or 0x9999, 8-bit 0xAB is 0xABAB.

use crate::error::Error;
use crate::header::{ColorType, Header};
use crate::metadata::Transparency;
use crate::unpack::read_samples;

/// An output channel type: `u8` or `u16`.
pub trait Channel: Copy + Default + private::Sealed + 'static {
    /// The largest value, 255 or 65535.
    const MAX: u32;
    #[doc(hidden)]
    fn from_u32(v: u32) -> Self;
}

impl Channel for u8 {
    const MAX: u32 = 255;
    fn from_u32(v: u32) -> Self {
        v as u8
    }
}

impl Channel for u16 {
    const MAX: u32 = 65535;
    fn from_u32(v: u32) -> Self {
        v as u16
    }
}

mod private {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// Rescales `sample` from `0..=max_in` to `0..=C::MAX`, rounding to
/// nearest. `max_in` is always `2^depth − 1`, which is odd, so a tie is
/// impossible. The product is at most 65535² + 32767, which fits a u32.
pub(crate) fn rescale<C: Channel>(sample: u16, max_in: u32) -> C {
    C::from_u32((u32::from(sample) * C::MAX + max_in / 2) / max_in)
}

/// Converts scanlines of one image to RGBA, reusing its sample buffer.
pub(crate) struct RowConverter<'a> {
    color_type: ColorType,
    bit_depth: u8,
    max_in: u32,
    palette: &'a [[u8; 3]],
    transparency: Option<&'a Transparency>,
    samples: Vec<u16>,
}

impl<'a> RowConverter<'a> {
    pub(crate) fn new(
        header: &Header,
        palette: Option<&'a [[u8; 3]]>,
        transparency: Option<&'a Transparency>,
    ) -> Self {
        RowConverter {
            color_type: header.color_type,
            bit_depth: header.bit_depth,
            max_in: (1u32 << header.bit_depth) - 1,
            palette: palette.unwrap_or(&[]),
            transparency,
            samples: Vec::new(),
        }
    }

    /// Converts one unfiltered scanline of `width` pixels, handing each
    /// pixel to `put(index_in_row, rgba)`.
    pub(crate) fn convert<C: Channel>(
        &mut self,
        row: &[u8],
        width: usize,
        mut put: impl FnMut(usize, [C; 4]),
    ) -> Result<(), Error> {
        let channels = self.color_type.channels();
        read_samples(row, self.bit_depth, width * channels, &mut self.samples);
        let max = self.max_in;
        let opaque = C::from_u32(C::MAX);
        let clear = C::from_u32(0);
        let s = &self.samples;
        match self.color_type {
            ColorType::Grayscale => {
                let key = match self.transparency {
                    Some(Transparency::Gray(g)) => Some(*g),
                    _ => None,
                };
                for (i, &g) in s.iter().enumerate() {
                    let v = rescale(g, max);
                    let a = if key == Some(g) { clear } else { opaque };
                    put(i, [v, v, v, a]);
                }
            }
            ColorType::Rgb => {
                let key = match self.transparency {
                    Some(&Transparency::Rgb(r, g, b)) => Some([r, g, b]),
                    _ => None,
                };
                for (i, px) in s.chunks_exact(3).enumerate() {
                    let a = if key.as_ref().map(|k| &k[..]) == Some(px) {
                        clear
                    } else {
                        opaque
                    };
                    put(
                        i,
                        [
                            rescale(px[0], max),
                            rescale(px[1], max),
                            rescale(px[2], max),
                            a,
                        ],
                    );
                }
            }
            ColorType::Indexed => {
                let alphas: &[u8] = match self.transparency {
                    Some(Transparency::Palette(a)) => a,
                    _ => &[],
                };
                for (i, &index) in s.iter().enumerate() {
                    let entry = usize::from(index);
                    let &[r, g, b] =
                        self.palette
                            .get(entry)
                            .ok_or(Error::PaletteIndexOutOfRange {
                                index: index as u8,
                                palette_len: self.palette.len(),
                            })?;
                    let a = alphas.get(entry).copied().unwrap_or(255);
                    put(
                        i,
                        [
                            rescale(r.into(), 255),
                            rescale(g.into(), 255),
                            rescale(b.into(), 255),
                            rescale(a.into(), 255),
                        ],
                    );
                }
            }
            ColorType::GrayscaleAlpha => {
                for (i, px) in s.chunks_exact(2).enumerate() {
                    let v = rescale(px[0], max);
                    put(i, [v, v, v, rescale(px[1], max)]);
                }
            }
            ColorType::Rgba => {
                for (i, px) in s.chunks_exact(4).enumerate() {
                    put(
                        i,
                        [
                            rescale(px[0], max),
                            rescale(px[1], max),
                            rescale(px[2], max),
                            rescale(px[3], max),
                        ],
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Interlace;

    #[test]
    fn rescaling_up_is_bit_replication() {
        // Scaling from 2^d - 1 to 2^16 - 1 multiplies by 0x FFFF / (2^d - 1),
        // which repeats the d-bit pattern across 16 bits.
        for depth in [1u8, 2, 4, 8] {
            let max = (1u32 << depth) - 1;
            for s in 0..=max as u16 {
                let mut replicated = 0u32;
                for _ in 0..16 / depth {
                    replicated = (replicated << depth) | u32::from(s);
                }
                assert_eq!(
                    u32::from(rescale::<u16>(s, max)),
                    replicated,
                    "{depth}-bit {s}"
                );
            }
        }
    }

    #[test]
    fn rescaling_to_8_bits_is_exact_for_small_depths() {
        assert_eq!(rescale::<u8>(1, 1), 255);
        assert_eq!(rescale::<u8>(2, 3), 170);
        assert_eq!(rescale::<u8>(0x9, 15), 0x99);
        assert_eq!(rescale::<u8>(0xAB, 255), 0xAB);
    }

    #[test]
    fn sixteen_to_eight_rounds_to_nearest() {
        // round(v * 255 / 65535) = round(v / 257).
        for v in 0..=65535u16 {
            let expected = (f64::from(v) / 257.0).round() as u8;
            assert_eq!(rescale::<u8>(v, 65535), expected, "{v}");
        }
        // Truncation (v >> 8) would give 0 here.
        assert_eq!(rescale::<u8>(0x00FF, 65535), 1);
    }

    fn header(color_type: ColorType, bit_depth: u8) -> Header {
        Header {
            width: 1,
            height: 1,
            bit_depth,
            color_type,
            interlace: Interlace::None,
        }
    }

    fn convert<C: Channel>(
        header: Header,
        palette: Option<&[[u8; 3]]>,
        trns: Option<&Transparency>,
        row: &[u8],
        width: usize,
    ) -> Result<Vec<[C; 4]>, Error> {
        let mut out = vec![[C::default(); 4]; width];
        RowConverter::new(&header, palette, trns).convert(row, width, |i, px| out[i] = px)?;
        Ok(out)
    }

    #[test]
    fn gray_with_transparent_key() {
        let trns = Transparency::Gray(1);
        let px = convert::<u8>(
            header(ColorType::Grayscale, 2),
            None,
            Some(&trns),
            &[0b00_01_10_11],
            4,
        )
        .unwrap();
        assert_eq!(
            px,
            [
                [0, 0, 0, 255],
                [85, 85, 85, 0],
                [170, 170, 170, 255],
                [255, 255, 255, 255]
            ]
        );
    }

    #[test]
    fn rgb16_key_matches_all_three_samples_exactly() {
        let trns = Transparency::Rgb(0x0102, 0x0304, 0x0506);
        let row = [1, 2, 3, 4, 5, 6, 1, 2, 3, 4, 5, 7];
        let px = convert::<u16>(header(ColorType::Rgb, 16), None, Some(&trns), &row, 2).unwrap();
        assert_eq!(px[0][3], 0);
        assert_eq!(px[1][3], 65535);
        assert_eq!(px[1][..3], [0x0102, 0x0304, 0x0507]);
    }

    #[test]
    fn palette_with_partial_alpha() {
        let palette = [[10, 20, 30], [40, 50, 60], [70, 80, 90]];
        let trns = Transparency::Palette(vec![0, 128]);
        let px = convert::<u8>(
            header(ColorType::Indexed, 8),
            Some(&palette),
            Some(&trns),
            &[2, 1, 0],
            3,
        )
        .unwrap();
        // Entry 2 has no alpha value, so it is opaque.
        assert_eq!(px, [[70, 80, 90, 255], [40, 50, 60, 128], [10, 20, 30, 0]]);
    }

    #[test]
    fn palette_index_out_of_range() {
        let palette = [[0; 3]; 2];
        assert_eq!(
            convert::<u8>(
                header(ColorType::Indexed, 2),
                Some(&palette),
                None,
                &[0b00_01_10_00],
                3
            ),
            Err(Error::PaletteIndexOutOfRange {
                index: 2,
                palette_len: 2
            })
        );
    }

    #[test]
    fn gray_alpha_and_rgba_pass_through() {
        let px = convert::<u8>(
            header(ColorType::GrayscaleAlpha, 8),
            None,
            None,
            &[9, 200],
            1,
        )
        .unwrap();
        assert_eq!(px, [[9, 9, 9, 200]]);
        let px = convert::<u16>(header(ColorType::Rgba, 8), None, None, &[1, 2, 3, 4], 1).unwrap();
        assert_eq!(px, [[0x0101, 0x0202, 0x0303, 0x0404]]);
    }
}
