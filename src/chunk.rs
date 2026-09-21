//! The chunk layer: the signature, then a stream of
//! `length · type · data · CRC` records (PNG spec, section 5).
//!
//! This module knows nothing about what chunks mean. It checks that the
//! file is framed correctly and that every checksum holds.

use std::fmt;

use crate::crc::Crc32;
use crate::error::{Error, LineEndingConversion, SignatureError};

/// The eight bytes every PNG file starts with.
pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// The spec caps chunk lengths at 2³¹−1 so they fit a signed 32-bit int.
const MAX_CHUNK_LENGTH: u32 = 0x7FFF_FFFF;

/// A four-letter chunk type such as `IHDR` or `tEXt`.
///
/// The case of each letter is a flag (spec section 5.4), exposed by the
/// `is_*` methods.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkType(pub [u8; 4]);

#[allow(non_upper_case_globals, missing_docs)]
impl ChunkType {
    pub const IHDR: ChunkType = ChunkType(*b"IHDR");
    pub const PLTE: ChunkType = ChunkType(*b"PLTE");
    pub const IDAT: ChunkType = ChunkType(*b"IDAT");
    pub const IEND: ChunkType = ChunkType(*b"IEND");
    pub const tRNS: ChunkType = ChunkType(*b"tRNS");
    pub const cHRM: ChunkType = ChunkType(*b"cHRM");
    pub const gAMA: ChunkType = ChunkType(*b"gAMA");
    pub const iCCP: ChunkType = ChunkType(*b"iCCP");
    pub const sBIT: ChunkType = ChunkType(*b"sBIT");
    pub const sRGB: ChunkType = ChunkType(*b"sRGB");
    pub const cICP: ChunkType = ChunkType(*b"cICP");
    pub const mDCV: ChunkType = ChunkType(*b"mDCV");
    pub const cLLI: ChunkType = ChunkType(*b"cLLI");
    pub const tEXt: ChunkType = ChunkType(*b"tEXt");
    pub const zTXt: ChunkType = ChunkType(*b"zTXt");
    pub const iTXt: ChunkType = ChunkType(*b"iTXt");
    pub const bKGD: ChunkType = ChunkType(*b"bKGD");
    pub const hIST: ChunkType = ChunkType(*b"hIST");
    pub const pHYs: ChunkType = ChunkType(*b"pHYs");
    pub const sPLT: ChunkType = ChunkType(*b"sPLT");
    pub const eXIf: ChunkType = ChunkType(*b"eXIf");
    pub const tIME: ChunkType = ChunkType(*b"tIME");
    pub const acTL: ChunkType = ChunkType(*b"acTL");
    pub const fcTL: ChunkType = ChunkType(*b"fcTL");
    pub const fdAT: ChunkType = ChunkType(*b"fdAT");
}

impl ChunkType {
    /// Critical chunks (uppercase first letter) are needed to display
    /// the image. A decoder that meets an unknown one must give up.
    pub fn is_critical(self) -> bool {
        self.0[0] & 0x20 == 0
    }

    /// Public chunks (uppercase second letter) are defined by the spec
    /// or registered; private ones are application-specific.
    pub fn is_public(self) -> bool {
        self.0[1] & 0x20 == 0
    }

    /// The third letter must be uppercase in this version of PNG.
    pub fn is_reserved_bit_valid(self) -> bool {
        self.0[2] & 0x20 == 0
    }

    /// Editors may copy safe-to-copy chunks (lowercase fourth letter)
    /// into a modified file without understanding them.
    pub fn is_safe_to_copy(self) -> bool {
        self.0[3] & 0x20 != 0
    }
}

impl fmt::Display for ChunkType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.escape_ascii())
    }
}

impl fmt::Debug for ChunkType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChunkType(\"{self}\")")
    }
}

/// One chunk, borrowed from the input, with its CRC already verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk<'a> {
    /// The four-letter type.
    pub kind: ChunkType,
    /// The chunk's data, without length, type or CRC.
    pub data: &'a [u8],
    /// Byte offset of the chunk's length field in the file.
    pub offset: usize,
    /// The stored CRC, which matched.
    pub crc: u32,
}

