//! Hand-built files that are broken in exactly one way each.
//!
//! PngSuite's corrupt files cover the signature, IHDR and CRCs. These
//! cover the rest of what the spec says a decoder must reject: chunk
//! ordering, palette rules and image data of the wrong size.

mod support;

use paeth::chunk::ChunkType;
use paeth::{ColorType, Error, Limits, Png};
use support::{PngBuilder, zlib};

/// A valid 2×2 8-bit gray image: two scanlines, filter type 0.
const GRAY_2X2: [u8; 6] = [0, 10, 20, 0, 30, 40];

fn gray_2x2() -> PngBuilder {
    PngBuilder::new().ihdr(2, 2, 8, 0, 0)
}

#[test]
fn the_baseline_is_valid() {
    let png = gray_2x2().idat(&GRAY_2X2).iend().build();
    let img = paeth::decode(&png).unwrap();
    assert_eq!(
        img.pixels,
        [
            10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255, 40, 40, 40, 255
        ]
    );
}

#[test]
fn empty_file() {
    assert!(matches!(paeth::decode(&[]), Err(Error::Signature(_))));
}

#[test]
fn signature_only() {
    assert_eq!(
        paeth::decode(&PngBuilder::new().build()),
        Err(Error::MissingChunk(ChunkType::IHDR))
    );
}

#[test]
fn ihdr_must_be_first() {
    let png = PngBuilder::new()
        .chunk(b"gAMA", &45455u32.to_be_bytes())
        .ihdr(2, 2, 8, 0, 0)
        .idat(&GRAY_2X2)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::FirstChunkNotIhdr(ChunkType::gAMA))
    );
}

#[test]
fn two_ihdrs() {
    let png = gray_2x2()
        .ihdr(2, 2, 8, 0, 0)
        .idat(&GRAY_2X2)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::DuplicateChunk(ChunkType::IHDR))
    );
}

#[test]
fn missing_iend() {
    let png = gray_2x2().idat(&GRAY_2X2).build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::MissingChunk(ChunkType::IEND))
    );
}

#[test]
fn iend_with_data() {
    let png = gray_2x2().idat(&GRAY_2X2).chunk(b"IEND", &[0]).build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::BadChunkLength {
            chunk: ChunkType::IEND,
            length: 1
        })
    );
}

#[test]
fn truncated_mid_file() {
    let png = gray_2x2().idat(&GRAY_2X2).iend().build();
    // Losing the IEND chunk's CRC.
    assert!(matches!(
        paeth::decode(&png[..png.len() - 2]),
        Err(Error::Truncated { .. })
    ));
}

#[test]
fn bytes_after_iend_are_not_an_error() {
    let png = gray_2x2().idat(&GRAY_2X2).iend().raw(b"appended").build();
    let parsed = Png::parse(&png).unwrap();
    assert_eq!(parsed.trailing_bytes(), 8);
    assert!(parsed.decode().is_ok());
}

#[test]
fn idat_chunks_must_be_consecutive() {
    let z = zlib(&GRAY_2X2);
    let (a, b) = z.split_at(z.len() / 2);
    let png = gray_2x2()
        .chunk(b"IDAT", a)
        .chunk(b"tEXt", b"Comment\0in the way")
        .chunk(b"IDAT", b)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::MisplacedChunk {
            chunk: ChunkType::IDAT,
            rule: "IDAT chunks must be consecutive"
        })
    );
}

#[test]
fn idat_split_at_every_byte_decodes_the_same() {
    let reference = paeth::decode(&gray_2x2().idat(&GRAY_2X2).iend().build()).unwrap();
    let z = zlib(&GRAY_2X2);
    for cut in 0..=z.len() {
        let png = gray_2x2()
            .chunk(b"IDAT", &z[..cut])
            .chunk(b"IDAT", &z[cut..])
            .iend()
            .build();
        assert_eq!(paeth::decode(&png).unwrap(), reference, "cut at {cut}");
    }
    // And one IDAT per byte, like PngSuite's oi9 files.
    let png = z
        .iter()
        .fold(gray_2x2(), |b, byte| b.chunk(b"IDAT", &[*byte]))
        .iend()
        .build();
    assert_eq!(paeth::decode(&png).unwrap(), reference);
}

#[test]
fn unknown_critical_chunk() {
    let png = gray_2x2()
        .chunk(b"CRIT", &[])
        .idat(&GRAY_2X2)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::UnknownCriticalChunk(ChunkType(*b"CRIT")))
    );
}

#[test]
fn unknown_ancillary_chunk_is_skipped() {
    let png = gray_2x2()
        .chunk(b"vpAg", &[1, 2, 3])
        .idat(&GRAY_2X2)
        .iend()
        .build();
    let parsed = Png::parse(&png).unwrap();
    assert!(parsed.warnings().is_empty());
    assert!(
        parsed
            .chunks()
            .iter()
            .any(|c| c.kind == ChunkType(*b"vpAg"))
    );
    assert!(parsed.decode().is_ok());
}

