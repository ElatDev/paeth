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
