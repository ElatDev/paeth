//! Ancillary chunks (PNG spec, section 11.3).
//!
//! These are parsed and reported but never applied: the decoder hands
//! back the stored pixel values, with no gamma correction, no color
//! management and no compositing against a background. The one
//! exception is `tRNS`, which is how the file says which pixels are
//! transparent, and which the decoder uses to produce the alpha channel.
//!
//! A malformed ancillary chunk does not make the image unreadable. The
//! spec lets a decoder skip it, so each parser here returns a message
//! that becomes a [`Warning`](crate::Warning), and the chunk is dropped.

use std::io::Read;

use flate2::read::ZlibDecoder;

use crate::chunk::ChunkType;
use crate::header::{ColorType, Header};

/// Everything the ancillary chunks said about the image.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Metadata {
    /// `tRNS`: which pixels are transparent. Already applied to the
    /// decoded alpha channel.
    pub transparency: Option<Transparency>,
    /// `gAMA`: image gamma × 100000, so 45455 means 1/2.2.
    pub gamma: Option<u32>,
    /// `cHRM`: primary chromaticities.
    pub chromaticities: Option<Chromaticities>,
    /// `sRGB`: the image is in sRGB, with this rendering intent.
    pub srgb: Option<RenderingIntent>,
    /// `iCCP`: an embedded ICC profile.
    pub icc_profile: Option<IccProfile>,
    /// `cICP`: coding-independent code points (PNG Third Edition).
    pub cicp: Option<Cicp>,
    /// `mDCV`: mastering display color volume (PNG Third Edition).
    pub mastering_display: Option<MasteringDisplay>,
    /// `cLLI`: content light level (PNG Third Edition).
    pub content_light_level: Option<ContentLightLevel>,
    /// `sBIT`: significant bits per channel in the original data.
    pub significant_bits: Option<Vec<u8>>,
    /// `bKGD`: suggested background color.
    pub background: Option<Background>,
    /// `hIST`: approximate usage frequency of each palette entry.
    pub histogram: Option<Vec<u16>>,
    /// `pHYs`: intended pixel size or aspect ratio.
    pub physical_dimensions: Option<PhysicalDimensions>,
    /// `tIME`: when the image was last modified.
    pub modified: Option<Timestamp>,
    /// `tEXt`, `zTXt` and `iTXt`, in file order.
    pub text: Vec<Text>,
    /// `sPLT`: suggested reduced palettes.
    pub suggested_palettes: Vec<SuggestedPalette>,
    /// `eXIf`: raw Exif data.
    pub exif: Option<Vec<u8>>,
    /// `acTL`: present in animated PNGs. Only the default image is decoded.
    pub animation: Option<Animation>,
}

/// The `tRNS` chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transparency {
    /// Grayscale images: pixels with exactly this gray sample are fully
    /// transparent.
    Gray(u16),
    /// RGB images: pixels with exactly this color are fully transparent.
    Rgb(u16, u16, u16),
    /// Indexed images: alpha for the first `len()` palette entries. The
    /// rest are opaque.
    Palette(Vec<u8>),
}

/// The `cHRM` chunk, each coordinate × 100000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chromaticities {
    /// White point (x, y).
    pub white: (u32, u32),
    /// Red primary (x, y).
    pub red: (u32, u32),
    /// Green primary (x, y).
    pub green: (u32, u32),
    /// Blue primary (x, y).
    pub blue: (u32, u32),
}

/// The `sRGB` rendering intent (ICC-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum RenderingIntent {
    Perceptual,
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

/// The `iCCP` chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IccProfile {
    /// The profile name (Latin-1).
    pub name: String,
    /// The decompressed profile. Not parsed or applied.
    pub profile: Vec<u8>,
}

/// The `cICP` chunk: ITU-T H.273 code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Cicp {
    pub color_primaries: u8,
    pub transfer_function: u8,
    pub matrix_coefficients: u8,
    pub full_range: bool,
}

