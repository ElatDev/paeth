//! Helpers shared by the integration tests.

#![allow(dead_code)]

pub mod sha256;

use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// The PngSuite directory.
pub fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("pngsuite")
}

/// Every PngSuite file, sorted by name.
pub fn suite_files() -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(suite_dir())
        .expect("tests/pngsuite exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&p).unwrap())
        })
        .collect();
    files.sort();
    files
}

pub fn crc32(bytes: &[u8]) -> u32 {
    // Bitwise, deliberately unlike the table-driven one in the crate.
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// Assembles PNG files chunk by chunk, for tests that need a file with
/// one specific thing wrong with it.
#[derive(Clone, Default)]
pub struct PngBuilder {
    bytes: Vec<u8>,
}

pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

impl PngBuilder {
    pub fn new() -> Self {
        PngBuilder {
            bytes: SIGNATURE.to_vec(),
        }
    }

    /// A chunk with a correct length and CRC.
    pub fn chunk(mut self, kind: &[u8; 4], data: &[u8]) -> Self {
        self.bytes
            .extend_from_slice(&(data.len() as u32).to_be_bytes());
        self.bytes.extend_from_slice(kind);
        self.bytes.extend_from_slice(data);
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        self.bytes
            .extend_from_slice(&crc32(&crc_input).to_be_bytes());
        self
    }

    pub fn ihdr(
        self,
        width: u32,
        height: u32,
        bit_depth: u8,
        color_type: u8,
        interlace: u8,
    ) -> Self {
        let mut d = Vec::new();
        d.extend_from_slice(&width.to_be_bytes());
        d.extend_from_slice(&height.to_be_bytes());
        d.extend_from_slice(&[bit_depth, color_type, 0, 0, interlace]);
        self.chunk(b"IHDR", &d)
    }

    /// Raw scanlines (filter bytes included), compressed into one IDAT.
    pub fn idat(self, scanlines: &[u8]) -> Self {
        let z = zlib(scanlines);
        self.chunk(b"IDAT", &z)
    }

    pub fn iend(self) -> Self {
        self.chunk(b"IEND", &[])
    }

    pub fn raw(mut self, bytes: &[u8]) -> Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    pub fn build(self) -> Vec<u8> {
        self.bytes
    }
}

/// Splits a PNG into (type, data) pairs, ignoring CRCs. Stops at the
/// first chunk whose length runs past the end of the file.
pub fn split_chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 8;
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let Some(data) = png.get(i + 8..i + 8 + len) else {
            break;
        };
        let kind: [u8; 4] = png[i + 4..i + 8].try_into().unwrap();
        out.push((kind, data.to_vec()));
        i += 12 + len;
    }
    out
}

/// Reassembles chunks with fresh CRCs.
pub fn join_chunks(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    chunks
        .iter()
        .fold(PngBuilder::new(), |b, (kind, data)| b.chunk(kind, data))
        .build()
}

/// xorshift64*: deterministic randomness for tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn byte(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }
}
