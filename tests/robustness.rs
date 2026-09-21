//! Hostile input: mutate every PngSuite file thousands of ways and check
//! the decoder never panics, and that whatever it does accept is a
//! well-formed image.
//!
//! Random byte flips alone mostly test the CRC check, since almost every
//! flip breaks a checksum. So most strategies here repair the CRCs after
//! mutating, and one recompresses the image data after corrupting the
//! scanlines inside it. That pushes the damage past the checksums and
//! into the parser, the inflater, the unfilter and the pixel converter.
//!
//! Set `PAETH_MUTATIONS` to change the number of mutations per file and
//! strategy (default 40).

mod support;

use std::cell::Cell;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};

use flate2::read::ZlibDecoder;
use support::{Rng, join_chunks, split_chunks, zlib};

#[derive(Clone, Copy, Debug)]
enum Strategy {
    /// Flip random bits anywhere, CRCs left stale.
    FlipBits,
    /// Cut the file short at a random point.
    Truncate,
    /// Change random bytes inside one chunk, then fix its CRC.
    ChunkData,
    /// Replace IHDR fields with other legal values, then fix the CRC, so
    /// the image data no longer fits the header.
    Header,
    /// Corrupt the decompressed scanlines (filter bytes, palette
    /// indices, anything), recompress, fix the CRCs.
    Scanlines,
    /// Duplicate, drop or swap whole chunks.
    Reorder,
}

const STRATEGIES: [Strategy; 6] = [
    Strategy::FlipBits,
    Strategy::Truncate,
    Strategy::ChunkData,
    Strategy::Header,
    Strategy::Scanlines,
    Strategy::Reorder,
];

fn mutate(original: &[u8], strategy: Strategy, rng: &mut Rng) -> Vec<u8> {
    // The chunk-aware strategies need a file whose framing survives
    // splitting. PngSuite's corrupt files do not all qualify.
    let chunks = split_chunks(original);
    let framed = chunks
        .first()
        .is_some_and(|(k, d)| k == b"IHDR" && d.len() == 13)
        && chunks.iter().any(|(k, _)| k == b"IDAT");
    let strategy = match strategy {
        Strategy::FlipBits | Strategy::Truncate => strategy,
        _ if !framed => Strategy::FlipBits,
        _ => strategy,
    };
    match strategy {
        Strategy::FlipBits => {
            let mut png = original.to_vec();
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(png.len());
                png[i] ^= 1 << rng.below(8);
            }
            png
        }
        Strategy::Truncate => original[..rng.below(original.len())].to_vec(),
        Strategy::ChunkData => {
            let mut chunks = split_chunks(original);
            let with_data: Vec<usize> = (0..chunks.len())
                .filter(|&i| !chunks[i].1.is_empty())
                .collect();
            let (_, data) = &mut chunks[with_data[rng.below(with_data.len())]];
            for _ in 0..1 + rng.below(3) {
                let i = rng.below(data.len());
                data[i] = rng.byte();
            }
            join_chunks(&chunks)
        }
        Strategy::Header => {
            let mut chunks = split_chunks(original);
            let ihdr = &mut chunks[0].1;
            const LEGAL: [(u8, u8); 15] = [
                (0, 1),
                (0, 2),
                (0, 4),
                (0, 8),
                (0, 16),
                (2, 8),
                (2, 16),
                (3, 1),
                (3, 2),
                (3, 4),
                (3, 8),
                (4, 8),
                (4, 16),
                (6, 8),
                (6, 16),
            ];
            match rng.below(4) {
                0 => {
                    let (color, depth) = LEGAL[rng.below(LEGAL.len())];
                    ihdr[8] = depth;
                    ihdr[9] = color;
                }
                1 => ihdr[12] ^= 1, // toggle interlacing
                2 => {
                    let w = 1 + rng.below(64) as u32;
                    ihdr[0..4].copy_from_slice(&w.to_be_bytes());
                }
                _ => {
                    // Occasionally absurd sizes, to exercise the limits.
                    let h = (rng.next_u64() as u32) & 0x7FFF_FFFF | 1;
                    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
                }
            }
            join_chunks(&chunks)
        }
        Strategy::Scanlines => {
            let mut chunks = split_chunks(original);
            let compressed: Vec<u8> = chunks
                .iter()
                .filter(|(k, _)| k == b"IDAT")
                .flat_map(|(_, d)| d.clone())
                .collect();
            let mut raw = Vec::new();
            if ZlibDecoder::new(&compressed[..])
                .read_to_end(&mut raw)
                .is_err()
                || raw.is_empty()
            {
                return original.to_vec();
            }
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(raw.len());
                raw[i] = rng.byte();
            }
            if rng.below(8) == 0 {
                raw.truncate(rng.below(raw.len()));
            } else if rng.below(8) == 0 {
                raw.push(rng.byte());
            }
            let first = chunks.iter().position(|(k, _)| k == b"IDAT").unwrap();
            chunks.retain(|(k, _)| k != b"IDAT");
            chunks.insert(first, (*b"IDAT", zlib(&raw)));
            join_chunks(&chunks)
        }
        Strategy::Reorder => {
            let mut chunks = split_chunks(original);
            let n = chunks.len();
            match rng.below(3) {
                0 => {
                    let c = chunks[rng.below(n)].clone();
                    chunks.insert(rng.below(n + 1), c);
                }
                1 => {
                    chunks.remove(rng.below(n));
                }
                _ => chunks.swap(rng.below(n), rng.below(n)),
            }
            join_chunks(&chunks)
        }
    }
}

