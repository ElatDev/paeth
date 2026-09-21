//! Pass geometry for Adam7 interlacing (PNG spec, section 8.2).
//!
//! Adam7 sends the image in seven passes over a repeating 8×8 grid, each
//! a complete sub-image with its own scanlines and its own filtering. A
//! non-interlaced image is described here as a single pass covering
//! every pixel, so the decoder needs only one code path.

use crate::header::{Header, Interlace};

/// One pass: the pixels at `(x0 + i·dx, y0 + j·dy)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pass {
    pub x0: u32,
    pub y0: u32,
    pub dx: u32,
    pub dy: u32,
    /// Pixels per row of this pass. Zero when the image is too narrow
    /// to reach the pass's first column.
    pub width: u32,
    /// Rows in this pass. Zero when the image is too short.
    pub height: u32,
}

impl Pass {
    /// Passes with no pixels contribute no scanlines, not even filter
    /// bytes.
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// `(x0, y0, dx, dy)` for passes 1 to 7.
const ADAM7: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// The passes that make up an image, in stream order.
pub(crate) fn passes(header: &Header) -> Vec<Pass> {
    let (w, h) = (header.width, header.height);
    match header.interlace {
        Interlace::None => vec![Pass {
            x0: 0,
            y0: 0,
            dx: 1,
            dy: 1,
            width: w,
            height: h,
        }],
        Interlace::Adam7 => ADAM7
            .iter()
            .map(|&(x0, y0, dx, dy)| Pass {
                x0,
                y0,
                dx,
                dy,
                width: span(w, x0, dx),
                height: span(h, y0, dy),
            })
            .collect(),
    }
}

/// How many of `start, start + step, ...` fall below `size`.
fn span(size: u32, start: u32, step: u32) -> u32 {
    if size > start {
        (size - start).div_ceil(step)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::ColorType;

    /// The pass number of every position in the 8×8 tile, copied from
    /// the diagram in the spec.
    const TILE: [[u8; 8]; 8] = [
        [1, 6, 4, 6, 2, 6, 4, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [5, 6, 5, 6, 5, 6, 5, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [3, 6, 4, 6, 3, 6, 4, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [5, 6, 5, 6, 5, 6, 5, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
    ];

    fn header(width: u32, height: u32, interlace: Interlace) -> Header {
        Header {
            width,
            height,
            bit_depth: 8,
            color_type: ColorType::Grayscale,
            interlace,
        }
    }

    #[test]
    fn passes_match_the_spec_diagram_for_every_small_size() {
        for w in 1..=24 {
            for h in 1..=24 {
                let passes = passes(&header(w, h, Interlace::Adam7));
                for (p, pass) in passes.iter().enumerate() {
                    // Pixels the diagram assigns to this pass, in raster order.
                    let expected: Vec<(u32, u32)> = (0..h)
                        .flat_map(|y| (0..w).map(move |x| (x, y)))
                        .filter(|&(x, y)| {
                            usize::from(TILE[y as usize % 8][x as usize % 8]) == p + 1
                        })
                        .collect();
                    // Pixels the pass geometry claims, in the same order.
                    let claimed: Vec<(u32, u32)> = (0..pass.height)
                        .flat_map(|j| {
                            (0..pass.width)
                                .map(move |i| (pass.x0 + i * pass.dx, pass.y0 + j * pass.dy))
                        })
                        .collect();
                    assert_eq!(claimed, expected, "{w}x{h}, pass {}", p + 1);
                }
            }
        }
    }

    #[test]
    fn every_pixel_belongs_to_exactly_one_pass() {
        for (w, h) in [(1, 1), (7, 3), (8, 8), (33, 17), (100, 1), (1, 100)] {
            let total: u64 = passes(&header(w, h, Interlace::Adam7))
                .iter()
                .map(|p| u64::from(p.width) * u64::from(p.height))
                .sum();
            assert_eq!(total, u64::from(w) * u64::from(h), "{w}x{h}");
        }
    }

    #[test]
    fn a_1x1_image_has_only_pass_one() {
        let non_empty: Vec<_> = passes(&header(1, 1, Interlace::Adam7))
            .iter()
            .map(|p| !p.is_empty())
            .collect();
        assert_eq!(non_empty, [true, false, false, false, false, false, false]);
    }

    #[test]
    fn narrow_images_skip_passes_that_start_to_the_right() {
        // Width 4 reaches column 0 to 3: pass 2 starts at column 4.
        let passes = passes(&header(4, 8, Interlace::Adam7));
        assert!(passes[1].is_empty());
        assert_eq!((passes[0].width, passes[0].height), (1, 1));
        assert_eq!((passes[6].width, passes[6].height), (4, 4));
    }

    #[test]
    fn non_interlaced_is_one_full_pass() {
        assert_eq!(
            passes(&header(5, 3, Interlace::None)),
            [Pass {
                x0: 0,
                y0: 0,
                dx: 1,
                dy: 1,
                width: 5,
                height: 3
            }]
        );
    }

    #[test]
    fn huge_dimensions_do_not_overflow() {
        let max = 0x7FFF_FFFF;
        let passes = passes(&header(max, max, Interlace::Adam7));
        assert_eq!(passes[0].width, max.div_ceil(8));
        assert_eq!(passes[6].height, (max - 1).div_ceil(2));
    }
}
