//! A language's own drawings of pictures with lettering
//! (`locales/<tag>/graphics.txt`), drawn over the ROM's.
//!
//! Each picture is a line `[Label] WxH` (the decomp's label for the picture,
//! `[Label+N]` for one N bytes after it, and its size) followed by H rows of
//! W pixels: `.`, `:`, `x` and `#` are the picture's colors 0 (white) to 3
//! (black), and `?` keeps the ROM's pixel. Its 8x8 blocks are the ROM's tiles
//! from the label on, left to right and top to bottom, as the ROM stores the
//! original. A picture the game puts together from scattered tiles (a
//! sprite, a sign) is drawn as it appears instead, with a `tiles` line before
//! the rows for each row of blocks, giving each block's tile number (in hex,
//! counted from the label) or `--` for none. A tile drawn twice is drawn the
//! same both times.
//!
//! A tile is 16 bytes: each row two bytes, the low bits of its eight pixels,
//! then the high bits.

use std::collections::HashMap;

/// One picture: its label, the tile under each block, and its pixels.
pub struct Picture {
    pub label: String,
    pub width: usize,
    pub height: usize,
    /// Each 8x8 block's tile, counted from the label (`None`: none).
    pub tiles: Vec<Option<usize>>,
    /// The pixels, row by row: a color, or `None` to keep the ROM's.
    pub pixels: Vec<Option<u8>>,
}

impl Picture {
    /// The picture's blocks: each one's tile and 64 pixels.
    fn blocks(&self) -> impl Iterator<Item = (Option<usize>, [Option<u8>; 64])> + '_ {
        let across = self.width / 8;
        self.tiles.iter().enumerate().map(move |(i, &tile)| {
            let (bx, by) = (i % across * 8, i / across * 8);
            let mut pixels = [None; 64];
            for (j, p) in pixels.iter_mut().enumerate() {
                *p = self.pixels[(by + j / 8) * self.width + bx + j % 8];
            }
            (tile, pixels)
        })
    }

    /// Draws the picture over the ROM's tiles, `rom` starting at its label.
    pub fn draw(&self, rom: &mut [u8]) -> Result<(), String> {
        for (tile, pixels) in self.blocks() {
            let Some(tile) = tile else { continue };
            let bytes = rom
                .get_mut(tile * 16..tile * 16 + 16)
                .ok_or(format!("tile {tile:02x} is past the ROM's end"))?;
            for (y, row) in bytes.chunks_exact_mut(2).enumerate() {
                for (x, pixel) in pixels[y * 8..y * 8 + 8].iter().enumerate() {
                    if let Some(c) = pixel {
                        let bit = 0x80 >> x;
                        row[0] = row[0] & !bit | if c & 1 != 0 { bit } else { 0 };
                        row[1] = row[1] & !bit | if c & 2 != 0 { bit } else { 0 };
                    }
                }
            }
        }
        Ok(())
    }
}

/// Reads a graphics file.
pub fn parse(source: &str) -> Result<Vec<Picture>, String> {
    let mut pictures = Vec::new();
    // Every block drawn into each tile, to check that a tile drawn twice
    // comes out the same.
    let mut drawn: HashMap<(String, usize), [Option<u8>; 64]> = HashMap::new();
    let mut lines = source
        .lines()
        .enumerate()
        // Comments start with "# "; pixel rows ("#::#") never hold a space.
        .filter(|(_, l)| !l.starts_with("# ") && !l.trim().is_empty())
        .peekable();
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
        let mut tiles = Vec::with_capacity(width * height / 64);
        while let Some((number, line)) = lines.next_if(|(_, l)| l.starts_with("tiles ")) {
            let row: Vec<_> = line["tiles ".len()..].split_whitespace().collect();
            if row.len() != width / 8 {
                return Err(format!(
                    "line {}: a tiles line of [{label}] names {} tiles",
                    number + 1,
                    width / 8
                ));
            }
            for tile in row {
                tiles.push(match tile {
                    "--" => None,
                    _ => {
                        Some(usize::from_str_radix(tile, 16).map_err(|_| {
                            format!("line {}: {tile:?} is no tile number", number + 1)
                        })?)
                    }
                });
            }
        }
        if tiles.is_empty() {
            tiles = (0..width * height / 64).map(Some).collect();
        } else if tiles.len() != width * height / 64 {
            return Err(format!("[{label}] needs {} tiles lines", height / 8));
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
                    '.' => Some(0u8),
                    ':' => Some(1),
                    'x' => Some(2),
                    '#' => Some(3),
                    '?' => None,
                    _ => return Err(format!("line {}: {c:?} is no color", number + 1)),
                });
            }
        }
        let picture = Picture {
            label: label.to_string(),
            width,
            height,
            tiles,
            pixels,
        };
        for (tile, block) in picture.blocks() {
            let Some(tile) = tile else {
                if block.iter().any(Option::is_some) {
                    return Err(format!("[{label}] draws where it has no tile"));
                }
                continue;
            };
            if *drawn.entry((label.to_string(), tile)).or_insert(block) != block {
                return Err(format!("[{label}] draws tile {tile:02x} two ways"));
            }
        }
        pictures.push(picture);
    }
    Ok(pictures)
}
