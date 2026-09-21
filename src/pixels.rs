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
///
/// At depths of 8 bits and below there are at most 256 possible sample
/// values, so the rescaling is done once into a table and looked up
/// afterwards rather than divided out per sample.
pub(crate) struct RowConverter<'a, C> {
    color_type: ColorType,
    bit_depth: u8,
    max_in: u32,
    palette: &'a [[u8; 3]],
    transparency: Option<&'a Transparency>,
    samples: Vec<u16>,
    /// Every sample value at this bit depth, rescaled. Empty at 16 bits,
    /// where a table would need 65536 entries to save one division.
    sample_lut: Vec<C>,
    /// Every byte value rescaled, for palette entries and their alpha,
    /// which are 8-bit whatever the image's depth is.
    byte_lut: Vec<C>,
}

impl<'a, C: Channel> RowConverter<'a, C> {
    pub(crate) fn new(
        header: &Header,
        palette: Option<&'a [[u8; 3]]>,
        transparency: Option<&'a Transparency>,
    ) -> Self {
        let max_in = (1u32 << header.bit_depth) - 1;
        RowConverter {
            color_type: header.color_type,
            bit_depth: header.bit_depth,
            max_in,
            palette: palette.unwrap_or(&[]),
            transparency,
            samples: Vec::new(),
            sample_lut: if header.bit_depth <= 8 {
                (0..=max_in as u16).map(|s| rescale(s, max_in)).collect()
            } else {
                Vec::new()
            },
            byte_lut: (0..=255).map(|b| rescale(b, 255)).collect(),
        }
    }

    /// One stored sample, rescaled to the output depth.
    #[inline]
    fn scale(&self, sample: u16) -> C {
        match self.sample_lut.get(usize::from(sample)) {
            Some(&value) => value,
            // 16-bit samples: too many to tabulate.
            None => rescale(sample, self.max_in),
        }
    }

    /// One 8-bit value from a palette, rescaled to the output depth.
    #[inline]
    fn scale_byte(&self, byte: u8) -> C {
        self.byte_lut[usize::from(byte)]
    }

    /// Converts one unfiltered scanline of `width` pixels, handing each
    /// pixel to `put(index_in_row, rgba)`.
    pub(crate) fn convert(
        &mut self,
        row: &[u8],
        width: usize,
        mut put: impl FnMut(usize, [C; 4]),
    ) -> Result<(), Error> {
        let channels = self.color_type.channels();
        read_samples(row, self.bit_depth, width * channels, &mut self.samples);
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
                    let v = self.scale(g);
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
                        [self.scale(px[0]), self.scale(px[1]), self.scale(px[2]), a],
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
                            self.scale_byte(r),
                            self.scale_byte(g),
                            self.scale_byte(b),
                            self.scale_byte(a),
                        ],
                    );
                }
            }
            ColorType::GrayscaleAlpha => {
                for (i, px) in s.chunks_exact(2).enumerate() {
                    let v = self.scale(px[0]);
                    put(i, [v, v, v, self.scale(px[1])]);
                }
            }
            ColorType::Rgba => {
                for (i, px) in s.chunks_exact(4).enumerate() {
                    put(
                        i,
                        [
                            self.scale(px[0]),
                            self.scale(px[1]),
                            self.scale(px[2]),
                            self.scale(px[3]),
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

    #[test]
    fn the_lookup_tables_agree_with_the_formula() {
        // The tables are an optimisation; rescale is the definition.
        for &color_type in &[ColorType::Grayscale, ColorType::Indexed, ColorType::Rgba] {
            for &bit_depth in color_type.allowed_bit_depths() {
                let h = header(color_type, bit_depth);
                let to8: RowConverter<'_, u8> = RowConverter::new(&h, None, None);
                let to16: RowConverter<'_, u16> = RowConverter::new(&h, None, None);
                let max = (1u32 << bit_depth) - 1;
                for sample in 0..=max as u16 {
                    assert_eq!(to8.scale(sample), rescale::<u8>(sample, max));
                    assert_eq!(to16.scale(sample), rescale::<u16>(sample, max));
                }
                // 16-bit samples fall through to the formula instead.
                if bit_depth == 16 {
                    assert!(to8.sample_lut.is_empty());
                    assert_eq!(to8.scale(65535), 255);
                    assert_eq!(to8.scale(0x00FF), 1);
                }
                for byte in 0..=255u8 {
                    assert_eq!(to8.scale_byte(byte), byte);
                    assert_eq!(to16.scale_byte(byte), u16::from(byte) * 257);
                }
            }
        }
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
        RowConverter::<C>::new(&header, palette, trns).convert(row, width, |i, px| out[i] = px)?;
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