impl Chunk<'_> {
    /// Bytes the chunk occupies in the file, framing included.
    pub fn total_len(&self) -> usize {
        12 + self.data.len()
    }
}

/// Checks the signature and returns an iterator over the chunks that
/// follow it.
///
/// The iterator stops after IEND, at the end of the input, or after the
/// first error, whichever comes first. It does not check chunk order;
/// that is [`Png::parse`](crate::Png::parse)'s job.
///
/// # Errors
///
/// [`Error::Signature`] if the file does not start with the PNG
/// signature. Errors in the chunks themselves come from the iterator.
pub fn chunks(png: &[u8]) -> Result<Chunks<'_>, Error> {
    check_signature(png)?;
    Ok(Chunks {
        data: png,
        pos: SIGNATURE.len(),
        done: false,
    })
}

/// Iterator returned by [`chunks`].
#[derive(Clone, Debug)]
pub struct Chunks<'a> {
    data: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> Chunks<'a> {
    /// The bytes not yet read. After IEND, these are whatever trails
    /// the PNG datastream.
    pub fn remainder(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.pos == self.data.len() {
            return None;
        }
        let result = read_chunk(self.data, self.pos);
        match &result {
            Ok(chunk) => {
                self.pos += chunk.total_len();
                self.done = chunk.kind == ChunkType::IEND;
            }
            Err(_) => self.done = true,
        }
        Some(result)
    }
}

fn read_chunk(data: &[u8], offset: usize) -> Result<Chunk<'_>, Error> {
    let rest = &data[offset..];
    let truncated = Error::Truncated { offset };
    let (length, rest) = split_u32(rest).ok_or(truncated.clone())?;
    if length > MAX_CHUNK_LENGTH {
        return Err(Error::ChunkTooLong { offset, length });
    }
    let (type_bytes, rest) = rest.split_first_chunk::<4>().ok_or(truncated.clone())?;
    if !type_bytes.iter().all(u8::is_ascii_alphabetic) {
        return Err(Error::InvalidChunkType {
            offset,
            bytes: *type_bytes,
        });
    }
    let kind = ChunkType(*type_bytes);
    // Length is at most 2^31-1, so this fits a usize on any 32-bit target.
    let length = length as usize;
    if rest.len() < length {
        return Err(truncated);
    }
    let (payload, rest) = rest.split_at(length);
    let (stored, _) = split_u32(rest).ok_or(truncated)?;
    let computed = Crc32::new().update(type_bytes).update(payload).finish();
    if stored != computed {
        return Err(Error::CrcMismatch {
            chunk: kind,
            offset,
            stored,
            computed,
        });
    }
    Ok(Chunk {
        kind,
        data: payload,
        offset,
        crc: stored,
    })
}

fn split_u32(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let (head, rest) = bytes.split_first_chunk::<4>()?;
    Some((u32::from_be_bytes(*head), rest))
}

/// Checks the eight-byte signature, and when it is wrong, works out how.
///
/// # Errors
///
/// [`Error::Signature`], saying what kind of damage it found.
pub fn check_signature(data: &[u8]) -> Result<(), Error> {
    if data.starts_with(&SIGNATURE) {
        return Ok(());
    }
    Err(Error::Signature(diagnose_signature(data)))
}

fn diagnose_signature(data: &[u8]) -> SignatureError {
    if data.len() < SIGNATURE.len() && SIGNATURE.starts_with(data) {
        return SignatureError::TooShort;
    }
    if data.len() >= SIGNATURE.len() && data[0] == 0x09 && data[1..8] == SIGNATURE[1..] {
        return SignatureError::HighBitStripped;
    }
    const CONVERSIONS: [LineEndingConversion; 4] = [
        LineEndingConversion::CrLfToLf,
        LineEndingConversion::LfToCrLf,
        LineEndingConversion::CrToLf,
        LineEndingConversion::LfToCr,
    ];
    for conversion in CONVERSIONS {
        if data.starts_with(&convert_line_endings(&SIGNATURE, conversion)) {
            return SignatureError::LineEndingsConverted(conversion);
        }
    }
    SignatureError::Mismatch
}

