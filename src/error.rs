use std::fmt;

use crate::chunk::ChunkType;
use crate::header::ColorType;

/// Why a file was rejected.
///
/// Every variant is a hard error: the decoder does not guess at pixels
/// for a file the specification says is broken. Problems in ancillary
/// chunks, which the specification lets a decoder skip, are reported as
/// [`Warning`](crate::Warning)s instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The first eight bytes are not the PNG signature.
    Signature(SignatureError),
    /// The file ends partway through the chunk that starts at `offset`.
    Truncated {
        /// Byte offset of the incomplete chunk.
        offset: usize,
    },
    /// A chunk declares a length above 2³¹−1, which the spec forbids.
    ChunkTooLong {
        /// Byte offset of the chunk.
        offset: usize,
        /// The declared length.
        length: u32,
    },
    /// A chunk type contains something other than ASCII letters.
    InvalidChunkType {
        /// Byte offset of the chunk.
        offset: usize,
        /// The four type bytes as found.
        bytes: [u8; 4],
    },
    /// The CRC stored after a chunk does not match its contents.
    CrcMismatch {
        /// The chunk whose checksum failed.
        chunk: ChunkType,
        /// Byte offset of the chunk.
        offset: usize,
        /// CRC stored in the file.
        stored: u32,
        /// CRC computed over the chunk type and data.
        computed: u32,
    },
    /// The first chunk is not IHDR.
    FirstChunkNotIhdr(ChunkType),
    /// A chunk that may appear only once appears again.
    DuplicateChunk(ChunkType),
    /// A critical chunk appears somewhere the spec does not allow.
    MisplacedChunk {
        /// The misplaced chunk.
        chunk: ChunkType,
        /// The ordering rule it breaks.
        rule: &'static str,
    },
    /// A chunk the image cannot be decoded without is absent.
    MissingChunk(ChunkType),
    /// A critical chunk this decoder does not know. The spec requires
    /// rejecting these, since the image may depend on it.
    UnknownCriticalChunk(ChunkType),
    /// A critical chunk has the wrong length for its type.
    BadChunkLength {
        /// The chunk.
        chunk: ChunkType,
        /// Its length in bytes.
        length: usize,
    },
    /// Width or height is zero or above 2³¹−1.
    InvalidDimensions {
        /// Width from IHDR.
        width: u32,
        /// Height from IHDR.
        height: u32,
    },
    /// IHDR names a color type other than 0, 2, 3, 4 or 6.
    InvalidColorType(u8),
    /// The bit depth is not allowed for this color type.
    InvalidBitDepth {
        /// Color type from IHDR.
        color_type: ColorType,
        /// Bit depth from IHDR.
        bit_depth: u8,
    },
    /// IHDR compression method is not 0 (zlib).
    InvalidCompressionMethod(u8),
    /// IHDR filter method is not 0 (adaptive, five filter types).
    InvalidFilterMethod(u8),
    /// IHDR interlace method is not 0 (none) or 1 (Adam7).
    InvalidInterlaceMethod(u8),
    /// A PLTE chunk in a grayscale image, where the spec forbids one.
    UnexpectedPalette(ColorType),
    /// PLTE has more entries than the bit depth can index.
    PaletteTooLarge {
        /// Entries in PLTE.
        entries: usize,
        /// Bit depth from IHDR.
        bit_depth: u8,
    },
    /// The zlib stream inside IDAT is corrupt.
    CorruptImageData(String),
    /// The zlib stream ended before every scanline was delivered.
    ImageDataTooShort {
        /// Bytes the header says the scanlines need.
        expected: usize,
        /// Bytes the stream produced.
        actual: usize,
    },
    /// The zlib stream produced more bytes than the scanlines need.
    ImageDataTooLong {
        /// Bytes the header says the scanlines need.
        expected: usize,
    },
    /// Compressed bytes follow the end of the zlib stream inside IDAT.
    TrailingImageData {
        /// How many bytes follow the stream.
        bytes: usize,
    },
    /// A scanline starts with a filter type other than 0–4.
    InvalidFilterType {
        /// The filter byte found.
        filter: u8,
        /// Scanline index in the order they are stored (all Adam7 passes
        /// counted together).
        row: usize,
    },
    /// A pixel refers to a palette entry that does not exist.
    PaletteIndexOutOfRange {
        /// The offending index.
        index: u8,
        /// Entries in PLTE.
        palette_len: usize,
    },
    /// The image is larger than the configured [`Limits`](crate::Limits).
    LimitExceeded {
        /// Pixels in the image (width × height).
        pixels: u64,
        /// The configured maximum.
        limit: u64,
    },
}

