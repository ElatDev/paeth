//! IHDR: the image header (PNG spec, section 11.2.1).

use std::fmt;

use crate::chunk::ChunkType;
use crate::error::Error;

/// How pixels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorType {
    /// Type 0: one gray sample per pixel.
    Grayscale,
    /// Type 2: red, green, blue.
    Rgb,
    /// Type 3: one index into PLTE per pixel.
    Indexed,
    /// Type 4: gray, then alpha.
    GrayscaleAlpha,
    /// Type 6: red, green, blue, alpha.
    Rgba,
}

impl ColorType {
    /// Parses the IHDR color type byte.
    pub fn from_code(code: u8) -> Option<ColorType> {
        Some(match code {
            0 => ColorType::Grayscale,
            2 => ColorType::Rgb,
            3 => ColorType::Indexed,
            4 => ColorType::GrayscaleAlpha,
            6 => ColorType::Rgba,
            _ => return None,
        })
    }

    /// The IHDR color type byte.
    pub fn code(self) -> u8 {
        match self {
            ColorType::Grayscale => 0,
            ColorType::Rgb => 2,
            ColorType::Indexed => 3,
            ColorType::GrayscaleAlpha => 4,
            ColorType::Rgba => 6,
        }
    }

    /// Samples per pixel as stored in the file.
    pub fn channels(self) -> usize {
        match self {
            ColorType::Grayscale | ColorType::Indexed => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        }
    }

    /// Bit depths the spec allows for this color type (Table 11.1).
    pub fn allowed_bit_depths(self) -> &'static [u8] {
        match self {
            ColorType::Grayscale => &[1, 2, 4, 8, 16],
            ColorType::Indexed => &[1, 2, 4, 8],
            ColorType::Rgb | ColorType::GrayscaleAlpha | ColorType::Rgba => &[8, 16],
        }
    }

    /// Whether each pixel carries its own alpha sample.
    pub fn has_alpha(self) -> bool {
        matches!(self, ColorType::GrayscaleAlpha | ColorType::Rgba)
    }
}

impl fmt::Display for ColorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ColorType::Grayscale => "grayscale",
            ColorType::Rgb => "RGB",
            ColorType::Indexed => "indexed",
            ColorType::GrayscaleAlpha => "grayscale+alpha",
            ColorType::Rgba => "RGBA",
        })
    }
}

/// Scanline order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interlace {
    /// Method 0: rows top to bottom.
    None,
    /// Method 1: seven passes over an 8×8 grid, coarse to fine.
    Adam7,
}

/// The fields of IHDR.
///
/// Compression and filter method are not stored: PNG defines exactly one
/// of each, and [`Header::parse`] rejects anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Header {
    /// Width in pixels, 1 to 2³¹−1.
    pub width: u32,
    /// Height in pixels, 1 to 2³¹−1.
    pub height: u32,
    /// Bits per sample (or per palette index).
    pub bit_depth: u8,
    /// How pixels are stored.
    pub color_type: ColorType,
    /// Scanline order.
    pub interlace: Interlace,
}

const MAX_DIMENSION: u32 = 0x7FFF_FFFF;

