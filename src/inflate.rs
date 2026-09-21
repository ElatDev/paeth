//! Decompressing the image data.
//!
//! The IDAT chunks together hold one zlib stream, split at arbitrary
//! points: a chunk boundary can fall in the middle of a deflate block,
//! or even in the middle of the zlib header. So the chunks are joined
//! first and inflated as one.
//!
//! The header says exactly how many bytes the scanlines take, and the
//! stream has to deliver exactly that many: no fewer, no more, and with
//! nothing after its Adler-32 trailer.

use flate2::{Decompress, FlushDecompress, Status};

use crate::error::Error;

/// Output buffer to start with. It grows as data actually arrives, so a
/// header claiming a gigapixel image cannot make a 100-byte file
/// allocate gigabytes.
const INITIAL_CAPACITY: usize = 64 * 1024;

/// Joins the IDAT payloads.
pub(crate) fn concat(idat: &[&[u8]]) -> Vec<u8> {
    idat.concat()
}

/// Inflates `compressed` and checks it yields exactly `expected` bytes.
pub(crate) fn decompress(compressed: &[u8], expected: usize) -> Result<Vec<u8>, Error> {
    // One byte of headroom lets an over-long stream show itself.
    let limit = expected.saturating_add(1);
    let mut out = Vec::with_capacity(limit.min(INITIAL_CAPACITY.max(compressed.len())));
    let mut z = Decompress::new(true);
    loop {
        if out.len() == out.capacity() {
            let grow = out.capacity().max(4096).min(limit - out.len());
            out.reserve_exact(grow);
        }
        let consumed = z.total_in() as usize;
        let produced = out.len();
        let status = z
            .decompress_vec(&compressed[consumed..], &mut out, FlushDecompress::None)
            .map_err(|e| Error::CorruptImageData(e.to_string()))?;
        if out.len() > expected {
            return Err(Error::ImageDataTooLong { expected });
        }
        if status == Status::StreamEnd {
            break;
        }
        let stalled = z.total_in() as usize == consumed && out.len() == produced;
        if stalled && out.len() < out.capacity() {
            // Room to write and nothing written: the input ran out
            // before the stream's end marker.
            return Err(Error::ImageDataTooShort {
                expected,
                actual: out.len(),
            });
        }
    }
    if out.len() < expected {
        return Err(Error::ImageDataTooShort {
            expected,
            actual: out.len(),
        });
    }
    let trailing = compressed.len() - z.total_in() as usize;
    if trailing > 0 {
        return Err(Error::TrailingImageData { bytes: trailing });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn sample() -> Vec<u8> {
        (0..10_000u32).map(|i| (i * i % 251) as u8).collect()
    }

    #[test]
    fn exact_length_round_trips() {
        let data = sample();
        assert_eq!(decompress(&zlib(&data), data.len()).unwrap(), data);
    }

    #[test]
    fn a_split_anywhere_decodes_the_same() {
        // Chunk boundaries can fall anywhere, including inside the
        // two-byte zlib header and the four-byte Adler-32 trailer.
        let data = sample();
        let z = zlib(&data);
        for cut in 0..=z.len() {
            let joined = concat(&[&z[..cut], &z[cut..]]);
            assert_eq!(decompress(&joined, data.len()).unwrap(), data, "cut {cut}");
        }
    }

    #[test]
    fn too_short() {
        let data = sample();
        assert_eq!(
            decompress(&zlib(&data), data.len() + 1),
            Err(Error::ImageDataTooShort {
                expected: data.len() + 1,
                actual: data.len()
            })
        );
    }

    #[test]
    fn too_long() {
        let data = sample();
        assert_eq!(
            decompress(&zlib(&data), data.len() - 1),
            Err(Error::ImageDataTooLong {
                expected: data.len() - 1
            })
        );
    }

    #[test]
    fn truncated_stream() {
        let data = sample();
        let z = zlib(&data);
        for cut in [0, 1, 2, z.len() / 2, z.len() - 4, z.len() - 1] {
            let err = decompress(&z[..cut], data.len()).unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::ImageDataTooShort { .. } | Error::CorruptImageData(_)
                ),
                "cut {cut}: {err:?}"
            );
        }
    }

    #[test]
    fn trailing_bytes_after_the_stream() {
        let data = sample();
        let mut z = zlib(&data);
        z.extend_from_slice(&[0, 0, 0]);
        assert_eq!(
            decompress(&z, data.len()),
            Err(Error::TrailingImageData { bytes: 3 })
        );
    }

    #[test]
    fn bad_adler32_is_caught() {
        let data = sample();
        let mut z = zlib(&data);
        let last = z.len() - 1;
        z[last] ^= 1;
        assert!(matches!(
            decompress(&z, data.len()),
            Err(Error::CorruptImageData(_))
        ));
    }

    #[test]
    fn bad_zlib_header_is_caught() {
        let data = sample();
        let mut z = zlib(&data);
        z[0] = 0x79; // compression method 9 does not exist
        assert!(matches!(
            decompress(&z, data.len()),
            Err(Error::CorruptImageData(_))
        ));
    }

    #[test]
    fn huge_expected_size_does_not_preallocate() {
        // A tiny stream claiming to be 4 GB must fail fast on the data it
        // actually holds, without trying to reserve the claimed size.
        let z = zlib(b"tiny");
        assert_eq!(
            decompress(&z, u32::MAX as usize),
            Err(Error::ImageDataTooShort {
                expected: u32::MAX as usize,
                actual: 4
            })
        );
    }

    #[test]
    fn highly_compressible_data_grows_the_buffer() {
        // 50 MB of zeros compresses to about 50 KB, far past the initial
        // capacity guess.
        let data = vec![0u8; 50_000_000];
        let z = zlib(&data);
        assert!(z.len() < 100_000);
        assert_eq!(decompress(&z, data.len()).unwrap().len(), data.len());
    }
}