#[test]
fn mutated_files_never_panic() {
    let per_file: usize = std::env::var("PAETH_MUTATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let files = support::suite_files();
    let mut rng = Rng(0x5EED_F00D_BADF_11E5);
    let (mut total, mut accepted, mut rejected) = (0usize, 0usize, 0usize);
    let mut panics = Vec::new();

    // Keep panics inside the decoder quiet, so a failure lists every
    // culprit at once; anything else still reports normally.
    thread_local!(static DECODING: Cell<bool> = const { Cell::new(false) });
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !DECODING.get() {
            default_hook(info);
        }
    }));
    for (name, original) in &files {
        for strategy in STRATEGIES {
            for i in 0..per_file {
                let png = mutate(original, strategy, &mut rng);
                total += 1;
                DECODING.set(true);
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let png = paeth::Png::parse(&png)?;
                    let img8 = png.decode()?;
                    let img16 = png.decode_rgba16()?;
                    Ok::<_, paeth::Error>((img8, img16))
                }));
                DECODING.set(false);
                match outcome {
                    Err(_) => panics.push(format!("{name} {strategy:?} #{i}")),
                    Ok(Err(_)) => rejected += 1,
                    Ok(Ok((img8, img16))) => {
                        let n = img8.width as usize * img8.height as usize * 4;
                        assert_eq!(img8.pixels.len(), n, "{name} {strategy:?} #{i}");
                        assert_eq!(img16.pixels.len(), n, "{name} {strategy:?} #{i}");
                        // Both depths come from the same samples.
                        for (a, b) in img8.pixels.iter().zip(&img16.pixels) {
                            assert_eq!(u32::from(*a), (u32::from(*b) * 255 + 32767) / 65535);
                        }
                        accepted += 1;
                    }
                }
            }
        }
    }
    let _ = std::panic::take_hook();

    println!(
        "{total} mutated files: {accepted} decoded, {rejected} rejected, {} panics",
        panics.len()
    );
    assert!(
        panics.is_empty(),
        "decoder panicked on:\n  {}",
        panics.join("\n  ")
    );
    // The strategies are only worth running if they reach past the CRC
    // check often enough to produce both outcomes.
    assert!(
        accepted > total / 20,
        "only {accepted} of {total} mutations decoded"
    );
    assert!(
        rejected > total / 4,
        "only {rejected} of {total} mutations rejected"
    );
}
