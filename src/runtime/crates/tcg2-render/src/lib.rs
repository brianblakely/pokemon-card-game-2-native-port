//! The look of the game's 160x144 picture at any window size.
//!
//! The host scales one 160x144 texture into the window with its 2D
//! renderer (`Filter` picks the sampling, `Fit` the rectangle). The LCD's
//! colour response and frame blending are applied here, on the CPU, to the
//! 23,040 pixels of each new picture: a few microseconds per changed frame,
//! nothing at all while the screen is still.

pub const SOURCE_WIDTH: u32 = 160;
pub const SOURCE_HEIGHT: u32 = 144;
const PIXELS: usize = (SOURCE_WIDTH * SOURCE_HEIGHT) as usize;

/// How texels map to window pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    /// Crisp square pixels at any scale (nearest, with a one-pixel blend at
    /// texel edges: SDL's pixel-art scaling).
    #[default]
    Sharp,
    /// Nearest texel (exact at integer scales).
    Nearest,
    /// Bilinear.
    Smooth,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::Sharp, Filter::Nearest, Filter::Smooth];

    pub fn name(self) -> &'static str {
        match self {
            Filter::Sharp => "sharp",
            Filter::Nearest => "nearest",
            Filter::Smooth => "smooth",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }
}

/// How the picture fills the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fit {
    /// As large as fits, keeping the 10:9 aspect.
    #[default]
    Aspect,
    /// The largest whole multiple of 160x144 that fits.
    Integer,
    /// The whole window.
    Stretch,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Aspect, Fit::Integer, Fit::Stretch];

    pub fn name(self) -> &'static str {
        match self {
            Fit::Aspect => "aspect",
            Fit::Integer => "integer",
            Fit::Stretch => "stretch",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }

    /// The picture's rectangle (x, y, width, height) in a window of `size`.
    pub fn viewport(self, size: (u32, u32)) -> (f32, f32, f32, f32) {
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let (sw, sh) = (SOURCE_WIDTH as f32, SOURCE_HEIGHT as f32);
        let (vw, vh) = match self {
            Fit::Stretch => (w, h),
            Fit::Aspect => {
                let scale = (w / sw).min(h / sh);
                (sw * scale, sh * scale)
            }
            Fit::Integer => {
                let scale = (w / sw).min(h / sh).floor().max(1.0);
                (sw * scale, sh * scale)
            }
        };
        (
            ((w - vw) / 2.0).floor(),
            ((h - vh) / 2.0).floor(),
            vw.round(),
            vh.round(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Look {
    pub filter: Filter,
    pub fit: Fit,
    /// The GBC LCD's colour response instead of the raw colours.
    pub lcd_colours: bool,
    /// Shows half of the previous frame (the LCD's slow response, which
    /// some of the game's flicker effects rely on).
    pub frame_blending: bool,
}

/// Turns the console's frames into the picture to show.
pub struct Picture {
    current: Vec<u8>,
    previous: Vec<u8>,
    shown: Vec<u8>,
    /// LCD colour for each 15-bit console colour, built on first use.
    lcd: Option<Box<[[u8; 3]]>>,
}

impl Default for Picture {
    fn default() -> Self {
        Self {
            current: vec![0; PIXELS * 4],
            previous: vec![0; PIXELS * 4],
            shown: vec![0; PIXELS * 4],
            lcd: None,
        }
    }
}

impl Picture {
    /// Takes a new frame (160x144 RGBA8); the last one becomes the previous.
    pub fn push(&mut self, rgba: &[u8]) {
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.copy_from_slice(rgba);
    }

    /// The picture to show (160x144 RGBA8) with `look`'s colours and
    /// blending.
    pub fn compose(&mut self, look: &Look) -> &[u8] {
        let lcd = look
            .lcd_colours
            .then(|| &**self.lcd.get_or_insert_with(lcd_table));
        let colour = |p: &[u8]| -> [u8; 3] {
            match lcd {
                // The console's colours have 5 bits per channel.
                Some(table) => {
                    table[(p[0] as usize >> 3)
                        | (p[1] as usize >> 3) << 5
                        | (p[2] as usize >> 3) << 10]
                }
                None => [p[0], p[1], p[2]],
            }
        };
        let pixels = self.shown.chunks_exact_mut(4).zip(
            self.current
                .chunks_exact(4)
                .zip(self.previous.chunks_exact(4)),
        );
        for (out, (now, before)) in pixels {
            let mut c = colour(now);
            if look.frame_blending {
                let b = colour(before);
                for (c, b) in c.iter_mut().zip(b) {
                    *c = (*c as u16 + b as u16).div_ceil(2) as u8;
                }
            }
            out.copy_from_slice(&[c[0], c[1], c[2], 0xff]);
        }
        &self.shown
    }
}

/// The GBC's LCD: darker, less saturated colours than the raw 15-bit values.
fn lcd_table() -> Box<[[u8; 3]]> {
    (0..1 << 15)
        .map(|c: u32| {
            let l = |shift: u32| (((c >> shift) & 31) as f32 / 31.0).powf(2.2);
            let (r, g, b) = (l(0), l(5), l(10));
            let out = |v: f32| ((v / 32.0).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
            [
                out(r * 26.0 + g * 4.0 + b * 2.0),
                out(g * 24.0 + b * 8.0),
                out(r * 6.0 + g * 4.0 + b * 22.0),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewports_keep_the_aspect_or_whole_multiples() {
        assert_eq!(
            Fit::Aspect.viewport((1920, 1080)),
            (360.0, 0.0, 1200.0, 1080.0)
        );
        assert_eq!(
            Fit::Integer.viewport((1920, 1080)),
            (400.0, 36.0, 1120.0, 1008.0)
        );
        assert_eq!(Fit::Stretch.viewport((800, 600)), (0.0, 0.0, 800.0, 600.0));
        assert_eq!(
            Fit::Integer.viewport((100, 100)),
            (-30.0, -22.0, 160.0, 144.0)
        );
    }

    #[test]
    fn colours_pass_through_or_blend_with_the_previous_frame() {
        let mut picture = Picture::default();
        let frame = |c: u8| vec![c; PIXELS * 4];
        picture.push(&frame(0));
        picture.push(&frame(200));
        let raw = Look::default();
        assert_eq!(&picture.compose(&raw)[..4], &[200, 200, 200, 255]);
        let blended = Look {
            frame_blending: true,
            ..raw
        };
        assert_eq!(&picture.compose(&blended)[..4], &[100, 100, 100, 255]);
    }

    #[test]
    fn the_lcd_keeps_black_and_white_and_dims_colours() {
        let table = lcd_table();
        assert_eq!(table[0], [0, 0, 0]);
        assert_eq!(table[0x7fff], [255, 255, 255]);
        let red = table[31];
        assert!(red[0] < 255 && red[1] == 0 && red[2] > 0);
    }
}