/// Applies a text-mode newline conversion the way a naive transfer
/// program would.
fn convert_line_endings(bytes: &[u8], conversion: LineEndingConversion) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match conversion {
            LineEndingConversion::CrLfToLf if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') => {}
            LineEndingConversion::LfToCrLf if b == b'\n' => out.extend_from_slice(b"\r\n"),
            LineEndingConversion::CrToLf if b == b'\r' => out.push(b'\n'),
            LineEndingConversion::LfToCr if b == b'\n' => out.push(b'\r'),
            _ => out.push(b),
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::crc32;

    /// Frames `data` as a chunk with a correct CRC.
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        out
    }

    fn file(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        for c in chunks {
            out.extend_from_slice(c);
        }
        out
    }

    fn collect(png: &[u8]) -> Result<Vec<Chunk<'_>>, Error> {
        chunks(png)?.collect()
    }

    #[test]
    fn reads_chunks_in_order_and_stops_after_iend() {
        let mut png = file(&[
            chunk(b"IHDR", &[0; 13]),
            chunk(b"tEXt", b"a\0b"),
            chunk(b"IEND", &[]),
        ]);
        png.extend_from_slice(b"trailing garbage");
        let mut iter = chunks(&png).unwrap();
        let kinds: Vec<_> = iter.by_ref().map(|c| c.unwrap().kind).collect();
        assert_eq!(kinds, [ChunkType::IHDR, ChunkType::tEXt, ChunkType::IEND]);
        assert_eq!(iter.remainder(), b"trailing garbage");
    }

    #[test]
    fn records_offsets_and_crcs() {
        let png = file(&[chunk(b"IHDR", &[1; 13]), chunk(b"IEND", &[])]);
        let found = collect(&png).unwrap();
        assert_eq!(found[0].offset, 8);
        assert_eq!(found[0].data, &[1; 13]);
        assert_eq!(found[1].offset, 8 + 12 + 13);
        assert_eq!(found[1].crc, 0xAE42_6082);
    }

    #[test]
    fn missing_signature() {
        let png = chunk(b"IHDR", &[0; 13]);
        assert_eq!(
            chunks(&png).unwrap_err(),
            Error::Signature(SignatureError::Mismatch)
        );
    }

    #[test]
    fn empty_and_short_input() {
        for len in 0..8 {
            assert_eq!(
                check_signature(&SIGNATURE[..len]),
                Err(Error::Signature(SignatureError::TooShort)),
                "{len} bytes"
            );
        }
    }

    #[test]
    fn diagnoses_transfer_damage() {
        let mut seven_bit = SIGNATURE;
        seven_bit[0] = 0x09;
        assert_eq!(
            diagnose_signature(&seven_bit),
            SignatureError::HighBitStripped
        );

        let cases: [(&[u8], LineEndingConversion); 4] = [
            (b"\x89PNG\n\x1a\n", LineEndingConversion::CrLfToLf),
            (b"\x89PNG\r\r\n\x1a\r\n", LineEndingConversion::LfToCrLf),
            (b"\x89PNG\n\n\x1a\n", LineEndingConversion::CrToLf),
            (b"\x89PNG\r\r\x1a\r", LineEndingConversion::LfToCr),
        ];
        for (bytes, conversion) in cases {
            // Real damage continues past the signature, so pad it out.
            let mut data = bytes.to_vec();
            data.extend_from_slice(&[0; 16]);
            assert_eq!(
                diagnose_signature(&data),
                SignatureError::LineEndingsConverted(conversion)
            );
        }
    }

    #[test]
    fn a_jpeg_is_just_a_mismatch() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
        assert_eq!(diagnose_signature(&jpeg), SignatureError::Mismatch);
    }

    #[test]
    fn truncated_chunk() {
        let full = file(&[chunk(b"IHDR", &[0; 13])]);
        // Cut the file at every point inside the chunk: in the length,
        // the type, the data and the CRC.
        for cut in SIGNATURE.len() + 1..full.len() {
            let err = collect(&full[..cut]).unwrap_err();
            assert_eq!(err, Error::Truncated { offset: 8 }, "cut at {cut}");
        }
    }

    #[test]
    fn truncated_second_chunk_reports_its_own_offset() {
        let mut png = file(&[chunk(b"IHDR", &[0; 13]), chunk(b"IDAT", &[0; 10])]);
        png.truncate(png.len() - 1);
        assert_eq!(collect(&png).unwrap_err(), Error::Truncated { offset: 33 });
    }

    #[test]
    fn bad_crc() {
        let mut png = file(&[chunk(b"IHDR", &[0; 13])]);
        let last = png.len() - 1;
        png[last] ^= 0x01;
        match collect(&png).unwrap_err() {
            Error::CrcMismatch {
                chunk,
                offset,
                stored,
                computed,
            } => {
                assert_eq!(chunk, ChunkType::IHDR);
                assert_eq!(offset, 8);
                assert_eq!(stored ^ computed, 0x01);
            }
            other => panic!("expected a CRC error, got {other:?}"),
        }
    }

    #[test]
    fn crc_covers_the_type_not_just_the_data() {
        let mut png = file(&[chunk(b"tEXt", b"k\0v")]);
        png[12] = b'z'; // tEXt -> zEXt, data and CRC untouched
        assert!(matches!(
            collect(&png).unwrap_err(),
            Error::CrcMismatch { .. }
        ));
    }

    #[test]
    fn corrupted_data_byte_fails_crc() {
        let good = file(&[chunk(b"IDAT", &[7; 32])]);
        for i in 16..16 + 32 {
            let mut png = good.clone();
            png[i] ^= 0x80;
            assert!(
                matches!(collect(&png).unwrap_err(), Error::CrcMismatch { .. }),
                "flipped byte {i}"
            );
        }
    }

    #[test]
    fn length_above_2_pow_31_is_rejected() {
        let mut png = SIGNATURE.to_vec();
        png.extend_from_slice(&0x8000_0000u32.to_be_bytes());
        png.extend_from_slice(b"IDAT");
        assert_eq!(
            collect(&png).unwrap_err(),
            Error::ChunkTooLong {
                offset: 8,
                length: 0x8000_0000
            }
        );
    }

    #[test]
    fn huge_length_on_short_file_is_truncation_not_allocation() {
        let mut png = SIGNATURE.to_vec();
        png.extend_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        png.extend_from_slice(b"IDAT");
        assert_eq!(collect(&png).unwrap_err(), Error::Truncated { offset: 8 });
    }

    #[test]
    fn chunk_type_must_be_letters() {
        for bad in [*b"IH1R", *b"IH R", *b"\0HDR", *b"IHD\xC4"] {
            let png = file(&[chunk(&bad, &[])]);
            assert_eq!(
                collect(&png).unwrap_err(),
                Error::InvalidChunkType {
                    offset: 8,
                    bytes: bad
                }
            );
        }
    }

    #[test]
    fn nothing_after_the_signature_is_an_empty_stream() {
        assert_eq!(collect(&SIGNATURE).unwrap(), []);
    }

    #[test]
    fn property_bits() {
        let idat = ChunkType::IDAT;
        assert!(idat.is_critical() && idat.is_public() && idat.is_reserved_bit_valid());
        assert!(!idat.is_safe_to_copy());
        let text = ChunkType::tEXt;
        assert!(!text.is_critical() && text.is_public() && text.is_safe_to_copy());
        let private = ChunkType(*b"prVt");
        assert!(!private.is_public());
        assert!(!ChunkType(*b"IHdR").is_reserved_bit_valid());
    }
}
