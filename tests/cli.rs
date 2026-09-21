//! The `paeth` binary, run as a user would run it.

mod support;

use std::process::{Command, Output};

fn paeth(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_paeth"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("the binary was built for this test")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("output is UTF-8")
}

#[test]
fn reports_one_file_in_full() {
    let out = paeth(&["tests/pngsuite/basn6a16.png"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("32 x 32"), "{text}");
    assert!(text.contains("RGBA, 16-bit"), "{text}");
    assert!(text.contains("decoded"), "{text}");
    // Piped output carries no ANSI escapes.
    assert!(!text.contains('\x1b'), "{text}");
}

#[test]
fn walks_a_directory_and_summarises() {
    let out = paeth(&["tests/pngsuite"]);
    let text = stdout(&out);
    assert!(
        text.ends_with("176 files: 162 decoded, 14 rejected\n"),
        "{text}"
    );
    // Corrupt files are rejected, so the exit status is 1.
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn lists_chunks_with_offsets_and_crcs() {
    let out = paeth(&["--chunks", "tests/pngsuite/basn3p02.png"]);
    let text = stdout(&out);
    assert!(text.contains("offset  type      length  crc"), "{text}");
    // The IHDR chunk always starts at offset 8 and is 13 bytes long.
    assert!(text.contains("         8  IHDR          13  "), "{text}");
    assert!(text.contains("IEND           0  ae426082"), "{text}");
}

#[test]
fn writes_the_same_pixels_the_library_returns() {
    let dir = std::env::temp_dir().join("paeth-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.pam");
    let name = path.to_str().unwrap();

    let out = paeth(&["tests/pngsuite/basn6a08.png", "-o", name]);
    assert!(out.status.success(), "{}", stdout(&out));
    let written = std::fs::read(&path).unwrap();
    let header = b"P7\nWIDTH 32\nHEIGHT 32\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n";
    assert!(written.starts_with(header));
    let source = std::fs::read(support::suite_dir().join("basn6a08.png")).unwrap();
    assert_eq!(
        &written[header.len()..],
        paeth::decode(&source).unwrap().pixels
    );

    // And 16-bit samples come out big-endian.
    let out = paeth(&["tests/pngsuite/basn6a16.png", "-o", name, "--16"]);
    assert!(out.status.success(), "{}", stdout(&out));
    let written = std::fs::read(&path).unwrap();
    assert!(written.starts_with(b"P7\nWIDTH 32\nHEIGHT 32\nDEPTH 4\nMAXVAL 65535\n"));
    let source = std::fs::read(support::suite_dir().join("basn6a16.png")).unwrap();
    let expected: Vec<u8> = paeth::decode_rgba16(&source)
        .unwrap()
        .pixels
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
    assert!(written.ends_with(&expected));
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_corrupt_file_explains_itself_and_fails() {
    let out = paeth(&["tests/pngsuite/xhdn0g08.png"]);
    let text = stdout(&out);
    assert!(text.contains("rejected"), "{text}");
    assert!(
        text.contains("IHDR chunk at offset 8 fails its CRC"),
        "{text}"
    );
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn usage_errors_exit_with_2() {
    assert_eq!(paeth(&[]).status.code(), Some(2));
    assert_eq!(paeth(&["--nonsense"]).status.code(), Some(2));
    assert_eq!(
        paeth(&["--16", "tests/pngsuite/basn0g01.png"])
            .status
            .code(),
        Some(2)
    );
    let help = paeth(&["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("Usage: paeth"));
    let version = paeth(&["--version"]);
    assert_eq!(
        stdout(&version).trim(),
        concat!("paeth ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn preview_draws_the_image_with_half_blocks() {
    let out = paeth(&["--preview", "tests/pngsuite/basn2c08.png"]);
    let text = stdout(&out);
    // 24-bit color escapes, and one line per two rows of pixels.
    assert!(text.contains("\x1b[38;2;"), "{text}");
    assert_eq!(text.matches('\u{2580}').count() % 32, 0, "{text}");
}