/// How the eight-byte signature is wrong.
///
/// The PNG signature was designed so that the classic ways of mangling
/// a binary file in transit each leave a recognisable mark on it. When
/// the damage matches one of those, the error says which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignatureError {
    /// Fewer than eight bytes of input.
    TooShort,
    /// The leading 0x89 arrived as 0x09: the file went through a 7-bit
    /// channel that cleared the high bit of every byte.
    HighBitStripped,
    /// The CR and LF bytes were rewritten by a text-mode transfer.
    LineEndingsConverted(LineEndingConversion),
    /// Any other mismatch. Usually the file is not a PNG at all.
    Mismatch,
}

/// The newline conversion a text-mode transfer applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndingConversion {
    /// CR LF became LF (DOS to Unix).
    CrLfToLf,
    /// LF became CR LF (Unix to DOS).
    LfToCrLf,
    /// CR became LF (classic Mac to Unix).
    CrToLf,
    /// LF became CR (Unix to classic Mac).
    LfToCr,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Signature(e) => write!(f, "bad PNG signature: {e}"),
            Error::Truncated { offset } => {
                write!(f, "file ends partway through the chunk at offset {offset}")
            }
            Error::ChunkTooLong { offset, length } => write!(
                f,
                "chunk at offset {offset} declares length {length}, above the 2^31-1 maximum"
            ),
            Error::InvalidChunkType { offset, bytes } => write!(
                f,
                "chunk at offset {offset} has an invalid type \"{}\"",
                bytes.escape_ascii()
            ),
            Error::CrcMismatch {
                chunk,
                offset,
                stored,
                computed,
            } => write!(
                f,
                "{chunk} chunk at offset {offset} fails its CRC (stored {stored:08x}, computed {computed:08x})"
            ),
            Error::FirstChunkNotIhdr(chunk) => {
                write!(f, "the first chunk must be IHDR, found {chunk}")
            }
            Error::DuplicateChunk(chunk) => write!(f, "{chunk} chunk appears more than once"),
            Error::MisplacedChunk { chunk, rule } => write!(f, "misplaced {chunk} chunk: {rule}"),
            Error::MissingChunk(chunk) => write!(f, "required {chunk} chunk is missing"),
            Error::UnknownCriticalChunk(chunk) => write!(
                f,
                "unknown critical chunk {chunk}: the image may depend on it, so it cannot be skipped"
            ),
            Error::BadChunkLength { chunk, length } => {
                write!(f, "{chunk} chunk has an invalid length of {length} bytes")
            }
            Error::InvalidDimensions { width, height } => {
                write!(f, "invalid image dimensions {width}x{height}")
            }
            Error::InvalidColorType(t) => write!(f, "invalid color type {t}"),
            Error::InvalidBitDepth {
                color_type,
                bit_depth,
            } => write!(
                f,
                "bit depth {bit_depth} is not allowed for {color_type} (allowed: {:?})",
                color_type.allowed_bit_depths()
            ),
            Error::InvalidCompressionMethod(m) => write!(f, "invalid compression method {m}"),
            Error::InvalidFilterMethod(m) => write!(f, "invalid filter method {m}"),
            Error::InvalidInterlaceMethod(m) => write!(f, "invalid interlace method {m}"),
            Error::UnexpectedPalette(color_type) => {
                write!(f, "PLTE chunk is not allowed in a {color_type} image")
            }
            Error::PaletteTooLarge { entries, bit_depth } => write!(
                f,
                "palette has {entries} entries but a {bit_depth}-bit index can only reach {}",
                1u32 << bit_depth
            ),
            Error::CorruptImageData(msg) => write!(f, "corrupt zlib stream in IDAT: {msg}"),
            Error::ImageDataTooShort { expected, actual } => write!(
                f,
                "image data is too short: the scanlines need {expected} bytes, the stream holds {actual}"
            ),
            Error::ImageDataTooLong { expected } => write!(
                f,
                "image data is too long: the stream holds more than the {expected} bytes the scanlines need"
            ),
            Error::TrailingImageData { bytes } => write!(
                f,
                "{bytes} bytes of IDAT data follow the end of the zlib stream"
            ),
            Error::InvalidFilterType { filter, row } => {
                write!(f, "scanline {row} has invalid filter type {filter}")
            }
            Error::PaletteIndexOutOfRange { index, palette_len } => write!(
                f,
                "pixel uses palette index {index} but the palette has {palette_len} entries"
            ),
            Error::LimitExceeded { pixels, limit } => {
                write!(f, "image has {pixels} pixels, above the limit of {limit}")
            }
        }
    }
}

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignatureError::TooShort => f.write_str("file is shorter than the 8-byte signature"),
            SignatureError::HighBitStripped => {
                f.write_str("first byte is 0x09, not 0x89: the file passed through a 7-bit channel")
            }
            SignatureError::LineEndingsConverted(c) => write!(
                f,
                "line endings were converted ({c}): the file was transferred as text"
            ),
            SignatureError::Mismatch => f.write_str("not a PNG file"),
        }
    }
}

