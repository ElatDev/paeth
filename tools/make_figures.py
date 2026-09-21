# /// script
# requires-python = ">=3.10"
# dependencies = ["pypng==0.20220715.0", "pillow==12.3.0"]
# ///
"""Draw the README figures from paeth's own output.

    cargo build --release
    uv run tools/make_figures.py

docs/showcase.png  A test image written by pypng: 16-bit RGBA, Adam7-
                   interlaced, with a soft alpha edge. The hard cases in
                   one file.
docs/hero.png      That file as Pillow decodes it, next to paeth's decode.
docs/pngsuite.png  Every PngSuite file as paeth sees it: the decoded image,
                   or the reason it was rejected.

paeth's pixels come from its CLI (`paeth FILE -o out.pam`), so the figures
show exactly what the binary produces. Before drawing anything, the script
checks paeth's 16-bit output for the showcase against pypng's, and fails if
a single sample differs.
"""

from __future__ import annotations

import cmath
import colorsys
import math
import subprocess
import sys
import tempfile
from pathlib import Path

import png
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
SUITE = ROOT / "tests" / "pngsuite"
EXE = ROOT / "target" / "release" / ("paeth.exe" if sys.platform == "win32" else "paeth")

INK = (228, 230, 235)
MUTED = (140, 146, 158)
PANEL = (24, 26, 31)
RED = (240, 96, 96)


def font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    return ImageFont.load_default(size=size)


# --- The showcase image --------------------------------------------------


def showcase_pixels(width: int, height: int) -> list[list[int]]:
    """Domain coloring of f(z) = (z^3 - 1) / (z^2 + 0.6i): hue is the
    argument, bands mark powers of two in the modulus. Alpha fades out
    over a rounded edge, so the file needs every bit of its 16-bit alpha."""
    rows = []
    for y in range(height):
        row = []
        for x in range(width):
            z = complex((x - width / 2) / (height / 3.2), (height / 2 - y) / (height / 3.2))
            try:
                f = (z**3 - 1) / (z**2 + 0.6j)
            except ZeroDivisionError:
                f = complex(1e9, 0)
            hue = (cmath.phase(f) / (2 * math.pi)) % 1.0
            band = math.log2(abs(f) + 1e-12) % 1.0
            value = 0.62 + 0.38 * band
            r, g, b = colorsys.hsv_to_rgb(hue, 0.78, value)
            # Rounded-rectangle alpha with a 24 px feathered edge.
            dx = max(abs(x + 0.5 - width / 2) - (width / 2 - 40), 0)
            dy = max(abs(y + 0.5 - height / 2) - (height / 2 - 40), 0)
            edge = 40 - math.hypot(dx, dy)
            a = min(max(edge / 24, 0.0), 1.0)
            a = a * a * (3 - 2 * a)  # smoothstep
            row.extend(round(c * 65535) for c in (r, g, b, a))
        rows.append(row)
    return rows


def write_showcase(path: Path) -> None:
    width, height = 400, 300
    writer = png.Writer(
        width,
        height,
        greyscale=False,
        alpha=True,
        bitdepth=16,
        interlace=True,
        gamma=1 / 2.2,
        compression=9,
    )
    with open(path, "wb") as f:
        writer.write(f, showcase_pixels(width, height))


# --- Running paeth ---------------------------------------------------------


def read_pam(path: Path) -> tuple[int, int, int, bytes]:
    data = path.read_bytes()
    end = data.index(b"ENDHDR\n") + len(b"ENDHDR\n")
    fields = dict(
        line.split(" ", 1) for line in data[:end].decode().splitlines()[1:-1]
    )
    return int(fields["WIDTH"]), int(fields["HEIGHT"]), int(fields["MAXVAL"]), data[end:]


def paeth_decode(path: Path, sixteen: bool = False) -> tuple[Image.Image | None, bytes, str]:
    """Returns (RGBA image, raw samples, message) from paeth's CLI."""
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "out.pam"
        args = [str(EXE), str(path), "-o", str(out)] + (["--16"] if sixteen else [])
        run = subprocess.run(args, capture_output=True, encoding="utf-8", errors="replace")
        if run.returncode != 0:
            reason = next(
                (l.split("rejected", 1)[1].strip() for l in run.stdout.splitlines() if "rejected" in l),
                run.stdout.strip() or run.stderr.strip(),
            )
            return None, b"", reason
        width, height, maxval, samples = read_pam(out)
        if maxval == 255:
            return Image.frombytes("RGBA", (width, height), samples), samples, "ok"
        return None, samples, "ok"


# --- Drawing ----------------------------------------------------------------