/// The `mDCV` chunk. Chromaticities × 50000, luminance × 10000 cd/m².
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct MasteringDisplay {
    pub primaries: [(u16, u16); 3],
    pub white_point: (u16, u16),
    pub max_luminance: u32,
    pub min_luminance: u32,
}

/// The `cLLI` chunk, × 10000 cd/m².
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ContentLightLevel {
    pub max_content: u32,
    pub max_frame_average: u32,
}

/// The `bKGD` chunk, in the image's own sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Background {
    Gray(u16),
    Rgb(u16, u16, u16),
    PaletteIndex(u8),
}

/// The `pHYs` chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalDimensions {
    /// Pixels per unit, horizontally.
    pub x: u32,
    /// Pixels per unit, vertically.
    pub y: u32,
    /// `true` if the unit is the metre; otherwise only the aspect ratio
    /// is meaningful.
    pub per_metre: bool,
}

/// The `tIME` chunk, in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// One text chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    /// Which chunk it came from: `tEXt`, `zTXt` or `iTXt`.
    pub chunk: ChunkType,
    /// The keyword, such as `Title` or `Software`.
    pub keyword: String,
    /// The text, decompressed if it was compressed.
    pub text: String,
    /// `iTXt` only: the language tag, possibly empty.
    pub language: String,
    /// `iTXt` only: the keyword translated into that language.
    pub translated_keyword: String,
}

/// One `sPLT` chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestedPalette {
    /// The palette name.
    pub name: String,
    /// 8 or 16.
    pub sample_depth: u8,
    /// The entries, at `sample_depth` precision.
    pub entries: Vec<SuggestedColor>,
}

/// One entry of a suggested palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct SuggestedColor {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub alpha: u16,
    pub frequency: u16,
}

/// The `acTL` chunk of an animated PNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Animation {
    /// Number of frames.
    pub frames: u32,
    /// Times to loop; 0 means forever.
    pub plays: u32,
}

/// What a chunk parser needs to know about the rest of the file.
pub(crate) struct Context<'a> {
    pub header: &'a Header,
    pub palette: Option<&'a [[u8; 3]]>,
    pub max_decompressed: usize,
}

type Parsed<T> = Result<T, String>;

