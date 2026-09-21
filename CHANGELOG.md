# Changelog

## v0.1.0 — 2026-09-20

First release.

- Decodes every PNG color type and bit depth, interlaced or not, to RGBA at 8
  or 16 bits per channel.
- Verifies every chunk CRC and enforces the chunk ordering rules; rejects the
  files the specification calls broken, and says which rule was broken.
- Reports the signature damage caused by 7-bit transfers and text-mode
  newline conversion.
- Parses the ancillary chunks (including the PNG Third Edition cICP, mDCV and
  cLLI) without applying them; malformed ones become warnings, not errors.
- `paeth` command line tool: per-file report, chunk listing, terminal preview,
  raw PAM pixel dump.
- Conformance: all 162 valid PngSuite images decode identically to pypng, all
  14 corrupt ones are rejected for the documented reason.
- Robustness: 1,584,000 mutated files decoded without a panic, and 16,652
  real-world PNGs decoded byte-identically to Pillow.
- 153 tests, and the tooling to reproduce every claim: reference hashes from
  pypng, README figures drawn from the CLI's own output, a corpus comparison
  against Pillow, and mutation testing over the library.
