//! The structural layer: which chunks may appear, in what order, and
//! what they mean together (PNG spec, section 5.6).

use std::fmt;

use crate::adam7;
use crate::chunk::{self, Chunk, ChunkType};
use crate::error::Error;
use crate::filter;
use crate::header::{ColorType, Header};
use crate::image::Image;
use crate::inflate;
use crate::metadata::{self, Context, Metadata};
use crate::pixels::{Channel, RowConverter};

/// Caps that keep a hostile file from exhausting memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest width × height accepted. The default, 2²⁸, is a
    /// 16384 × 16384 image: 1 GiB decoded as RGBA8.
    pub max_pixels: u64,
    /// Largest size a compressed text or ICC chunk may expand to.
    pub max_metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_pixels: 1 << 28,
            max_metadata_bytes: 16 << 20,
        }
    }
}

/// A problem in an ancillary chunk. The chunk was ignored and the image
/// is still decodable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// The chunk that was ignored.
    pub chunk: ChunkType,
    /// Its byte offset in the file.
    pub offset: usize,
    /// What was wrong with it.
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at offset {} ignored: {}",
            self.chunk, self.offset, self.message
        )
    }
}

/// A parsed PNG file, ready to decode.
///
/// Parsing reads every chunk, verifies every CRC and checks the chunk
/// ordering rules, but does not inflate the image data. That happens in
/// [`decode`](Png::decode) or [`decode_rgba16`](Png::decode_rgba16).
#[derive(Clone, Debug)]
pub struct Png<'a> {
    header: Header,
    palette: Option<Vec<[u8; 3]>>,
    metadata: Metadata,
    chunks: Vec<Chunk<'a>>,
    /// The chunks from [`SINGLE`] seen so far. Bounded by its length, so
    /// a file with thousands of ancillary chunks still parses in linear
    /// time.
    seen: Vec<ChunkType>,
    warnings: Vec<Warning>,
    trailing_bytes: usize,
    limits: Limits,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageData {
    NotSeen,
    InProgress,
    Finished,
}

/// Chunks that must come before PLTE and IDAT.
const BEFORE_PLTE: [ChunkType; 8] = [
    ChunkType::cHRM,
    ChunkType::gAMA,
    ChunkType::iCCP,
    ChunkType::sBIT,
    ChunkType::sRGB,
    ChunkType::cICP,
    ChunkType::mDCV,
    ChunkType::cLLI,
];

/// Chunks that must come before IDAT.
const BEFORE_IDAT: [ChunkType; 7] = [
    ChunkType::bKGD,
    ChunkType::hIST,
    ChunkType::tRNS,
    ChunkType::pHYs,
    ChunkType::sPLT,
    ChunkType::eXIf,
    ChunkType::acTL,
];

/// The ancillary chunks this decoder reads that may appear only once.
/// Everything else it reads (`tEXt`, `zTXt`, `iTXt`, `sPLT`) may repeat.
const SINGLE: [ChunkType; 15] = [
    ChunkType::tRNS,
    ChunkType::gAMA,
    ChunkType::cHRM,
    ChunkType::sRGB,
    ChunkType::iCCP,
    ChunkType::cICP,
    ChunkType::mDCV,
    ChunkType::cLLI,
    ChunkType::sBIT,
    ChunkType::bKGD,
    ChunkType::hIST,
    ChunkType::pHYs,
    ChunkType::tIME,
    ChunkType::eXIf,
    ChunkType::acTL,
];

/// How many warnings are kept. A file can carry an unbounded number of
/// broken ancillary chunks, and the decoder should not grow a string for
/// each one.
const MAX_WARNINGS: usize = 64;