fn exact<const N: usize>(data: &[u8]) -> Parsed<[u8; N]> {
    data.try_into()
        .map_err(|_| format!("length is {}, expected {N}", data.len()))
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// The largest sample value the header's bit depth can hold.
fn sample_max(header: &Header) -> u16 {
    ((1u32 << header.bit_depth) - 1) as u16
}

pub(crate) fn parse_trns(data: &[u8], cx: &Context<'_>) -> Parsed<Transparency> {
    match cx.header.color_type {
        ColorType::Grayscale => {
            let gray = be16(&exact::<2>(data)?);
            if gray > sample_max(cx.header) {
                return Err(format!(
                    "gray value {gray} does not fit in {} bits",
                    cx.header.bit_depth
                ));
            }
            Ok(Transparency::Gray(gray))
        }
        ColorType::Rgb => {
            let b = exact::<6>(data)?;
            Ok(Transparency::Rgb(
                be16(&b[0..]),
                be16(&b[2..]),
                be16(&b[4..]),
            ))
        }
        ColorType::Indexed => {
            let palette = cx.palette.ok_or("tRNS must come after PLTE")?;
            if data.len() > palette.len() {
                return Err(format!(
                    "{} alpha values for a {}-entry palette",
                    data.len(),
                    palette.len()
                ));
            }
            Ok(Transparency::Palette(data.to_vec()))
        }
        ColorType::GrayscaleAlpha | ColorType::Rgba => Err(format!(
            "not allowed in a {} image, which already has alpha",
            cx.header.color_type
        )),
    }
}

pub(crate) fn parse_gama(data: &[u8]) -> Parsed<u32> {
    let gamma = be32(&exact::<4>(data)?);
    if gamma == 0 {
        return Err("gamma of zero".into());
    }
    Ok(gamma)
}

pub(crate) fn parse_chrm(data: &[u8]) -> Parsed<Chromaticities> {
    let b = exact::<32>(data)?;
    let pair = |i: usize| (be32(&b[i..]), be32(&b[i + 4..]));
    Ok(Chromaticities {
        white: pair(0),
        red: pair(8),
        green: pair(16),
        blue: pair(24),
    })
}

pub(crate) fn parse_srgb(data: &[u8]) -> Parsed<RenderingIntent> {
    Ok(match exact::<1>(data)?[0] {
        0 => RenderingIntent::Perceptual,
        1 => RenderingIntent::RelativeColorimetric,
        2 => RenderingIntent::Saturation,
        3 => RenderingIntent::AbsoluteColorimetric,
        other => return Err(format!("unknown rendering intent {other}")),
    })
}

pub(crate) fn parse_iccp(data: &[u8], cx: &Context<'_>) -> Parsed<IccProfile> {
    let (name, rest) = keyword(data)?;
    let (&method, compressed) = rest.split_first().ok_or("missing compression method")?;
    if method != 0 {
        return Err(format!("unknown compression method {method}"));
    }
    Ok(IccProfile {
        name,
        profile: inflate(compressed, cx.max_decompressed)?,
    })
}

pub(crate) fn parse_cicp(data: &[u8]) -> Parsed<Cicp> {
    let [
        color_primaries,
        transfer_function,
        matrix_coefficients,
        full_range,
    ] = exact::<4>(data)?;
    if matrix_coefficients != 0 {
        return Err("matrix coefficients must be 0 (RGB) in PNG".into());
    }
    Ok(Cicp {
        color_primaries,
        transfer_function,
        matrix_coefficients,
        full_range: match full_range {
            0 => false,
            1 => true,
            other => return Err(format!("full-range flag must be 0 or 1, found {other}")),
        },
    })
}

pub(crate) fn parse_mdcv(data: &[u8]) -> Parsed<MasteringDisplay> {
    let b = exact::<24>(data)?;
    let pair = |i: usize| (be16(&b[i..]), be16(&b[i + 2..]));
    Ok(MasteringDisplay {
        primaries: [pair(0), pair(4), pair(8)],
        white_point: pair(12),
        max_luminance: be32(&b[16..]),
        min_luminance: be32(&b[20..]),
    })
}

pub(crate) fn parse_clli(data: &[u8]) -> Parsed<ContentLightLevel> {
    let b = exact::<8>(data)?;
    Ok(ContentLightLevel {
        max_content: be32(&b[0..]),
        max_frame_average: be32(&b[4..]),
    })
}

pub(crate) fn parse_sbit(data: &[u8], header: &Header) -> Parsed<Vec<u8>> {
    let (channels, max) = match header.color_type {
        // Indexed images give significant bits of the palette's RGB,
        // which is always 8-bit.
        ColorType::Indexed => (3, 8),
        other => (other.channels(), header.bit_depth),
    };
    if data.len() != channels {
        return Err(format!(
            "length is {}, expected {channels} for {}",
            data.len(),
            header.color_type
        ));
    }
    if let Some(&bad) = data.iter().find(|&&b| b == 0 || b > max) {
        return Err(format!("{bad} significant bits is outside 1..={max}"));
    }
    Ok(data.to_vec())
}

pub(crate) fn parse_bkgd(data: &[u8], cx: &Context<'_>) -> Parsed<Background> {
    match cx.header.color_type {
        ColorType::Grayscale | ColorType::GrayscaleAlpha => {
            let gray = be16(&exact::<2>(data)?);
            if gray > sample_max(cx.header) {
                return Err(format!("gray value {gray} does not fit the bit depth"));
            }
            Ok(Background::Gray(gray))
        }
        ColorType::Rgb | ColorType::Rgba => {
            let b = exact::<6>(data)?;
            Ok(Background::Rgb(be16(&b[0..]), be16(&b[2..]), be16(&b[4..])))
        }
        ColorType::Indexed => {
            let index = exact::<1>(data)?[0];
            let palette = cx.palette.ok_or("bKGD must come after PLTE")?;
            if usize::from(index) >= palette.len() {
                return Err(format!("palette index {index} is out of range"));
            }
            Ok(Background::PaletteIndex(index))
        }
    }
}

pub(crate) fn parse_hist(data: &[u8], cx: &Context<'_>) -> Parsed<Vec<u16>> {
    let palette = cx.palette.ok_or("hIST requires a PLTE before it")?;
    if data.len() != palette.len() * 2 {
        return Err(format!(
            "length is {}, expected 2 bytes for each of {} palette entries",
            data.len(),
            palette.len()
        ));
    }
    Ok(data.chunks_exact(2).map(be16).collect())
}

pub(crate) fn parse_phys(data: &[u8]) -> Parsed<PhysicalDimensions> {
    let b = exact::<9>(data)?;
    Ok(PhysicalDimensions {
        x: be32(&b[0..]),
        y: be32(&b[4..]),
        per_metre: match b[8] {
            0 => false,
            1 => true,
            other => return Err(format!("unknown unit {other}")),
        },
    })
}

pub(crate) fn parse_time(data: &[u8]) -> Parsed<Timestamp> {
    let b = exact::<7>(data)?;
    let t = Timestamp {
        year: be16(&b[0..]),
        month: b[2],
        day: b[3],
        hour: b[4],
        minute: b[5],
        second: b[6],
    };
    // Second 60 is allowed, for leap seconds.
    let valid = (1..=12).contains(&t.month)
        && (1..=31).contains(&t.day)
        && t.hour <= 23
        && t.minute <= 59
        && t.second <= 60;
    if !valid {
        return Err(format!(
            "invalid date {}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        ));
    }
    Ok(t)
}

pub(crate) fn parse_text(data: &[u8]) -> Parsed<Text> {
    let (keyword, text) = keyword(data)?;
    Ok(Text {
        chunk: ChunkType::tEXt,
        keyword,
        text: latin1(text),
        language: String::new(),
        translated_keyword: String::new(),
    })
}

pub(crate) fn parse_ztxt(data: &[u8], cx: &Context<'_>) -> Parsed<Text> {
    let (keyword, rest) = keyword(data)?;
    let (&method, compressed) = rest.split_first().ok_or("missing compression method")?;
    if method != 0 {
        return Err(format!("unknown compression method {method}"));
    }
    Ok(Text {
        chunk: ChunkType::zTXt,
        keyword,
        text: latin1(&inflate(compressed, cx.max_decompressed)?),
        language: String::new(),
        translated_keyword: String::new(),
    })
}

pub(crate) fn parse_itxt(data: &[u8], cx: &Context<'_>) -> Parsed<Text> {
    let (keyword, rest) = keyword(data)?;
    let [flag, method, rest @ ..] = rest else {
        return Err("missing compression flag and method".into());
    };
    let (language, rest) = split_nul(rest).ok_or("missing language tag terminator")?;
    let (translated, text) = split_nul(rest).ok_or("missing translated keyword terminator")?;
    if !language.is_ascii() {
        return Err("language tag is not ASCII".into());
    }
    let text = match (flag, method) {
        (0, _) => text.to_vec(),
        (1, 0) => inflate(text, cx.max_decompressed)?,
        (1, m) => return Err(format!("unknown compression method {m}")),
        (f, _) => return Err(format!("compression flag must be 0 or 1, found {f}")),
    };
    let utf8 = |bytes: Vec<u8>, what: &str| {
        String::from_utf8(bytes).map_err(|_| format!("{what} is not valid UTF-8"))
    };
    Ok(Text {
        chunk: ChunkType::iTXt,
        keyword,
        text: utf8(text, "text")?,
        language: latin1(language),
        translated_keyword: utf8(translated.to_vec(), "translated keyword")?,
    })
}

pub(crate) fn parse_splt(data: &[u8]) -> Parsed<SuggestedPalette> {
    let (name, rest) = keyword(data)?;
    let (&sample_depth, entries) = rest.split_first().ok_or("missing sample depth")?;
    let entry_len = match sample_depth {
        8 => 6,
        16 => 10,
        other => return Err(format!("sample depth must be 8 or 16, found {other}")),
    };
    if entries.len() % entry_len != 0 {
        return Err(format!(
            "{} bytes of entries is not a multiple of {entry_len}",
            entries.len()
        ));
    }
    let entries = entries
        .chunks_exact(entry_len)
        .map(|e| {
            if sample_depth == 8 {
                SuggestedColor {
                    red: e[0].into(),
                    green: e[1].into(),
                    blue: e[2].into(),
                    alpha: e[3].into(),
                    frequency: be16(&e[4..]),
                }
            } else {
                SuggestedColor {
                    red: be16(&e[0..]),
                    green: be16(&e[2..]),
                    blue: be16(&e[4..]),
                    alpha: be16(&e[6..]),
                    frequency: be16(&e[8..]),
                }
            }
        })
        .collect();
    Ok(SuggestedPalette {
        name,
        sample_depth,
        entries,
    })
}

pub(crate) fn parse_actl(data: &[u8]) -> Parsed<Animation> {
    let b = exact::<8>(data)?;
    let frames = be32(&b[0..]);
    if frames == 0 {
        return Err("zero frames".into());
    }
    Ok(Animation {
        frames,
        plays: be32(&b[4..]),
    })
}

fn split_nul(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let nul = data.iter().position(|&b| b == 0)?;
    Some((&data[..nul], &data[nul + 1..]))
}

/// Splits off a null-terminated keyword and checks it is 1–79 printable
/// Latin-1 characters (section 11.3.3.2).
fn keyword(data: &[u8]) -> Parsed<(String, &[u8])> {
    let (key, rest) = split_nul(data).ok_or("keyword is not null-terminated")?;
    if key.is_empty() || key.len() > 79 {
        return Err(format!("keyword length {} is outside 1..=79", key.len()));
    }
    if !key.iter().all(|&b| (32..=126).contains(&b) || b >= 161) {
        return Err("keyword contains unprintable characters".into());
    }
    Ok((latin1(key), rest))
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// Decompresses a zlib stream from a metadata chunk, refusing to produce
/// more than `limit` bytes so a small chunk cannot expand without bound.
fn inflate(compressed: &[u8], limit: usize) -> Parsed<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(compressed)
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("corrupt compressed data: {e}"))?;
    if out.len() > limit {
        return Err(format!(
            "decompresses to more than the {limit}-byte metadata limit"
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Interlace;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn header(color_type: ColorType, bit_depth: u8) -> Header {
        Header {
            width: 1,
            height: 1,
            bit_depth,
            color_type,
            interlace: Interlace::None,
        }
    }

    fn cx<'a>(header: &'a Header, palette: Option<&'a [[u8; 3]]>) -> Context<'a> {
        Context {
            header,
            palette,
            max_decompressed: 1024,
        }
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn trns_per_color_type() {
        let gray = header(ColorType::Grayscale, 4);
        assert_eq!(
            parse_trns(&[0, 9], &cx(&gray, None)),
            Ok(Transparency::Gray(9))
        );
        assert!(
            parse_trns(&[0, 16], &cx(&gray, None)).is_err(),
            "16 needs 5 bits"
        );

        let rgb = header(ColorType::Rgb, 16);
        assert_eq!(
            parse_trns(&[1, 2, 3, 4, 5, 6], &cx(&rgb, None)),
            Ok(Transparency::Rgb(0x0102, 0x0304, 0x0506))
        );

        let indexed = header(ColorType::Indexed, 8);
        let palette = [[0; 3]; 2];
        assert_eq!(
            parse_trns(&[10], &cx(&indexed, Some(&palette))),
            Ok(Transparency::Palette(vec![10]))
        );
        assert!(parse_trns(&[1, 2, 3], &cx(&indexed, Some(&palette))).is_err());
        assert!(parse_trns(&[1], &cx(&indexed, None)).is_err());

        let rgba = header(ColorType::Rgba, 8);
        assert!(parse_trns(&[0; 6], &cx(&rgba, None)).is_err());
    }

    #[test]
    fn text_chunks() {
        let t = parse_text(b"Title\0Caf\xe9").unwrap();
        assert_eq!((t.keyword.as_str(), t.text.as_str()), ("Title", "Café"));

        let h = header(ColorType::Rgb, 8);
        let mut z = b"Comment\0\0".to_vec();
        z.extend(zlib(b"squeezed"));
        assert_eq!(parse_ztxt(&z, &cx(&h, None)).unwrap().text, "squeezed");

        let mut i = b"Title\0\x01\x00fr\0Titre\0".to_vec();
        i.extend(zlib("Été".as_bytes()));
        let t = parse_itxt(&i, &cx(&h, None)).unwrap();
        assert_eq!(t.language, "fr");
        assert_eq!(t.translated_keyword, "Titre");
        assert_eq!(t.text, "Été");
    }

    #[test]
    fn keywords_are_validated() {
        assert!(parse_text(b"\0empty keyword").is_err());
        assert!(parse_text(b"no terminator").is_err());
        assert!(parse_text(&[b'k'; 80].iter().chain(b"\0x").copied().collect::<Vec<_>>()).is_err());
        assert!(parse_text(b"tab\there\0x").is_err());
        assert!(parse_text(&[b'k'; 79].iter().chain(b"\0x").copied().collect::<Vec<_>>()).is_ok());
    }

    #[test]
    fn decompression_is_capped() {
        let h = header(ColorType::Rgb, 8);
        let mut z = b"Bomb\0\0".to_vec();
        z.extend(zlib(&vec![b'a'; 1025]));
        let err = parse_ztxt(&z, &cx(&h, None)).unwrap_err();
        assert!(err.contains("limit"), "{err}");
    }

    #[test]
    fn corrupt_compressed_text_is_an_error_not_a_panic() {
        let h = header(ColorType::Rgb, 8);
        assert!(parse_ztxt(b"k\0\0\x78\x9c\xff\xff", &cx(&h, None)).is_err());
    }

    #[test]
    fn time_ranges() {
        assert!(parse_time(&[0x07, 0xEA, 9, 20, 23, 59, 60]).is_ok());
        assert!(parse_time(&[0x07, 0xEA, 13, 20, 0, 0, 0]).is_err());
        assert!(parse_time(&[0x07, 0xEA, 1, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn sbit_lengths_follow_the_color_type() {
        assert!(parse_sbit(&[5], &header(ColorType::Grayscale, 8)).is_ok());
        assert!(parse_sbit(&[5, 5, 5], &header(ColorType::Indexed, 2)).is_ok());
        assert!(parse_sbit(&[9, 5, 5], &header(ColorType::Indexed, 8)).is_err());
        assert!(parse_sbit(&[13, 13, 13, 13], &header(ColorType::Rgba, 16)).is_ok());
        assert!(parse_sbit(&[0, 5], &header(ColorType::GrayscaleAlpha, 8)).is_err());
    }

    #[test]
    fn hist_needs_one_entry_per_palette_color() {
        let h = header(ColorType::Indexed, 8);
        let palette = [[0; 3]; 3];
        assert_eq!(
            parse_hist(&[0, 1, 0, 2, 0, 3], &cx(&h, Some(&palette))),
            Ok(vec![1, 2, 3])
        );
        assert!(parse_hist(&[0, 1], &cx(&h, Some(&palette))).is_err());
    }
}
