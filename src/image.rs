use std::fmt;

/// Decoded pixels.
///
/// Four channels per pixel in R, G, B, A order, rows top to bottom,
/// no padding. Alpha is straight, not premultiplied. `C` is `u8` or
/// `u16`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Image<C> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height × 4` channel values.
    pub pixels: Vec<C>,
}

impl<C: Copy> Image<C> {
    /// The pixel at `(x, y)`, counting from the top left.
    ///
    /// # Panics
    ///
    /// If the coordinates are outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> [C; 4] {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the image"
        );
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// One row of pixels, `width × 4` values.
    ///
    /// # Panics
    ///
    /// If `y` is outside the image.
    pub fn row(&self, y: u32) -> &[C] {
        assert!(y < self.height, "row {y} is outside the image");
        let len = self.width as usize * 4;
        &self.pixels[y as usize * len..(y as usize + 1) * len]
    }
}

impl<C> fmt::Debug for Image<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("channel", &std::any::type_name::<C>())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 image whose pixels are (10x + y) in every channel, so a
    /// wrong index shows up as a wrong value rather than a panic.
    fn sample() -> Image<u8> {
        let mut pixels = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                pixels.extend_from_slice(&[10 * x + y, 10 * x + y + 1, 10 * x + y + 2, 255]);
            }
        }
        Image {
            width: 3,
            height: 2,
            pixels,
        }
    }

    #[test]
    fn pixel_indexes_row_major_from_the_top_left() {
        let img = sample();
        assert_eq!(img.pixel(0, 0), [0, 1, 2, 255]);
        assert_eq!(img.pixel(1, 0), [10, 11, 12, 255]);
        assert_eq!(img.pixel(2, 0), [20, 21, 22, 255]);
        assert_eq!(img.pixel(0, 1), [1, 2, 3, 255]);
        assert_eq!(img.pixel(2, 1), [21, 22, 23, 255]);
    }

    #[test]
    fn row_returns_one_row_of_four_channel_pixels() {
        let img = sample();
        assert_eq!(img.row(0), [0, 1, 2, 255, 10, 11, 12, 255, 20, 21, 22, 255]);
        assert_eq!(img.row(1), [1, 2, 3, 255, 11, 12, 13, 255, 21, 22, 23, 255]);
        assert_eq!(img.row(1).len(), img.width as usize * 4);
        // Every row together is the whole buffer.
        let joined: Vec<u8> = (0..img.height).flat_map(|y| img.row(y).to_vec()).collect();
        assert_eq!(joined, img.pixels);
    }

    #[test]
    #[should_panic(expected = "outside the image")]
    fn pixel_past_the_right_edge_panics() {
        sample().pixel(3, 0);
    }

    #[test]
    #[should_panic(expected = "outside the image")]
    fn pixel_past_the_bottom_edge_panics() {
        sample().pixel(0, 2);
    }

    #[test]
    #[should_panic(expected = "outside the image")]
    fn row_past_the_bottom_edge_panics() {
        sample().row(2);
    }

    #[test]
    fn debug_summarises_instead_of_printing_every_pixel() {
        let text = format!("{:?}", sample());
        assert!(text.contains("width: 3"), "{text}");
        assert!(text.contains("height: 2"), "{text}");
        assert!(text.contains("u8"), "{text}");
        assert!(!text.contains("255"), "{text}");
    }
}
