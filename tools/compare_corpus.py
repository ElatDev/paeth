# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow==12.3.0"]
# ///
"""Decode every PNG under a directory with paeth and with Pillow, and
compare the pixels.

    cargo build --release
    uv run tools/compare_corpus.py ~/Pictures

PngSuite is a set of images built to break decoders; this is the opposite
test, on whatever real files you happen to have. Nothing is uploaded and
nothing is written except a temporary PAM file per image.

Pillow keeps only the high byte of 16-bit samples while paeth rounds, so
for those files a difference of 1 in a channel is expected and reported
separately from a real mismatch. Pillow also identifies files by content,
so it will happily decode a JPEG or WebP named `.png`; paeth rejects
those, and they are listed as rejections rather than failures.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "target" / "release" / ("paeth.exe" if sys.platform == "win32" else "paeth")


def read_pam(path: Path) -> tuple[int, int, int, bytes]:
    data = path.read_bytes()
    end = data.index(b"ENDHDR\n") + len(b"ENDHDR\n")
    fields = dict(line.split(" ", 1) for line in data[:end].decode().splitlines()[1:-1])
    return int(fields["WIDTH"]), int(fields["HEIGHT"]), int(fields["MAXVAL"]), data[end:]


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    if not EXE.exists():
        print(f"{EXE} not found; run `cargo build --release` first", file=sys.stderr)
        return 1
    roots = [Path(a) for a in sys.argv[1:]]
    files = sorted(p for root in roots for p in root.rglob("*.png"))
    print(f"{len(files)} files")

    stats: Counter[str] = Counter()
    mismatches: list[tuple[str, str]] = []
    rejected: list[tuple[str, str]] = []
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "out.pam"
        for path in files:
            try:
                with Image.open(path) as im:
                    im.load()
                    size, mode = im.size, im.mode
                    if mode.startswith("I;16"):
                        # Pillow's RGBA conversion clips 16-bit gray instead
                        # of scaling it, so take the high byte itself, the
                        # way Pillow does for 16-bit RGB.
                        key = im.info.get("transparency")
                        flat = getattr(im, "get_flattened_data", im.getdata)()
                        expected = bytes(
                            v >> 8
                            for g in (int(x) for x in flat)
                            for v in (g, g, g, 0 if g == key else 0xFFFF)
                        )
                    else:
                        expected = im.convert("RGBA").tobytes()
            except Exception as e:
                stats[f"pillow could not read it ({type(e).__name__})"] += 1
                continue
            run = subprocess.run(
                [str(EXE), str(path), "-o", str(out)],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
            )
            if run.returncode != 0:
                reason = next(
                    (
                        line.split("rejected", 1)[1].strip()
                        for line in run.stdout.splitlines()
                        if "rejected" in line
                    ),
                    run.stdout.strip(),
                )
                rejected.append((path.name, reason))
                continue
            width, height, _, got = read_pam(out)
            if (width, height) != size:
                mismatches.append((path.name, f"{width}x{height}, Pillow says {size[0]}x{size[1]}"))
            elif got == expected:
                stats["identical"] += 1
            else:
                differing = [i for i, (a, b) in enumerate(zip(got, expected)) if a != b]
                worst = max(abs(got[i] - expected[i]) for i in differing)
                if worst <= 1:
                    stats["off by one, 16-bit rounding"] += 1
                elif all(i % 4 == 3 for i in differing):
                    # Only the alpha channel: Pillow compares a tRNS key
                    # against the sample after rescaling it, so it misses
                    # transparent pixels below 8 bits (PngSuite tbbn0g04).
                    stats["alpha only, Pillow's tRNS bug"] += 1
                else:
                    mismatches.append(
                        (
                            path.name,
                            f"{len(differing)} channels differ, worst {worst}, mode {mode}",
                        )
                    )

    for label, count in stats.most_common():
        print(f"  {count:6d}  {label}")
    print(f"  {len(rejected):6d}  rejected by paeth")
    for name, reason in rejected[:20]:
        print(f"          {name}: {reason}")
    print(f"  {len(mismatches):6d}  mismatches")
    for name, reason in mismatches[:20]:
        print(f"          {name}: {reason}")
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
