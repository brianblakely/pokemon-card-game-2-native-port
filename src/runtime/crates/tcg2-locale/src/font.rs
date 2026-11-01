//! A language's own half-width font (`locales/<tag>/font.txt`), drawn for
//! the port, in place of the ROM's (`HalfWidthFont`).
//!
//! After comment lines (`# ...`), the file holds the 96 characters from $20 to $7F in order, each a line
//! `XX c` (its code in hex, then the character) and eight rows of four
//! pixels (`#` ink, `.` paper). The game draws two characters per tile from
//! the left four pixels of each 1bpp row (`CreateHalfWidthFontTile`), so a
//! character keeps to columns 0-2 and leaves column 3 as the gap.

/// Characters in the font: $20-$7F.
pub const CHARS: usize = 96;
/// The font as the ROM stores it: 8 bytes (1bpp rows) per character.
pub const SIZE: usize = CHARS * 8;

/// Reads a font file into the ROM's layout.
pub fn parse(source: &str) -> Result<[u8; SIZE], String> {
    let mut font = [0u8; SIZE];
    let mut lines = source
        .lines()
        .enumerate()
        // Comments start with "# "; pixel rows ("#.#.") never hold a space.
        .filter(|(_, l)| !l.starts_with("# ") && *l != "#" && !l.trim().is_empty());
    for code in 0x20..0x20 + CHARS {
        let (number, header) = lines
            .next()
            .ok_or(format!("the font ends before character {code:02X}"))?;
        if !header.starts_with(&format!("{code:02X}")) {
            return Err(format!(
                "line {}: expected character {code:02X}",
                number + 1
            ));
        }
        for row in 0..8 {
            let (number, pixels) = lines
                .next()
                .ok_or(format!("character {code:02X} has fewer than 8 rows"))?;
            let bad = || {
                format!(
                    "line {}: a row is 4 of '#' or '.', with column 3 blank",
                    number + 1
                )
            };
            if pixels.len() != 4 || !pixels.chars().all(|c| c == '#' || c == '.') {
                return Err(bad());
            }
            if pixels.ends_with('#') {
                return Err(bad());
            }
            let bits = pixels
                .chars()
                .enumerate()
                .filter(|&(_, c)| c == '#')
                .fold(0u8, |b, (x, _)| b | 0x80 >> x);
            font[(code - 0x20) * 8 + row] = bits;
        }
    }
    if let Some((number, _)) = lines.next() {
        return Err(format!("line {}: more than {CHARS} characters", number + 1));
    }
    Ok(font)
}

/// Where the full-width font (`FullWidthFonts`) keeps the characters the
/// game's code writes itself (`GetFullWidthFontTileOffset`): hiragana and
/// `TX_FULLWIDTH0` codes are 8 bytes each after the 80 katakana tiles.
pub fn full_width_offset(c: char) -> Option<usize> {
    let code = match c {
        '枚' => 0x84,
        _ => return None,
    };
    Some(0x280 + code * 8)
}

/// Reads redrawn full-width characters: each a line holding the character,
/// then eight rows of eight pixels.
pub fn parse_full_width(source: &str) -> Result<Vec<(char, [u8; 8])>, String> {
    let mut glyphs = Vec::new();
    let mut lines = source
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with("# ") && *l != "#" && !l.trim().is_empty());
    while let Some((number, header)) = lines.next() {
        let mut chars = header.trim().chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return Err(format!("line {}: expected one character", number + 1));
        };
        if full_width_offset(c).is_none() {
            return Err(format!(
                "line {}: the game's code never writes {c:?}",
                number + 1
            ));
        }
        let mut glyph = [0u8; 8];
        for row in &mut glyph {
            let (number, pixels) = lines.next().ok_or(format!("{c:?} has fewer than 8 rows"))?;
            if pixels.len() != 8 || !pixels.chars().all(|p| p == '#' || p == '.') {
                return Err(format!("line {}: a row is 8 of '#' or '.'", number + 1));
            }
            *row = pixels
                .chars()
                .enumerate()
                .filter(|&(_, p)| p == '#')
                .fold(0u8, |b, (x, _)| b | 0x80 >> x);
        }
        glyphs.push((c, glyph));
    }
    Ok(glyphs)
}