impl fmt::Display for LineEndingConversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LineEndingConversion::CrLfToLf => "CR LF to LF",
            LineEndingConversion::LfToCrLf => "LF to CR LF",
            LineEndingConversion::CrToLf => "CR to LF",
            LineEndingConversion::LfToCr => "LF to CR",
        })
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_problem_and_where_it_is() {
        let cases: [(Error, &str); 8] = [
            (
                Error::Signature(SignatureError::HighBitStripped),
                "bad PNG signature: first byte is 0x09, not 0x89: the file passed through a 7-bit channel",
            ),
            (
                Error::Signature(SignatureError::LineEndingsConverted(
                    LineEndingConversion::CrLfToLf,
                )),
                "bad PNG signature: line endings were converted (CR LF to LF): the file was transferred as text",
            ),
            (
                Error::Truncated { offset: 91 },
                "file ends partway through the chunk at offset 91",
            ),
            (
                Error::CrcMismatch {
                    chunk: ChunkType::IHDR,
                    offset: 8,
                    stored: 0x4353_554D,
                    computed: 0x5611_2528,
                },
                "IHDR chunk at offset 8 fails its CRC (stored 4353554d, computed 56112528)",
            ),
            (
                Error::InvalidBitDepth {
                    color_type: ColorType::Rgb,
                    bit_depth: 3,
                },
                "bit depth 3 is not allowed for RGB (allowed: [8, 16])",
            ),
            (
                Error::PaletteTooLarge {
                    entries: 3,
                    bit_depth: 1,
                },
                "palette has 3 entries but a 1-bit index can only reach 2",
            ),
            (
                Error::ImageDataTooShort {
                    expected: 79,
                    actual: 78,
                },
                "image data is too short: the scanlines need 79 bytes, the stream holds 78",
            ),
            (
                Error::InvalidFilterType { filter: 5, row: 12 },
                "scanline 12 has invalid filter type 5",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn every_variant_says_something() {
        // Display must never come back empty, whatever the variant.
        let errors = [
            Error::Signature(SignatureError::TooShort),
            Error::Signature(SignatureError::Mismatch),
            Error::ChunkTooLong {
                offset: 8,
                length: 0x8000_0000,
            },
            Error::InvalidChunkType {
                offset: 8,
                bytes: *b"IH1R",
            },
            Error::FirstChunkNotIhdr(ChunkType::gAMA),
            Error::DuplicateChunk(ChunkType::PLTE),
            Error::MisplacedChunk {
                chunk: ChunkType::IDAT,
                rule: "IDAT chunks must be consecutive",
            },
            Error::MissingChunk(ChunkType::IEND),
            Error::UnknownCriticalChunk(ChunkType(*b"CRIT")),
            Error::BadChunkLength {
                chunk: ChunkType::IEND,
                length: 1,
            },
            Error::InvalidDimensions {
                width: 0,
                height: 1,
            },
            Error::InvalidColorType(9),
            Error::InvalidCompressionMethod(1),
            Error::InvalidFilterMethod(1),
            Error::InvalidInterlaceMethod(2),
            Error::UnexpectedPalette(ColorType::Grayscale),
            Error::CorruptImageData("invalid stored block lengths".into()),
            Error::ImageDataTooLong { expected: 6 },
            Error::TrailingImageData { bytes: 4 },
            Error::PaletteIndexOutOfRange {
                index: 1,
                palette_len: 1,
            },
            Error::LimitExceeded {
                pixels: 1_000_000,
                limit: 999_999,
            },
        ];
        for error in errors {
            let text = error.to_string();
            assert!(text.len() > 10, "{error:?} printed as {text:?}");
            assert!(!text.ends_with('.'), "{text:?}");
        }
        for conversion in [
            LineEndingConversion::CrLfToLf,
            LineEndingConversion::LfToCrLf,
            LineEndingConversion::CrToLf,
            LineEndingConversion::LfToCr,
        ] {
            assert!(conversion.to_string().contains("to"));
        }
    }
}
