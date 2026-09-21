//! Conformance against PngSuite.
//!
//! Every file in tests/pngsuite must either decode to exactly the pixels
//! in reference.tsv, which come from other decoders (see
//! tools/gen_reference.py), or, for the files PngSuite marks as corrupt,
//! be rejected, and for the reason PngSuite says it is corrupt.

mod support;

use std::collections::BTreeMap;

use paeth::chunk::ChunkType;
use paeth::{ColorType, Error, LineEndingConversion, SignatureError};
use support::sha256::sha256_hex;

enum Expected {
    Pixels {
        width: u32,
        height: u32,
        rgba8: String,
        rgba16: String,
    },
    Reject,
}

fn reference() -> BTreeMap<String, Expected> {
    let text = std::fs::read_to_string(support::suite_dir().join("reference.tsv"))
        .expect("tests/pngsuite/reference.tsv exists; run tools/gen_reference.py");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let expected = match f[1] {
                "ok" => Expected::Pixels {
                    width: f[2].parse().unwrap(),
                    height: f[3].parse().unwrap(),
                    rgba8: f[4].to_string(),
                    rgba16: f[5].to_string(),
                },
                "reject" => Expected::Reject,
                other => panic!("bad verdict {other:?} in reference.tsv"),
            };
            (f[0].to_string(), expected)
        })
        .collect()
}

fn rgba16_bytes(pixels: &[u16]) -> Vec<u8> {
    pixels.iter().flat_map(|v| v.to_be_bytes()).collect()
}

#[test]
fn every_file_matches_the_reference() {
    let reference = reference();
    let files = support::suite_files();
    assert_eq!(
        files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        reference.keys().map(String::as_str).collect::<Vec<_>>(),
        "reference.tsv and the files on disk list the same images"
    );

    let mut failures = Vec::new();
    let (mut decoded, mut rejected) = (0, 0);
    for (name, bytes) in &files {
        match &reference[name] {
            Expected::Pixels {
                width,
                height,
                rgba8,
                rgba16,
            } => {
                let result = paeth::decode(bytes).and_then(|img8| {
                    let img16 = paeth::decode_rgba16(bytes)?;
                    Ok((img8, img16))
                });
                match result {
                    Err(e) => failures.push(format!("{name}: rejected a valid file: {e}")),
                    Ok((img8, img16)) => {
                        if (img8.width, img8.height) != (*width, *height) {
                            failures.push(format!(
                                "{name}: size {}x{}, expected {width}x{height}",
                                img8.width, img8.height
                            ));
                        } else if sha256_hex(&img8.pixels) != *rgba8 {
                            failures.push(format!("{name}: RGBA8 pixels differ"));
                        } else if sha256_hex(&rgba16_bytes(&img16.pixels)) != *rgba16 {
                            failures.push(format!("{name}: RGBA16 pixels differ"));
                        } else {
                            decoded += 1;
                        }
                    }
                }
            }
            Expected::Reject => match paeth::decode(bytes) {
                Ok(_) => failures.push(format!("{name}: accepted a corrupt file")),
                Err(_) => rejected += 1,
            },
        }
    }
    println!("PngSuite: {decoded} decoded pixel-exact, {rejected} corrupt files rejected");
    assert!(
        failures.is_empty(),
        "{} of {} files failed:\n  {}",
        failures.len(),
        files.len(),
        failures.join("\n  ")
    );
    // Pin the headline numbers, so the README cannot drift from the test.
    assert_eq!((decoded, rejected), (162, 14));
}

/// Each corrupt file is rejected for the reason its name gives
/// (PngSuite documentation, "corrupted files").
#[test]
fn corrupt_files_are_rejected_for_the_right_reason() {
    let files: BTreeMap<_, _> = support::suite_files().into_iter().collect();
    let err = |name: &str| paeth::decode(&files[name]).unwrap_err();
    let sig = |e| Error::Signature(e);
    let rgb_depth = |bit_depth| Error::InvalidBitDepth {
        color_type: ColorType::Rgb,
        bit_depth,
    };

    // Signature damage.
    assert_eq!(err("xs1n0g01.png"), sig(SignatureError::HighBitStripped));
    assert_eq!(err("xs2n0g01.png"), sig(SignatureError::Mismatch));
    assert_eq!(err("xs4n0g01.png"), sig(SignatureError::Mismatch));
    assert_eq!(err("xs7n0g01.png"), sig(SignatureError::Mismatch));
    assert_eq!(
        err("xcrn0g04.png"),
        sig(SignatureError::LineEndingsConverted(
            LineEndingConversion::LfToCr
        ))
    );
    assert_eq!(
        err("xlfn0g04.png"),
        sig(SignatureError::LineEndingsConverted(
            LineEndingConversion::CrToLf
        ))
    );
    // Header fields.
    assert_eq!(err("xc1n0g08.png"), Error::InvalidColorType(1));
    assert_eq!(err("xc9n2c08.png"), Error::InvalidColorType(9));
    assert_eq!(err("xd0n2c08.png"), rgb_depth(0));
    assert_eq!(err("xd3n2c08.png"), rgb_depth(3));
    assert_eq!(err("xd9n2c08.png"), rgb_depth(99));
    // Checksums.
    assert!(matches!(
        err("xhdn0g08.png"),
        Error::CrcMismatch {
            chunk: ChunkType::IHDR,
            ..
        }
    ));
    assert!(matches!(
        err("xcsn0g01.png"),
        Error::CrcMismatch {
            chunk: ChunkType::IDAT,
            ..
        }
    ));
    // Structure.
    assert_eq!(err("xdtn0g01.png"), Error::MissingChunk(ChunkType::IDAT));
}

/// No valid file should produce a warning: PngSuite's ancillary chunks
/// are all well-formed and correctly placed.
#[test]
fn valid_files_parse_without_warnings() {
    for (name, bytes) in support::suite_files() {
        if name.starts_with('x') {
            continue;
        }
        let png = paeth::Png::parse(&bytes).unwrap();
        assert_eq!(png.warnings(), [], "{name}");
        assert_eq!(png.trailing_bytes(), 0, "{name}");
    }
}

#[test]
fn sha256_known_answers() {
    // FIPS 180-4 example vectors, so a bug in the test's own hash cannot
    // make every comparison pass.
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        sha256_hex(&[b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}