impl<'a> Png<'a> {
    /// Parses a PNG file with the default [`Limits`].
    ///
    /// # Errors
    ///
    /// See [`parse_with_limits`](Png::parse_with_limits).
    pub fn parse(data: &'a [u8]) -> Result<Png<'a>, Error> {
        Png::parse_with_limits(data, Limits::default())
    }

    /// Parses a PNG file: every chunk, every CRC, the ordering rules
    /// and the ancillary chunks, but not the image data.
    ///
    /// # Errors
    ///
    /// A bad signature, a truncated or corrupt chunk, an invalid IHDR,
    /// a critical chunk that is missing, repeated, misplaced or unknown,
    /// or an image larger than `limits` allows.
    pub fn parse_with_limits(data: &'a [u8], limits: Limits) -> Result<Png<'a>, Error> {
        let mut iter = chunk::chunks(data)?;
        let first = iter.next().ok_or(Error::MissingChunk(ChunkType::IHDR))??;
        if first.kind != ChunkType::IHDR {
            return Err(Error::FirstChunkNotIhdr(first.kind));
        }
        let header = Header::parse(first.data)?;
        if header.pixel_count() > limits.max_pixels {
            return Err(Error::LimitExceeded {
                pixels: header.pixel_count(),
                limit: limits.max_pixels,
            });
        }
        let mut png = Png {
            header,
            palette: None,
            metadata: Metadata::default(),
            chunks: vec![first],
            seen: Vec::new(),
            warnings: Vec::new(),
            trailing_bytes: 0,
            limits,
        };
        let mut image_data = ImageData::NotSeen;
        let mut saw_iend = false;
        for chunk in iter.by_ref() {
            let chunk = chunk?;
            if chunk.kind != ChunkType::IDAT && image_data == ImageData::InProgress {
                image_data = ImageData::Finished;
            }
            match chunk.kind {
                ChunkType::IHDR => return Err(Error::DuplicateChunk(ChunkType::IHDR)),
                ChunkType::PLTE => png.read_palette(&chunk, image_data)?,
                ChunkType::IDAT => {
                    if image_data == ImageData::Finished {
                        return Err(Error::MisplacedChunk {
                            chunk: ChunkType::IDAT,
                            rule: "IDAT chunks must be consecutive",
                        });
                    }
                    image_data = ImageData::InProgress;
                }
                ChunkType::IEND => {
                    if !chunk.data.is_empty() {
                        return Err(Error::BadChunkLength {
                            chunk: ChunkType::IEND,
                            length: chunk.data.len(),
                        });
                    }
                    saw_iend = true;
                }
                kind if kind.is_critical() => return Err(Error::UnknownCriticalChunk(kind)),
                _ => png.ancillary(&chunk, image_data != ImageData::NotSeen),
            }
            png.chunks.push(chunk);
        }
        if !saw_iend {
            return Err(Error::MissingChunk(ChunkType::IEND));
        }
        if image_data == ImageData::NotSeen {
            return Err(Error::MissingChunk(ChunkType::IDAT));
        }
        if header.color_type == ColorType::Indexed && png.palette.is_none() {
            return Err(Error::MissingChunk(ChunkType::PLTE));
        }
        png.trailing_bytes = iter.remainder().len();
        Ok(png)
    }

    fn read_palette(&mut self, chunk: &Chunk<'_>, image_data: ImageData) -> Result<(), Error> {
        if self.palette.is_some() {
            return Err(Error::DuplicateChunk(ChunkType::PLTE));
        }
        if image_data != ImageData::NotSeen {
            return Err(Error::MisplacedChunk {
                chunk: ChunkType::PLTE,
                rule: "PLTE must come before IDAT",
            });
        }
        let color_type = self.header.color_type;
        if matches!(color_type, ColorType::Grayscale | ColorType::GrayscaleAlpha) {
            return Err(Error::UnexpectedPalette(color_type));
        }
        let len = chunk.data.len();
        if len == 0 || len % 3 != 0 || len > 256 * 3 {
            return Err(Error::BadChunkLength {
                chunk: ChunkType::PLTE,
                length: len,
            });
        }
        let entries = len / 3;
        if color_type == ColorType::Indexed && entries > 1 << self.header.bit_depth {
            return Err(Error::PaletteTooLarge {
                entries,
                bit_depth: self.header.bit_depth,
            });
        }
        self.palette = Some(
            chunk
                .data
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect(),
        );
        Ok(())
    }

    fn ancillary(&mut self, chunk: &Chunk<'_>, after_idat: bool) {
        let kind = chunk.kind;
        let placement = if BEFORE_PLTE.contains(&kind) && (self.palette.is_some() || after_idat) {
            Some("must come before PLTE and IDAT")
        } else if BEFORE_IDAT.contains(&kind) && after_idat {
            Some("must come before IDAT")
        } else {
            None
        };
        if let Some(rule) = placement {
            return self.warn(chunk, rule.to_string());
        }
        if SINGLE.contains(&kind) {
            if self.seen.contains(&kind) {
                return self.warn(chunk, "duplicate; only the first one counts".to_string());
            }
            self.seen.push(kind);
        }
        let cx = Context {
            header: &self.header,
            palette: self.palette.as_deref(),
            max_decompressed: self.limits.max_metadata_bytes,
        };
        let data = chunk.data;
        let m = &mut self.metadata;
        let result = match kind {
            ChunkType::tRNS => metadata::parse_trns(data, &cx).map(|v| m.transparency = Some(v)),
            ChunkType::gAMA => metadata::parse_gama(data).map(|v| m.gamma = Some(v)),
            ChunkType::cHRM => metadata::parse_chrm(data).map(|v| m.chromaticities = Some(v)),
            ChunkType::sRGB if m.icc_profile.is_some() => {
                Err("sRGB and iCCP must not both be present".into())
            }
            ChunkType::sRGB => metadata::parse_srgb(data).map(|v| m.srgb = Some(v)),
            ChunkType::iCCP if m.srgb.is_some() => {
                Err("sRGB and iCCP must not both be present".into())
            }
            ChunkType::iCCP => metadata::parse_iccp(data, &cx).map(|v| m.icc_profile = Some(v)),
            ChunkType::cICP => metadata::parse_cicp(data).map(|v| m.cicp = Some(v)),
            ChunkType::mDCV => metadata::parse_mdcv(data).map(|v| m.mastering_display = Some(v)),
            ChunkType::cLLI => metadata::parse_clli(data).map(|v| m.content_light_level = Some(v)),
            ChunkType::sBIT => {
                metadata::parse_sbit(data, cx.header).map(|v| m.significant_bits = Some(v))
            }
            ChunkType::bKGD => metadata::parse_bkgd(data, &cx).map(|v| m.background = Some(v)),
            ChunkType::hIST => metadata::parse_hist(data, &cx).map(|v| m.histogram = Some(v)),
            ChunkType::pHYs => metadata::parse_phys(data).map(|v| m.physical_dimensions = Some(v)),
            ChunkType::tIME => metadata::parse_time(data).map(|v| m.modified = Some(v)),
            ChunkType::tEXt => metadata::parse_text(data).map(|v| m.text.push(v)),
            ChunkType::zTXt => metadata::parse_ztxt(data, &cx).map(|v| m.text.push(v)),
            ChunkType::iTXt => metadata::parse_itxt(data, &cx).map(|v| m.text.push(v)),
            ChunkType::sPLT => metadata::parse_splt(data).map(|v| m.suggested_palettes.push(v)),
            ChunkType::eXIf => {
                m.exif = Some(data.to_vec());
                Ok(())
            }
            ChunkType::acTL => metadata::parse_actl(data).map(|v| m.animation = Some(v)),
            // Unknown ancillary chunks are skipped, as the spec says.
            _ => Ok(()),
        };
        if let Err(message) = result {
            self.warn(chunk, message);
        }
    }

    fn warn(&mut self, chunk: &Chunk<'_>, message: String) {
        let message = match self.warnings.len() {
            n if n >= MAX_WARNINGS => return,
            n if n == MAX_WARNINGS - 1 => "further warnings are not reported".to_string(),
            _ => message,
        };
        self.warnings.push(Warning {
            chunk: chunk.kind,
            offset: chunk.offset,
            message,
        });
    }

    /// The image header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The PLTE entries: required for indexed images, an optional
    /// suggested palette for RGB and RGBA ones.
    pub fn palette(&self) -> Option<&[[u8; 3]]> {
        self.palette.as_deref()
    }

    /// What the ancillary chunks said.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Every chunk read, in file order, IHDR through IEND.
    pub fn chunks(&self) -> &[Chunk<'a>] {
        &self.chunks
    }

    /// Problems in ancillary chunks that were skipped. At most 64 are
    /// kept; the last one then says so.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Bytes after the IEND chunk. They are not part of the PNG and
    /// were not read.
    pub fn trailing_bytes(&self) -> usize {
        self.trailing_bytes
    }

    /// Whether the file is an animated PNG. Only its default image is
    /// decoded.
    pub fn is_animated(&self) -> bool {
        self.metadata.animation.is_some()
    }

    /// Decodes to 8 bits per channel. 16-bit images are rounded to the
    /// nearest 8-bit value.
    ///
    /// # Errors
    ///
    /// A corrupt zlib stream, image data of the wrong length, an invalid
    /// filter type, or a pixel indexing past the end of the palette.
    pub fn decode(&self) -> Result<Image<u8>, Error> {
        self.decode_as()
    }

    /// Decodes to 16 bits per channel. Lossless for every PNG.
    ///
    /// # Errors
    ///
    /// The same as [`decode`](Png::decode).
    pub fn decode_rgba16(&self) -> Result<Image<u16>, Error> {
        self.decode_as()
    }

    fn decode_as<C: Channel>(&self) -> Result<Image<C>, Error> {
        let header = &self.header;
        let too_big = || Error::LimitExceeded {
            pixels: header.pixel_count(),
            limit: self.limits.max_pixels,
        };
        let passes: Vec<_> = adam7::passes(header)
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        // Bytes the scanlines occupy, filter bytes included. Parsing
        // capped the pixel count, so this fits a u64 easily; on a 32-bit
        // target it may still not fit a usize.
        let mut expected = 0u64;
        for pass in &passes {
            expected += u64::from(pass.height) * (1 + header.row_bytes(pass.width));
        }
        let expected = usize::try_from(expected).map_err(|_| too_big())?;

        let idat: Vec<&[u8]> = self
            .chunks
            .iter()
            .filter(|c| c.kind == ChunkType::IDAT)
            .map(|c| c.data)
            .collect();
        let raw = inflate::decompress(&inflate::concat(&idat), expected)?;

        let width = header.width as usize;
        let len = width
            .checked_mul(header.height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(too_big)?;
        let mut pixels = vec![C::default(); len];
        let mut converter = RowConverter::new(
            header,
            self.palette.as_deref(),
            self.metadata.transparency.as_ref(),
        );
        let stride = header.filter_stride();
        let mut offset = 0;
        let mut row_index = 0;
        for pass in &passes {
            let row_bytes = header.row_bytes(pass.width) as usize;
            let pass_len = pass.height as usize * (row_bytes + 1);
            let rows = filter::unfilter_pass(
                &raw[offset..offset + pass_len],
                row_bytes,
                stride,
                row_index,
            )?;
            offset += pass_len;
            row_index += pass.height as usize;
            for (j, row) in rows.chunks_exact(row_bytes).enumerate() {
                let y = pass.y0 as usize + j * pass.dy as usize;
                let line = &mut pixels[y * width * 4..(y + 1) * width * 4];
                converter.convert(row, pass.width as usize, |i, rgba: [C; 4]| {
                    let x = pass.x0 as usize + i * pass.dx as usize;
                    line[x * 4..x * 4 + 4].copy_from_slice(&rgba);
                })?;
            }
        }
        Ok(Image {
            width: header.width,
            height: header.height,
            pixels,
        })
    }
}
