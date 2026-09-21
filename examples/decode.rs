//! Decode a PNG and summarise its pixels.
//!
//! ```text
//! cargo run --example decode -- tests/pngsuite/tbrn2c08.png
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: decode <file.png>");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Parsing checks the structure; decoding inflates and unfilters.
    let png = match paeth::Png::parse(&bytes) {
        Ok(png) => png,
        Err(e) => {
            eprintln!("{path}: rejected: {e}");
            return ExitCode::FAILURE;
        }
    };
    let image = match png.decode() {
        Ok(image) => image,
        Err(e) => {
            eprintln!("{path}: rejected: {e}");
            return ExitCode::FAILURE;
        }
    };

    let header = png.header();
    println!(
        "{path}: {}x{}, {} at {} bits",
        image.width, image.height, header.color_type, header.bit_depth
    );

    let mut sum = [0u64; 4];
    let mut transparent = 0;
    for px in image.pixels.chunks_exact(4) {
        for (total, &channel) in sum.iter_mut().zip(px) {
            *total += u64::from(channel);
        }
        transparent += usize::from(px[3] == 0);
    }
    let n = (image.pixels.len() / 4) as u64;
    println!("mean RGBA:          {:?}", sum.map(|s| s / n));
    println!("transparent pixels: {transparent} of {n}");
    if let Some(gamma) = png.metadata().gamma {
        println!("file gamma:         {:.5}", f64::from(gamma) / 100_000.0);
    }
    ExitCode::SUCCESS
}