def checkerboard(size: tuple[int, int], cell: int = 8) -> Image.Image:
    im = Image.new("RGB", size)
    draw = ImageDraw.Draw(im)
    for y in range(0, size[1], cell):
        for x in range(0, size[0], cell):
            shade = 58 if (x // cell + y // cell) % 2 == 0 else 44
            draw.rectangle([x, y, x + cell - 1, y + cell - 1], fill=(shade, shade, shade + 4))
    return im


def on_checkerboard(im: Image.Image, cell: int = 8) -> Image.Image:
    base = checkerboard(im.size, cell).convert("RGBA")
    base.alpha_composite(im)
    return base.convert("RGB")


def make_hero(showcase: Path, out: Path) -> None:
    ours, _, message = paeth_decode(showcase)
    assert ours is not None, message
    with Image.open(showcase) as im:
        # Pillow keeps the high byte of 16-bit samples; paeth rounds. The
        # difference is at most 1/255 and invisible, and the exact check
        # against pypng happens in main().
        theirs = im.convert("RGBA")

    pad, gap, label_h = 28, 28, 64
    w, h = ours.size
    canvas = Image.new("RGB", (pad * 2 + w * 2 + gap, pad + label_h + h + pad), PANEL)
    draw = ImageDraw.Draw(canvas)
    for i, (title, sub, im) in enumerate(
        [
            ("original", "showcase.png, decoded by Pillow", theirs),
            ("paeth", "same file, decoded by paeth", ours),
        ]
    ):
        x = pad + i * (w + gap)
        draw.text((x, pad), title, font=font(26), fill=INK)
        draw.text((x, pad + 34), sub, font=font(15), fill=MUTED)
        canvas.paste(on_checkerboard(im), (x, pad + label_h))
    canvas.save(out, optimize=True)


def make_contact_sheet(out: Path) -> tuple[int, int]:
    files = sorted(SUITE.glob("*.png"))
    columns, scale = 16, 2
    tile, label_h, gap = 32 * scale, 16, 10
    cell_w, cell_h = tile + gap, tile + label_h + gap
    rows = math.ceil(len(files) / columns)
    header = 56
    canvas = Image.new("RGB", (gap + columns * cell_w, header + rows * cell_h + gap), PANEL)
    draw = ImageDraw.Draw(canvas)
    decoded = rejected = 0
    for n, path in enumerate(files):
        x = gap + (n % columns) * cell_w
        y = header + (n // columns) * cell_h
        im, _, _ = paeth_decode(path)
        if im is not None:
            decoded += 1
            # Most PngSuite images are 32x32. The size tests run from 1x1
            # to 40x40, and the suite's logo is larger: shrink those to fit,
            # centre the small ones, then double everything.
            if im.width > 32 or im.height > 32:
                im = im.copy()
                im.thumbnail((32, 32), Image.Resampling.BOX)
            fitted = Image.new("RGBA", (32, 32))
            fitted.paste(im, ((32 - im.width) // 2, (32 - im.height) // 2))
            big = fitted.resize((tile, tile), Image.Resampling.NEAREST)
            canvas.paste(on_checkerboard(big, cell=4), (x, y))
        else:
            rejected += 1
            draw.rectangle([x, y, x + tile - 1, y + tile - 1], fill=(52, 28, 30), outline=RED)
            draw.line([x + 18, y + 18, x + tile - 19, y + tile - 19], fill=RED, width=3)
            draw.line([x + tile - 19, y + 18, x + 18, y + tile - 19], fill=RED, width=3)
        draw.text((x, y + tile + 2), path.stem, font=font(11), fill=MUTED if im else RED)
    title = f"PngSuite through paeth: {decoded} decoded, {rejected} rejected"
    draw.text((gap, 16), title, font=font(22), fill=INK)
    canvas.save(out, optimize=True)
    return decoded, rejected


def main() -> int:
    if not EXE.exists():
        print(f"{EXE} not found; run `cargo build --release` first", file=sys.stderr)
        return 1
    DOCS.mkdir(exist_ok=True)
    showcase = DOCS / "showcase.png"
    write_showcase(showcase)

    # The exact check: paeth's lossless output against pypng's.
    _, ours16, message = paeth_decode(showcase, sixteen=True)
    assert message == "ok", message
    reader = png.Reader(filename=str(showcase))
    _, _, rows, _ = reader.asRGBA()
    reference = b"".join(v.to_bytes(2, "big") for row in rows for v in row)
    if ours16 != reference:
        print("paeth and pypng disagree on showcase.png", file=sys.stderr)
        return 1
    print(f"showcase.png: paeth matches pypng on all {len(reference) // 2:,} samples")

    make_hero(showcase, DOCS / "hero.png")
    decoded, rejected = make_contact_sheet(DOCS / "pngsuite.png")
    print(f"pngsuite.png: {decoded} decoded, {rejected} rejected")
    print("wrote docs/showcase.png, docs/hero.png, docs/pngsuite.png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
