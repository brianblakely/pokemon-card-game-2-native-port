//! A language's own drawings of pictures with lettering
//! (`locales/<tag>/graphics.txt`), in place of the ROM's.
//!
//! Each picture is a line `[Label] WxH` (the decomp's label for the picture,
//! `[Label+N]` for one N bytes after it, and its size) followed by H rows of
//! W pixels: `.`, `:`, `x` and `#` are the
//! picture's colors 0 (white) to 3 (black). It is stored as the ROM stores
//! the original: 8x8 tiles, left to right and top to bottom, each row two
//! bytes (the low bits of its eight pixels, then the high bits).

/// One picture: its label and its tiles.
pub struct Picture {
    pub label: String,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<u8>,
}

/// Reads a graphics file.
pub fn parse(source: &str) -> Result<Vec<Picture>, String> {
    let mut pictures = Vec::new();
    let mut lines = source
        .lines()
        .enumerate()
        // Comments start with "# "; pixel rows ("#::#") never hold a space.
        .filter(|(_, l)| !l.starts_with("# ") && !l.trim().is_empty());
    while let Some((number, header)) = lines.next() {
        let bad = || format!("line {}: expected [Label] WIDTHxHEIGHT", number + 1);
        let (label, size) = header
            .strip_prefix('[')
            .and_then(|h| h.split_once("] "))
            .ok_or_else(bad)?;
        let (width, height) = size.split_once('x').ok_or_else(bad)?;
        let (width, height): (usize, usize) = (
            width.parse().map_err(|_| bad())?,
            height.parse().map_err(|_| bad())?,
        );
        if width == 0 || height == 0 || width % 8 != 0 || height % 8 != 0 {
            return Err(format!("line {}: a picture is whole 8x8 tiles", number + 1));
        }
        let mut pixels = Vec::with_capacity(width * height);
        for _ in 0..height {
            let (number, row) = lines
                .next()
                .ok_or(format!("[{label}] has fewer than {height} rows"))?;
            if row.chars().count() != width {
                return Err(format!(
                    "line {}: a row of [{label}] is {width} pixels",
                    number + 1
                ));
            }
            for c in row.chars() {
                pixels.push(match c {
                    '.' => 0u8,
                    ':' => 1,
                    'x' => 2,
                    '#' => 3,
                    _ => return Err(format!("line {}: {c:?} is no color", number + 1)),
                });
            }
        }
        let mut tiles = Vec::with_capacity(width * height / 4);
        for ty in 0..height / 8 {
            for tx in 0..width / 8 {
                for y in 0..8 {
                    let (mut low, mut high) = (0u8, 0u8);
                    for x in 0..8 {
                        let c = pixels[(ty * 8 + y) * width + tx * 8 + x];
                        low |= (c & 1) << (7 - x);
                        high |= (c >> 1) << (7 - x);
                    }
                    tiles.extend([low, high]);
                }
            }
        }
        pictures.push(Picture {
            label: label.to_string(),
            width,
            height,
            tiles,
        });
    }
    Ok(pictures)
}