// --- Palette rules --------------------------------------------------

/// A 2×1 indexed image using entries 0 and 1.
fn indexed(palette: &[u8]) -> PngBuilder {
    PngBuilder::new()
        .ihdr(2, 1, 8, 3, 0)
        .chunk(b"PLTE", palette)
}

const INDEXED_ROW: [u8; 3] = [0, 0, 1];

#[test]
fn palette_decodes() {
    let png = indexed(&[1, 2, 3, 4, 5, 6])
        .idat(&INDEXED_ROW)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png).unwrap().pixels,
        [1, 2, 3, 255, 4, 5, 6, 255]
    );
}

#[test]
fn indexed_image_without_palette() {
    let png = PngBuilder::new()
        .ihdr(2, 1, 8, 3, 0)
        .idat(&INDEXED_ROW)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::MissingChunk(ChunkType::PLTE))
    );
}

#[test]
fn palette_after_idat() {
    let png = PngBuilder::new()
        .ihdr(2, 1, 8, 3, 0)
        .idat(&INDEXED_ROW)
        .chunk(b"PLTE", &[0; 6])
        .iend()
        .build();
    assert!(matches!(
        paeth::decode(&png),
        Err(Error::MisplacedChunk {
            chunk: ChunkType::PLTE,
            ..
        })
    ));
}

#[test]
fn two_palettes() {
    let png = indexed(&[0; 6])
        .chunk(b"PLTE", &[0; 6])
        .idat(&INDEXED_ROW)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::DuplicateChunk(ChunkType::PLTE))
    );
}

#[test]
fn palette_in_grayscale_image() {
    let png = gray_2x2()
        .chunk(b"PLTE", &[0; 3])
        .idat(&GRAY_2X2)
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::UnexpectedPalette(ColorType::Grayscale))
    );
}

#[test]
fn palette_length_not_a_multiple_of_three() {
    for len in [0, 4, 769] {
        let png = indexed(&vec![0; len]).idat(&INDEXED_ROW).iend().build();
        assert_eq!(
            paeth::decode(&png),
            Err(Error::BadChunkLength {
                chunk: ChunkType::PLTE,
                length: len
            }),
            "length {len}"
        );
    }
}

#[test]
fn palette_larger_than_the_bit_depth_can_index() {
    // 1-bit indices reach entries 0 and 1 only.
    let png = PngBuilder::new()
        .ihdr(2, 1, 1, 3, 0)
        .chunk(b"PLTE", &[0; 9])
        .idat(&[0, 0b0100_0000])
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::PaletteTooLarge {
            entries: 3,
            bit_depth: 1
        })
    );
}

#[test]
fn pixel_index_past_the_end_of_the_palette() {
    let png = indexed(&[0; 3]).idat(&INDEXED_ROW).iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::PaletteIndexOutOfRange {
            index: 1,
            palette_len: 1
        })
    );
}

#[test]
fn suggested_palette_in_rgb_image_is_allowed() {
    let png = PngBuilder::new()
        .ihdr(1, 1, 8, 2, 0)
        .chunk(b"PLTE", &[9, 9, 9])
        .idat(&[0, 1, 2, 3])
        .iend()
        .build();
    let parsed = Png::parse(&png).unwrap();
    assert_eq!(parsed.palette(), Some(&[[9, 9, 9]][..]));
    assert_eq!(parsed.decode().unwrap().pixels, [1, 2, 3, 255]);
}

// --- Image data ------------------------------------------------------

#[test]
fn no_image_data() {
    let png = gray_2x2().iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::MissingChunk(ChunkType::IDAT))
    );
}

#[test]
fn empty_idat() {
    let png = gray_2x2().chunk(b"IDAT", &[]).iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::ImageDataTooShort {
            expected: 6,
            actual: 0
        })
    );
}

#[test]
fn one_scanline_short() {
    let png = gray_2x2().idat(&GRAY_2X2[..3]).iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::ImageDataTooShort {
            expected: 6,
            actual: 3
        })
    );
}

#[test]
fn one_byte_too_many() {
    let mut data = GRAY_2X2.to_vec();
    data.push(0);
    let png = gray_2x2().idat(&data).iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::ImageDataTooLong { expected: 6 })
    );
}

#[test]
fn garbage_after_the_zlib_stream() {
    let mut z = zlib(&GRAY_2X2);
    z.extend_from_slice(b"junk");
    let png = gray_2x2().chunk(b"IDAT", &z).iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::TrailingImageData { bytes: 4 })
    );
}

