//! A strict PNG decoder, written from the specification.
//!
//! `paeth` reads a PNG file and hands back RGBA pixels. It supports every
//! color type, every bit depth and Adam7 interlacing, verifies every CRC,
//! enforces the chunk ordering rules, and rejects files the spec calls
//! broken instead of guessing at them. It decodes all 162 valid images in
//! [PngSuite] identically to an independent reference decoder and rejects
//! all 14 of its corrupt ones.
//!
//! It is read-only: no encoder, no resizing, no color management. `gAMA`,
//! `cHRM`, `iCCP` and friends are parsed and reported in [`Metadata`], not
//! applied. The only dependency is [`flate2`] for inflate.
//!
//! [PngSuite]: http://www.schaik.com/pngsuite/
//!
//! # Example
//!
//! ```
//! # fn main() -> Result<(), paeth::Error> {
//! # let bytes = include_bytes!("../tests/pngsuite/basn6a08.png");
//! let image = paeth::decode(bytes)?;
//! assert_eq!((image.width, image.height), (32, 32));
//! let [r, g, b, a] = image.pixel(0, 0);
//! # let _ = (r, g, b, a);
//! # Ok(())
//! # }
//! ```
//!
//! To look at the file before decoding it:
//!
//! ```
//! # fn main() -> Result<(), paeth::Error> {
//! # let bytes = include_bytes!("../tests/pngsuite/basn3p04.png");
//! let png = paeth::Png::parse(bytes)?;
//! let header = png.header();
//! println!("{}x{} {} at {} bits", header.width, header.height, header.color_type, header.bit_depth);
//! for text in &png.metadata().text {
//!     println!("{}: {}", text.keyword, text.text);
//! }
//! let pixels = png.decode_rgba16()?; // lossless for every bit depth
//! # let _ = pixels;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod adam7;
pub mod chunk;
mod crc;
mod error;
mod filter;
mod header;
mod image;
mod inflate;
pub mod metadata;
mod pixels;
mod png;
mod unpack;

pub use crate::error::{Error, LineEndingConversion, SignatureError};
pub use crate::header::{ColorType, Header, Interlace};
pub use crate::image::Image;
pub use crate::metadata::Metadata;
pub use crate::pixels::Channel;
pub use crate::png::{Limits, Png, Warning};

/// Decodes a PNG file to 8-bit RGBA.
///
/// Shorthand for [`Png::parse`] followed by [`Png::decode`].
///
/// # Errors
///
/// Any [`Error`]: the file is not a PNG, is damaged, breaks a rule of
/// the spec, or exceeds the default [`Limits`].
pub fn decode(png: &[u8]) -> Result<Image<u8>, Error> {
    Png::parse(png)?.decode()
}

/// Decodes a PNG file to 16-bit RGBA, which loses nothing at any bit depth.
///
/// Shorthand for [`Png::parse`] followed by [`Png::decode_rgba16`].
///
/// # Errors
///
/// The same as [`decode`].
pub fn decode_rgba16(png: &[u8]) -> Result<Image<u16>, Error> {
    Png::parse(png)?.decode_rgba16()
}
