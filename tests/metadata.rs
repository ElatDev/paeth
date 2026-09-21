//! Ancillary chunks reach `Metadata` with the values the file holds.
//!
//! The parsers have unit tests of their own; these check the wiring, on
//! a file carrying one of everything, and then on real PngSuite files.

mod support;

use paeth::chunk::ChunkType;
use paeth::metadata::{Background, RenderingIntent, Transparency};
use paeth::{Png, metadata};
use support::{PngBuilder, zlib};

/// A 2×1 8-bit RGB image carrying every ancillary chunk paeth reads,
/// in the order the spec requires.
fn everything() -> Vec<u8> {
    let mut ztxt = b"Copyright\0\0".to_vec();
    ztxt.extend(zlib(b"Copyright 2026"));
    let mut itxt = b"Title\0\x01\x00de\0Titel\0".to_vec();
    itxt.extend(zlib("Grüße".as_bytes()));

    PngBuilder::new()
        .ihdr(2, 1, 8, 2, 0)
        // Before PLTE.
        .chunk(b"gAMA", &45455u32.to_be_bytes())
        .chunk(
            b"cHRM",
            &[31270u32, 32900, 64000, 33000, 30000, 60000, 15000, 6000]
                .iter()
                .flat_map(|v| v.to_be_bytes())
                .collect::<Vec<_>>(),
        )
        .chunk(b"sRGB", &[2])
        .chunk(b"cICP", &[9, 16, 0, 1])
        .chunk(
            b"mDCV",
            &[
                &[34000u16, 16000, 13250, 34500, 7500, 3000, 15635, 16450]
                    .iter()
                    .flat_map(|v| v.to_be_bytes())
                    .collect::<Vec<_>>()[..],
                &10_000_000u32.to_be_bytes(),
                &1u32.to_be_bytes(),
            ]
            .concat(),
        )
        .chunk(
            b"cLLI",
            &[
                &10_000_000u32.to_be_bytes()[..],
                &4_000_000u32.to_be_bytes(),
            ]
            .concat(),
        )
        .chunk(b"sBIT", &[5, 6, 7])
        // A suggested palette, then the chunks that depend on one.
        .chunk(b"PLTE", &[1, 2, 3, 4, 5, 6])
        .chunk(b"tRNS", &[0, 200, 0, 201, 0, 202])
        .chunk(b"bKGD", &[0, 10, 0, 20, 0, 30])
        .chunk(b"hIST", &[0, 7, 0, 9])
        // Before IDAT.
        .chunk(b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1])
        .chunk(b"sPLT", b"reduced\0\x08\x01\x02\x03\x04\x00\x05")
        .chunk(b"eXIf", b"II*\0exif data")
        .chunk(b"acTL", &[0, 0, 0, 3, 0, 0, 0, 0])
        // Anywhere.
        .chunk(b"tIME", &[0x07, 0xEA, 9, 20, 22, 4, 60])
        .chunk(b"tEXt", b"Author\0W. Marshall")
        .chunk(b"zTXt", &ztxt)
        .chunk(b"iTXt", &itxt)
        .idat(&[0, 10, 20, 30, 40, 50, 60])
        .iend()
        .build()
}

#[test]
fn every_ancillary_chunk_reaches_metadata() {
    let bytes = everything();
    let png = Png::parse(&bytes).unwrap();
    assert_eq!(png.warnings(), [], "no chunk should have been skipped");
    let m = png.metadata();

    assert_eq!(m.gamma, Some(45455));
    assert_eq!(
        m.chromaticities,
        Some(metadata::Chromaticities {
            white: (31270, 32900),
            red: (64000, 33000),
            green: (30000, 60000),
            blue: (15000, 6000),
        })
    );
    assert_eq!(m.srgb, Some(RenderingIntent::Saturation));
    assert_eq!(
        m.cicp,
        Some(metadata::Cicp {
            color_primaries: 9,
            transfer_function: 16,
            matrix_coefficients: 0,
            full_range: true,
        })
    );
    assert_eq!(
        m.mastering_display,
        Some(metadata::MasteringDisplay {
            primaries: [(34000, 16000), (13250, 34500), (7500, 3000)],
            white_point: (15635, 16450),
            max_luminance: 10_000_000,
            min_luminance: 1,
        })
    );
    assert_eq!(
        m.content_light_level,
        Some(metadata::ContentLightLevel {
            max_content: 10_000_000,
            max_frame_average: 4_000_000,
        })
    );
    assert_eq!(m.significant_bits.as_deref(), Some(&[5, 6, 7][..]));
    assert_eq!(png.palette(), Some(&[[1, 2, 3], [4, 5, 6]][..]));
    assert_eq!(m.transparency, Some(Transparency::Rgb(200, 201, 202)));
    assert_eq!(m.background, Some(Background::Rgb(10, 20, 30)));
    assert_eq!(m.histogram.as_deref(), Some(&[7, 9][..]));
    assert_eq!(
        m.physical_dimensions,
        Some(metadata::PhysicalDimensions {
            x: 2835,
            y: 2835,
            per_metre: true,
        })
    );
    assert_eq!(
        m.suggested_palettes,
        [metadata::SuggestedPalette {
            name: "reduced".to_string(),
            sample_depth: 8,
            entries: vec![metadata::SuggestedColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 4,
                frequency: 5,
            }],
        }]
    );
    assert_eq!(m.exif.as_deref(), Some(&b"II*\0exif data"[..]));
    assert_eq!(
        m.animation,
        Some(metadata::Animation {
            frames: 3,
            plays: 0
        })
    );
    assert!(png.is_animated());
    assert_eq!(
        m.modified,
        Some(metadata::Timestamp {
            year: 2026,
            month: 9,
            day: 20,
            hour: 22,
            minute: 4,
            second: 60, // a leap second is legal
        })
    );

    let text: Vec<_> = m
        .text
        .iter()
        .map(|t| (t.chunk, t.keyword.as_str(), t.text.as_str()))
        .collect();
    assert_eq!(
        text,
        [
            (ChunkType::tEXt, "Author", "W. Marshall"),
            (ChunkType::zTXt, "Copyright", "Copyright 2026"),
            (ChunkType::iTXt, "Title", "Grüße"),
        ]
    );
    assert_eq!(m.text[2].language, "de");
    assert_eq!(m.text[2].translated_keyword, "Titel");

    // The pixels are unaffected by any of it: no gamma, no background
    // compositing, and tRNS keys out the matching pixel.
    let image = png.decode().unwrap();
    assert_eq!(image.pixels, [10, 20, 30, 255, 40, 50, 60, 255]);
}

