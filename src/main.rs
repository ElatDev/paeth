//! `paeth`: inspect and decode PNG files from the command line.

use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use paeth::metadata::{Background, Metadata, RenderingIntent, Transparency};
use paeth::{ColorType, Header, Image, Interlace, Png};

const USAGE: &str = "\
paeth: inspect and decode PNG files

Usage: paeth [OPTIONS] <PATH>...

A PATH can be a file or a directory; directories are searched for *.png.
One file gets a full report. Several get one line each and a summary.

Options:
  -v, --verbose        full report for every file, even when there are several
  -c, --chunks         list every chunk with its offset, length and CRC
  -p, --preview        draw the decoded image in the terminal (24-bit color)
  -o, --output <FILE>  write the decoded pixels to FILE as a PAM image
      --16             with --output, write 16 bits per channel instead of 8
  -h, --help           print this help
  -V, --version        print the version

Exit status: 0 if every file decoded, 1 if any was rejected, 2 on bad usage.";

struct Options {
    paths: Vec<PathBuf>,
    verbose: bool,
    chunks: bool,
    preview: bool,
    output: Option<PathBuf>,
    sixteen: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options {
        paths: Vec::new(),
        verbose: false,
        chunks: false,
        preview: false,
        output: None,
        sixteen: false,
    };
    let mut args = std::env::args_os().skip(1);
    let mut only_paths = false;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if only_paths || !text.starts_with('-') || text == "-" {
            opts.paths.push(PathBuf::from(arg));
            continue;
        }
        match text.as_ref() {
            "--" => only_paths = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("paeth {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-v" | "--verbose" => opts.verbose = true,
            "-c" | "--chunks" => opts.chunks = true,
            "-p" | "--preview" => opts.preview = true,
            "--16" => opts.sixteen = true,
            "-o" | "--output" => {
                let file = args.next().ok_or("--output needs a file name")?;
                opts.output = Some(PathBuf::from(file));
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if opts.paths.is_empty() {
        return Err("no input files".into());
    }
    if opts.sixteen && opts.output.is_none() {
        return Err("--16 only applies with --output".into());
    }
    Ok(opts)
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("paeth: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let files = match expand(&opts.paths) {
        Ok(files) => files,
        Err(msg) => {
            eprintln!("paeth: {msg}");
            return ExitCode::from(2);
        }
    };
    if opts.output.is_some() && files.len() != 1 {
        eprintln!("paeth: --output needs exactly one input file");
        return ExitCode::from(2);
    }
    let style = Style::detect();
    let detailed = opts.verbose || files.len() == 1;
    let mut out = io::stdout().lock();
    let (mut ok, mut rejected) = (0, 0);
    for path in &files {
        let report = if detailed {
            report(path, &opts, style)
        } else {
            one_line(path, style)
        };
        let (text, success) = report;
        // A closed pipe (e.g. `paeth dir | head`) is not an error.
        if out.write_all(text.as_bytes()).is_err() {
            break;
        }
        if success {
            ok += 1;
        } else {
            rejected += 1;
        }
    }
    if files.len() > 1 {
        let _ = writeln!(
            out,
            "\n{} files: {} decoded, {} rejected",
            files.len(),
            style.good(&ok.to_string()),
            if rejected > 0 {
                style.bad(&rejected.to_string())
            } else {
                rejected.to_string()
            }
        );
    }
    if rejected == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Replaces directories with the PNG files inside them, sorted.
fn expand(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let entries =
                std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut found: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
                .collect();
            found.sort();
            if found.is_empty() {
                return Err(format!(
                    "{}: no .png files in this directory",
                    path.display()
                ));
            }
            files.extend(found);
        } else {
            files.push(path.clone());
        }
    }
    Ok(files)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn one_line(path: &Path, style: Style) -> (String, bool) {
    let name = file_name(path);
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return (format!("{}  {name}  {e}\n", style.bad("ERROR ")), false),
    };
    match Png::parse(&bytes).and_then(|png| png.decode().map(|_| png)) {
        Ok(png) => {
            let h = png.header();
            (
                format!(
                    "{}  {name:<14}  {}x{} {}\n",
                    style.good("OK    "),
                    h.width,
                    h.height,
                    describe_format(h)
                ),
                true,
            )
        }
        Err(e) => (format!("{}  {name:<14}  {e}\n", style.bad("REJECT")), false),
    }
}

fn report(path: &Path, opts: &Options, style: Style) -> (String, bool) {
    let mut s = String::new();
    let name = file_name(path);
    let fail = |mut s: String, label: &str, message: String| {
        painted_field(&mut s, label, &message, |l| style.bad(l));
        s.push('\n');
        (s, false)
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            let _ = writeln!(s, "{}", style.bold(&name));
            return fail(s, "error", e.to_string());
        }
    };
    let _ = writeln!(
        s,
        "{}  ({} bytes)",
        style.bold(&name),
        thousands(bytes.len() as u64)
    );
    let png = match Png::parse(&bytes) {
        Ok(png) => png,
        Err(e) => return fail(s, "rejected", e.to_string()),
    };
    let h = png.header();
    field(&mut s, "size", &format!("{} x {}", h.width, h.height));
    field(&mut s, "format", &describe_format(h));
    if let Some(palette) = png.palette() {
        let role = if h.color_type == ColorType::Indexed {
            ""
        } else {
            " (suggested)"
        };
        field(
            &mut s,
            "palette",
            &format!("{} entries{role}", palette.len()),
        );
    }
    describe_metadata(&mut s, png.metadata());
    let kinds: Vec<String> = png.chunks().iter().map(|c| c.kind.to_string()).collect();
    field(&mut s, "chunks", &collapse_runs(&kinds));
    if opts.chunks {
        let _ = writeln!(s, "\n      offset  type      length  crc");
        for c in png.chunks() {
            let _ = writeln!(
                s,
                "  {:>10}  {}  {:>10}  {:08x}",
                c.offset,
                c.kind,
                c.data.len(),
                c.crc
            );
        }
        s.push('\n');
    }
    for w in png.warnings() {
        painted_field(&mut s, "warning", &w.to_string(), |l| style.warn(l));
    }
    if png.trailing_bytes() > 0 {
        let note = format!("{} bytes after IEND were ignored", png.trailing_bytes());
        painted_field(&mut s, "note", &note, |l| style.warn(l));
    }

    let image = match png.decode() {
        Ok(img) => img,
        Err(e) => return fail(s, "rejected", e.to_string()),
    };
    field(
        &mut s,
        "decoded",
        &style.good(&format!(
            "OK, {} pixels",
            thousands(png.header().pixel_count())
        )),
    );
    if let Some(out_path) = &opts.output {
        let written = if opts.sixteen {
            png.decode_rgba16()
                .map_err(|e| e.to_string())
                .and_then(|img| {
                    write_pam(out_path, img.width, img.height, 65535, &to_be_bytes(&img))
                })
        } else {
            write_pam(out_path, image.width, image.height, 255, &image.pixels)
        };
        match written {
            Ok(()) => field(&mut s, "wrote", &out_path.display().to_string()),
            Err(e) => return fail(s, "error", format!("{}: {e}", out_path.display())),
        }
    }
    if opts.preview {
        s.push('\n');
        preview(&mut s, &image);
    }
    s.push('\n');
    (s, true)
}

fn field(s: &mut String, label: &str, value: &str) {
    let _ = writeln!(s, "  {label:<13}{value}");
}

/// A field with a colored label. The padding goes on before the color
/// codes, so the columns still line up.
fn painted_field(s: &mut String, label: &str, value: &str, paint: impl Fn(&str) -> String) {
    let _ = writeln!(s, "  {}{value}", paint(&format!("{label:<13}")));
}

fn describe_format(h: &Header) -> String {
    let interlace = match h.interlace {
        Interlace::None => "",
        Interlace::Adam7 => ", Adam7-interlaced",
    };
    let unit = if h.color_type == ColorType::Indexed {
        "bit index"
    } else {
        "bit"
    };
    format!("{}, {}-{unit}{interlace}", h.color_type, h.bit_depth)
}

fn describe_metadata(s: &mut String, m: &Metadata) {
    if let Some(t) = &m.transparency {
        let text = match t {
            Transparency::Gray(g) => format!("gray {g} is transparent"),
            Transparency::Rgb(r, g, b) => format!("RGB ({r}, {g}, {b}) is transparent"),
            Transparency::Palette(a) => format!("alpha for {} palette entries", a.len()),
        };
        field(s, "transparency", &text);
    }
    if let Some(g) = m.gamma {
        field(
            s,
            "gamma",
            &format!("{:.5} (stored {g}, not applied)", f64::from(g) / 100_000.0),
        );
    }
    if let Some(c) = &m.chromaticities {
        let p =
            |(x, y): (u32, u32)| format!("({:.4}, {:.4})", f64::from(x) / 1e5, f64::from(y) / 1e5);
        field(
            s,
            "chromaticity",
            &format!(
                "white {} red {} green {} blue {}",
                p(c.white),
                p(c.red),
                p(c.green),
                p(c.blue)
            ),
        );
    }
    if let Some(intent) = m.srgb {
        let name = match intent {
            RenderingIntent::Perceptual => "perceptual",
            RenderingIntent::RelativeColorimetric => "relative colorimetric",
            RenderingIntent::Saturation => "saturation",
            RenderingIntent::AbsoluteColorimetric => "absolute colorimetric",
        };
        field(s, "sRGB", &format!("{name} rendering intent"));
    }
    if let Some(icc) = &m.icc_profile {
        field(
            s,
            "ICC profile",
            &format!(
                "\"{}\", {} bytes (not applied)",
                icc.name,
                thousands(icc.profile.len() as u64)
            ),
        );
    }
    if let Some(c) = &m.cicp {
        field(
            s,
            "cICP",
            &format!(
                "primaries {}, transfer {}, matrix {}, {} range",
                c.color_primaries,
                c.transfer_function,
                c.matrix_coefficients,
                if c.full_range { "full" } else { "narrow" }
            ),
        );
    }
    if let Some(d) = &m.mastering_display {
        field(
            s,
            "mastering",
            &format!(
                "{:.4}-{:.1} cd/m2",
                f64::from(d.min_luminance) / 1e4,
                f64::from(d.max_luminance) / 1e4
            ),
        );
    }
    if let Some(l) = &m.content_light_level {
        field(
            s,
            "light level",
            &format!(
                "max {:.1} cd/m2, frame average {:.1}",
                f64::from(l.max_content) / 1e4,
                f64::from(l.max_frame_average) / 1e4
            ),
        );
    }
    if let Some(bits) = &m.significant_bits {
        let list: Vec<String> = bits.iter().map(u8::to_string).collect();
        field(s, "significant", &format!("{} bits", list.join(", ")));
    }
    if let Some(b) = &m.background {
        let text = match b {
            Background::Gray(g) => format!("gray {g}"),
            Background::Rgb(r, g, b) => format!("RGB ({r}, {g}, {b})"),
            Background::PaletteIndex(i) => format!("palette entry {i}"),
        };
        field(s, "background", &text);
    }
    if let Some(hist) = &m.histogram {
        field(s, "histogram", &format!("{} entries", hist.len()));
    }
    if let Some(p) = &m.physical_dimensions {
        let text = if p.per_metre {
            let dpi = f64::from(p.x) * 0.0254;
            format!("{} x {} pixels per metre ({dpi:.0} dpi)", p.x, p.y)
        } else {
            format!("aspect ratio {}:{}", p.x, p.y)
        };
        field(s, "pixel size", &text);
    }
    if let Some(t) = &m.modified {
        field(
            s,
            "modified",
            &format!(
                "{}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                t.year, t.month, t.day, t.hour, t.minute, t.second
            ),
        );
    }
    for t in &m.text {
        let mut text: String = t.text.replace('\n', " / ");
        if text.chars().count() > 60 {
            text = text.chars().take(57).collect::<String>() + "...";
        }
        let lang = if t.language.is_empty() {
            String::new()
        } else {
            format!(" [{}]", t.language)
        };
        field(
            s,
            &t.chunk.to_string(),
            &format!("{}{lang}: {text}", t.keyword),
        );
    }
    for p in &m.suggested_palettes {
        field(
            s,
            "sPLT",
            &format!(
                "\"{}\", {} entries at {} bits",
                p.name,
                p.entries.len(),
                p.sample_depth
            ),
        );
    }
    if let Some(exif) = &m.exif {
        field(
            s,
            "Exif",
            &format!("{} bytes", thousands(exif.len() as u64)),
        );
    }
    if let Some(a) = &m.animation {
        field(
            s,
            "animation",
            &format!("{} frames; only the default image is decoded", a.frames),
        );
    }
}

/// `IHDR IDAT IDAT IDAT IEND` becomes `IHDR IDAT x3 IEND`.
fn collapse_runs(kinds: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < kinds.len() {
        let run = kinds[i..].iter().take_while(|k| **k == kinds[i]).count();
        parts.push(if run > 1 {
            format!("{} x{run}", kinds[i])
        } else {
            kinds[i].clone()
        });
        i += run;
    }
    parts.join(" ")
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn to_be_bytes(img: &Image<u16>) -> Vec<u8> {
    img.pixels.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// Writes a PAM (Netpbm "P7") file: a text header followed by the raw
/// RGBA samples, big-endian when 16-bit. It is a pixel dump, not an
/// encoder; there is no compression and nothing to get wrong.
fn write_pam(
    path: &Path,
    width: u32,
    height: u32,
    maxval: u32,
    samples: &[u8],
) -> Result<(), String> {
    let mut data = format!(
        "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH 4\nMAXVAL {maxval}\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
    )
    .into_bytes();
    data.extend_from_slice(samples);
    std::fs::write(path, data).map_err(|e| e.to_string())
}

/// Draws the image with "▀" characters: foreground is the upper pixel,
/// background the lower, so each character cell shows two pixels.
/// Transparency is shown against a checkerboard. Small images are
/// scaled up and large ones down, by whole-pixel sampling, to fit.
fn preview(s: &mut String, img: &Image<u8>) {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(80)
        .saturating_sub(4)
        .max(8);
    let (w, h) = (img.width as usize, img.height as usize);
    let (out_w, out_h) = if w < columns / 2 {
        let scale = (columns / 2 / w).clamp(1, 4);
        (w * scale, h * scale)
    } else if w > columns {
        (columns, (h * columns / w).max(1))
    } else {
        (w, h)
    };
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let [r, g, b, a] = img.pixel((x * w / out_w) as u32, (y * h / out_h) as u32);
        let checker = if (x / 4 + y / 4) % 2 == 0 { 204 } else { 153 };
        let blend = |c: u8| {
            ((u32::from(c) * u32::from(a) + checker * (255 - u32::from(a)) + 127) / 255) as u8
        };
        [blend(r), blend(g), blend(b)]
    };
    for y in (0..out_h).step_by(2) {
        s.push_str("  ");
        for x in 0..out_w {
            let [r, g, b] = sample(x, y);
            let _ = write!(s, "\x1b[38;2;{r};{g};{b}m");
            if y + 1 < out_h {
                let [r, g, b] = sample(x, y + 1);
                let _ = write!(s, "\x1b[48;2;{r};{g};{b}m");
            }
            s.push('▀');
        }
        s.push_str("\x1b[0m\n");
    }
}

/// ANSI styling, on only when stdout is a terminal and `NO_COLOR` is unset.
#[derive(Clone, Copy)]
struct Style {
    color: bool,
}

impl Style {
    fn detect() -> Self {
        Style {
            color: io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn good(self, text: &str) -> String {
        self.paint("32", text)
    }

    fn bad(self, text: &str) -> String {
        self.paint("31", text)
    }

    fn warn(self, text: &str) -> String {
        self.paint("33", text)
    }

    fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn runs_of_chunks_collapse() {
        let kinds: Vec<String> = ["IHDR", "IDAT", "IDAT", "IDAT", "IEND"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(collapse_runs(&kinds), "IHDR IDAT x3 IEND");
    }
}