impl Header {
    /// Parses and validates the 13 bytes of an IHDR chunk.
    ///
    /// # Errors
    ///
    /// A wrong length, a zero or oversized dimension, or a color type,
    /// bit depth, compression, filter or interlace method the spec does
    /// not define.
    pub fn parse(data: &[u8]) -> Result<Header, Error> {
        let &[
            w0,
            w1,
            w2,
            w3,
            h0,
            h1,
            h2,
            h3,
            bit_depth,
            color,
            compression,
            filter,
            interlace,
        ] = data
        else {
            return Err(Error::BadChunkLength {
                chunk: ChunkType::IHDR,
                length: data.len(),
            });
        };
        let width = u32::from_be_bytes([w0, w1, w2, w3]);
        let height = u32::from_be_bytes([h0, h1, h2, h3]);
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(Error::InvalidDimensions { width, height });
        }
        let color_type = ColorType::from_code(color).ok_or(Error::InvalidColorType(color))?;
        if !color_type.allowed_bit_depths().contains(&bit_depth) {
            return Err(Error::InvalidBitDepth {
                color_type,
                bit_depth,
            });
        }
        if compression != 0 {
            return Err(Error::InvalidCompressionMethod(compression));
        }
        if filter != 0 {
            return Err(Error::InvalidFilterMethod(filter));
        }
        let interlace = match interlace {
            0 => Interlace::None,
            1 => Interlace::Adam7,
            other => return Err(Error::InvalidInterlaceMethod(other)),
        };
        Ok(Header {
            width,
            height,
            bit_depth,
            color_type,
            interlace,
        })
    }

    /// Bits per stored pixel: channels × bit depth.
    pub fn bits_per_pixel(&self) -> usize {
        self.color_type.channels() * usize::from(self.bit_depth)
    }

    /// The distance, in bytes, from a byte to "the corresponding byte of
    /// the pixel to its left" that the filters use. For depths below 8
    /// several pixels share a byte, and the spec rounds this up to 1.
    pub fn filter_stride(&self) -> usize {
        self.bits_per_pixel().div_ceil(8)
    }

    /// Bytes in one scanline of `width` pixels, excluding the filter
    /// byte. Rows are padded to a whole byte.
    pub fn row_bytes(&self, width: u32) -> u64 {
        (u64::from(width) * self.bits_per_pixel() as u64).div_ceil(8)
    }

    /// Total pixel count.
    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ihdr(width: u32, height: u32, depth: u8, color: u8, interlace: u8) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&[depth, color, 0, 0, interlace]);
        v
    }

    #[test]
    fn parses_fields() {
        let h = Header::parse(&ihdr(32, 17, 16, 6, 1)).unwrap();
        assert_eq!(
            h,
            Header {
                width: 32,
                height: 17,
                bit_depth: 16,
                color_type: ColorType::Rgba,
                interlace: Interlace::Adam7,
            }
        );
    }

    #[test]
    fn every_color_type_and_depth_combination() {
        // Table 11.1: exactly these 15 combinations are legal.
        let legal: &[(u8, &[u8])] = &[
            (0, &[1, 2, 4, 8, 16]),
            (2, &[8, 16]),
            (3, &[1, 2, 4, 8]),
            (4, &[8, 16]),
            (6, &[8, 16]),
        ];
        let mut accepted = 0;
        for color in 0..=255u8 {
            for depth in 0..=255u8 {
                let ok = Header::parse(&ihdr(1, 1, depth, color, 0)).is_ok();
                let expected = legal
                    .iter()
                    .any(|(c, depths)| *c == color && depths.contains(&depth));
                assert_eq!(ok, expected, "color type {color}, depth {depth}");
                accepted += usize::from(ok);
            }
        }
        assert_eq!(accepted, 15);
    }

    #[test]
    fn rejects_bad_fields() {
        let cases = [
            (
                ihdr(0, 1, 8, 0, 0),
                Error::InvalidDimensions {
                    width: 0,
                    height: 1,
                },
            ),
            (
                ihdr(1, 1 << 31, 8, 0, 0),
                Error::InvalidDimensions {
                    width: 1,
                    height: 1 << 31,
                },
            ),
            (ihdr(1, 1, 8, 1, 0), Error::InvalidColorType(1)),
            (
                ihdr(1, 1, 3, 2, 0),
                Error::InvalidBitDepth {
                    color_type: ColorType::Rgb,
                    bit_depth: 3,
                },
            ),
            (ihdr(1, 1, 8, 0, 2), Error::InvalidInterlaceMethod(2)),
        ];
        for (data, expected) in cases {
            assert_eq!(Header::parse(&data), Err(expected));
        }
        let mut compression = ihdr(1, 1, 8, 0, 0);
        compression[10] = 1;
        assert_eq!(
            Header::parse(&compression),
            Err(Error::InvalidCompressionMethod(1))
        );
        let mut filter = ihdr(1, 1, 8, 0, 0);
        filter[11] = 1;
        assert_eq!(Header::parse(&filter), Err(Error::InvalidFilterMethod(1)));
    }

    #[test]
    fn wrong_length() {
        for len in [0, 12, 14] {
            assert_eq!(
                Header::parse(&vec![0; len]),
                Err(Error::BadChunkLength {
                    chunk: ChunkType::IHDR,
                    length: len
                })
            );
        }
    }

    #[test]
    fn row_bytes_round_up_and_filter_stride_is_at_least_one() {
        let h = |depth, color| Header::parse(&ihdr(1, 1, depth, color, 0)).unwrap();
        // 1-bit gray: 9 pixels need 2 bytes, stride rounds up to 1.
        assert_eq!(h(1, 0).row_bytes(9), 2);
        assert_eq!(h(1, 0).filter_stride(), 1);
        // 4-bit palette: 3 pixels = 12 bits = 2 bytes.
        assert_eq!(h(4, 3).row_bytes(3), 2);
        // 16-bit RGBA: 8 bytes per pixel.
        assert_eq!(h(16, 6).row_bytes(3), 24);
        assert_eq!(h(16, 6).filter_stride(), 8);
        // 8-bit RGB: 3 bytes per pixel.
        assert_eq!(h(8, 2).filter_stride(), 3);
        // 16-bit gray+alpha: 4.
        assert_eq!(h(16, 4).filter_stride(), 4);
    }

    #[test]
    fn row_bytes_does_not_overflow_at_the_maximum_width() {
        let h = Header::parse(&ihdr(MAX_DIMENSION, 1, 16, 6, 0)).unwrap();
        assert_eq!(h.row_bytes(MAX_DIMENSION), u64::from(MAX_DIMENSION) * 8);
    }
}