#[test]
fn an_icc_profile_is_decompressed() {
    let mut iccp = b"sRGB IEC61966-2.1\0\0".to_vec();
    iccp.extend(zlib(&[0xAB; 3000]));
    let png = PngBuilder::new()
        .ihdr(1, 1, 8, 0, 0)
        .chunk(b"iCCP", &iccp)
        .idat(&[0, 7])
        .iend()
        .build();
    let parsed = Png::parse(&png).unwrap();
    let profile = parsed.metadata().icc_profile.as_ref().unwrap();
    assert_eq!(profile.name, "sRGB IEC61966-2.1");
    assert_eq!(profile.profile, vec![0xAB; 3000]);
    assert_eq!(parsed.warnings(), []);
}

#[test]
fn pngsuite_files_carry_the_metadata_they_advertise() {
    let files: std::collections::BTreeMap<_, _> = support::suite_files().into_iter().collect();
    let meta = |name: &str| {
        let bytes = files[name].clone();
        let png = Png::parse(&bytes).unwrap();
        assert_eq!(png.warnings(), [], "{name}");
        png.metadata().clone()
    };

    // cm7n0g04: "modified date, 1970"
    assert_eq!(
        meta("cm7n0g04.png").modified,
        Some(metadata::Timestamp {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        })
    );
    // cs3n2c16: "color, 13 significant bits"
    assert_eq!(
        meta("cs3n2c16.png").significant_bits.as_deref(),
        Some(&[13, 13, 13][..])
    );
    // ccwn2c08: "chroma chunk w:0.3127,0.3290 r:0.64,0.33 ..." with gamma 1.0
    let ccwn = meta("ccwn2c08.png");
    assert_eq!(ccwn.gamma, Some(100_000));
    let chrm = ccwn.chromaticities.unwrap();
    assert_eq!(chrm.white, (31270, 32900));
    assert_eq!(chrm.red, (64000, 33000));
    assert_eq!(chrm.blue, (15000, 6000));
    // cdfn2c08: "physical pixel dimensions, 8x32 flat pixels"
    let cdfn = meta("cdfn2c08.png").physical_dimensions.unwrap();
    assert_eq!((cdfn.x, cdfn.y, cdfn.per_metre), (1, 4, false));
    // ch1n3p04: "histogram, 15 colors"
    assert_eq!(meta("ch1n3p04.png").histogram.unwrap().len(), 15);
    // ps1n2c16: "suggested palette, 1 byte depth"
    let splt = meta("ps1n2c16.png").suggested_palettes;
    assert_eq!(splt.len(), 1);
    assert_eq!(splt[0].sample_depth, 8);
    // exif2c08: Exif data starting with a TIFF header. This one is
    // big-endian ("MM"), which is one reason the bytes are handed back
    // as they are rather than interpreted.
    let exif = meta("exif2c08.png").exif.unwrap();
    assert_eq!(&exif[..4], b"MM\0*");
    // tbbn3p08: transparency in a palette
    assert!(matches!(
        meta("tbbn3p08.png").transparency,
        Some(Transparency::Palette(_))
    ));
    // ctzn0g04: compressed text, six entries
    let text = meta("ctzn0g04.png").text;
    assert_eq!(text.len(), 6);
    assert_eq!(text[0].keyword, "Title");
    assert_eq!(text[0].text, "PngSuite");
    assert!(text.iter().any(|t| t.chunk == ChunkType::zTXt));
    // cten0g04: international text, translated keywords
    let itxt = meta("cten0g04.png").text;
    assert!(itxt.iter().all(|t| t.chunk == ChunkType::iTXt));
    assert!(itxt.iter().any(|t| !t.language.is_empty()));
    // bgbn4a08: "black background", 8-bit gray+alpha.
    assert_eq!(meta("bgbn4a08.png").background, Some(Background::Gray(0)));
    // bgyn6a16: "yellow background", 16-bit RGBA.
    assert_eq!(
        meta("bgyn6a16.png").background,
        Some(Background::Rgb(65535, 65535, 0))
    );
    // bgan6a16: the same image with no bKGD at all.
    assert_eq!(meta("bgan6a16.png").background, None);
}