#[test]
fn idat_that_is_not_zlib() {
    let png = gray_2x2()
        .chunk(b"IDAT", b"definitely not deflate")
        .iend()
        .build();
    assert!(matches!(
        paeth::decode(&png),
        Err(Error::CorruptImageData(_))
    ));
}

#[test]
fn filter_type_five() {
    let png = gray_2x2().idat(&[0, 10, 20, 5, 30, 40]).iend().build();
    assert_eq!(
        paeth::decode(&png),
        Err(Error::InvalidFilterType { filter: 5, row: 1 })
    );
}

#[test]
fn interlaced_image_needs_all_seven_passes_of_data() {
    // 8×8 interlaced 8-bit gray: every pass is non-empty. Pass sizes are
    // 1x1, 1x1, 2x1, 2x2, 4x2, 4x4, 8x4, so 15 scanlines and 79 bytes.
    let full = vec![0u8; 15 + 64];
    let ok = PngBuilder::new()
        .ihdr(8, 8, 8, 0, 1)
        .idat(&full)
        .iend()
        .build();
    assert!(paeth::decode(&ok).is_ok());
    let short = PngBuilder::new()
        .ihdr(8, 8, 8, 0, 1)
        .idat(&full[..78])
        .iend()
        .build();
    assert_eq!(
        paeth::decode(&short),
        Err(Error::ImageDataTooShort {
            expected: 79,
            actual: 78
        })
    );
}

#[test]
fn interlaced_1x1_has_a_single_scanline() {
    // Passes 2 to 7 are empty and contribute nothing, not even a filter
    // byte.
    let png = PngBuilder::new()
        .ihdr(1, 1, 8, 0, 1)
        .idat(&[0, 77])
        .iend()
        .build();
    assert_eq!(paeth::decode(&png).unwrap().pixels, [77, 77, 77, 255]);
}

// --- Limits ------------------------------------------------------------

#[test]
fn pixel_limit() {
    let png = PngBuilder::new().ihdr(1000, 1000, 8, 0, 0).iend().build();
    let limits = Limits {
        max_pixels: 999_999,
        ..Limits::default()
    };
    assert_eq!(
        Png::parse_with_limits(&png, limits).unwrap_err(),
        Error::LimitExceeded {
            pixels: 1_000_000,
            limit: 999_999
        }
    );
}

#[test]
fn a_huge_header_over_tiny_data_fails_without_allocating() {
    // Claims 16384×16384 RGBA16 (2 GiB of pixels) but holds 6 bytes.
    let png = PngBuilder::new()
        .ihdr(16384, 16384, 16, 6, 0)
        .idat(&GRAY_2X2)
        .iend()
        .build();
    assert!(matches!(
        paeth::decode(&png),
        Err(Error::ImageDataTooShort { actual: 6, .. })
    ));
}

// --- Ancillary chunks are warnings, not errors ---------------------------

#[test]
fn malformed_ancillary_chunks_are_skipped_with_a_warning() {
    let png = gray_2x2()
        .chunk(b"gAMA", &[0, 0])
        .chunk(b"tIME", &[0, 0, 13, 1, 0, 0, 0])
        .chunk(b"tRNS", &[0, 5])
        .chunk(b"tRNS", &[0, 6])
        .idat(&GRAY_2X2)
        .chunk(b"pHYs", &[0; 9])
        .iend()
        .build();
    let parsed = Png::parse(&png).unwrap();
    let warned: Vec<_> = parsed.warnings().iter().map(|w| w.chunk).collect();
    assert_eq!(
        warned,
        [
            ChunkType::gAMA,
            ChunkType::tIME,
            ChunkType::tRNS,
            ChunkType::pHYs
        ]
    );
    // The first tRNS counts; the second is a duplicate.
    assert_eq!(
        parsed.metadata().transparency,
        Some(paeth::metadata::Transparency::Gray(5))
    );
    assert!(parsed.decode().is_ok());
}

#[test]
fn gamma_after_plte_is_ignored() {
    let png = indexed(&[0; 6])
        .chunk(b"gAMA", &45455u32.to_be_bytes())
        .idat(&INDEXED_ROW)
        .iend()
        .build();
    let parsed = Png::parse(&png).unwrap();
    assert_eq!(parsed.metadata().gamma, None);
    assert_eq!(parsed.warnings().len(), 1);
}

#[test]
fn srgb_and_iccp_together() {
    let mut iccp = b"profile\0\0".to_vec();
    iccp.extend(zlib(b"not really a profile"));
    let png = gray_2x2()
        .chunk(b"sRGB", &[0])
        .chunk(b"iCCP", &iccp)
        .idat(&GRAY_2X2)
        .iend()
        .build();
    let parsed = Png::parse(&png).unwrap();
    assert!(parsed.metadata().srgb.is_some());
    assert!(parsed.metadata().icc_profile.is_none());
    assert_eq!(parsed.warnings().len(), 1);
}
