//! The translation files, and the text engine's two encodings of Latin
//! text.
//!
//! A file holds entries, each a header line `[Label]` (the decomp's label
//! for the text: letters, digits, `_`, `.`, `+`), perhaps followed by
//! `full-width`, then the text's lines:
//!
//! ```text
//! # A comment: any line starting with '#'.
//!
//! [MainMenuNewGameText]
//! New Game
//!
//! [KnockOutText]
//! `Knocked Out   `
//!
//! [Text0cb7] full-width
//! Use it in your duels
//! from now on.
//! ```
//!
//! - Each line of the entry is a line on screen. Trailing blank lines are
//!   not part of the text, and trailing spaces are dropped; a line written
//!   between backticks is taken exactly, spaces and all (for padding).
//! - A text is half-width ([`Width::Half`]): two letters a tile
//!   (`TX_HALFWIDTH`), from the language's own font (`font.txt`). Or it is
//!   full-width ([`Width::Full`]): a letter a tile, 8×8, from the ROM's
//!   full-width font, the one the name keyboard types. Full-width text
//!   needs twice the room; the crate's tests check that each full-width
//!   text fits where the game prints it.
//! - `<RAMNAME>` is the player's name (in a duel, the name of the duelist
//!   whose turn it is), `<RAMTEXT>` a text the game chose (a card's name,
//!   say) and `<RAMNUM>` a number. `<NAME>` is one of the game's symbols
//!   ([`SYMBOLS`]: `<FIRE>`, `<Lv>`, `<HP>`, ...), drawn a tile wide.
//! - Every other character is one of the half-width font's: ASCII letters,
//!   digits, space and ``! " & ' ( ) * + , - . / : ; = ? [ \ ] ^ _ { | }``,
//!   plus `É é ♂ ♀`. Typographic quotes become the font's straight ones. A
//!   language that encodes text this way brings its own font (`font.txt`),
//!   with 'p' at [`LETTER_P`]. Full-width text has all of these but
//!   ``\ { | }`` and `É` ([`full_width`]).

/// How a text is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Width {
    /// Two letters a tile, from the language's font.
    #[default]
    Half,
    /// A letter a tile, from the ROM's full-width font.
    Full,
}

/// What follows a label in the header of a full-width text.
pub const FULL_WIDTH: &str = "full-width";

/// One translated text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The decomp's label for the text (`src/text/text*.asm`).
    pub label: String,
    /// The text, lines joined with '\n', tokens as written.
    pub text: String,
    pub width: Width,
    /// Where it was written: the file and the header's line.
    pub file: &'static str,
    pub line: usize,
}

/// Reads a translation file.
pub fn parse(file: &'static str, source: &str) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let finish = |entries: &mut Vec<Entry>, lines: &mut Vec<String>| {
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        if let Some(entry) = entries.last_mut() {
            entry.text = lines.join("\n");
        }
        lines.clear();
    };
    for (number, raw) in source.lines().enumerate() {
        let number = number + 1;
        if raw.starts_with('#') {
            continue;
        }
        // A header is a label in brackets; other bracketed lines are text
        // ("[Basic Energy Card]").
        if let Some((label, width)) = header(raw) {
            finish(&mut entries, &mut lines);
            entries.push(Entry {
                label: label.to_string(),
                text: String::new(),
                width,
                file,
                line: number,
            });
            continue;
        }
        if entries.is_empty() {
            if raw.trim().is_empty() {
                continue;
            }
            return Err(format!("{file}:{number}: text before the first [Label]"));
        }
        let line = match raw.strip_prefix('`').and_then(|r| r.strip_suffix('`')) {
            Some(exact) => exact.to_string(),
            None if raw.contains('`') => {
                return Err(format!(
                    "{file}:{number}: a backtick line must start and end with one"
                ));
            }
            None => raw.trim_end().to_string(),
        };
        lines.push(line);
    }
    finish(&mut entries, &mut lines);
    Ok(entries)
}

/// A header line's label and width.
pub fn header(line: &str) -> Option<(&str, Width)> {
    let line = line.trim_end();
    let (line, width) = match line.strip_suffix(FULL_WIDTH) {
        Some(rest) if rest.ends_with("] ") => (rest.trim_end(), Width::Full),
        _ => (line, Width::Half),
    };
    let label = line.strip_prefix('[')?.strip_suffix(']')?;
    let valid = !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+'));
    valid.then_some((label, width))
}

/// Ends a text.
pub const TX_END: u8 = 0x00;
/// The next byte is a character of the full-width font of Latin letters
/// and signs (`gfx/fonts/full_width/4.png`).
pub const TX_FULLWIDTH4: u8 = 0x04;
/// The next byte is a symbol tile (`SYMBOLS`).
pub const TX_SYMBOL: u8 = 0x05;
/// The text that follows is half-width.
pub const TX_HALFWIDTH: u8 = 0x06;
pub const TX_RAM1: u8 = 0x09;
pub const TX_LINE: u8 = 0x0a;
pub const TX_RAM2: u8 = 0x0b;
pub const TX_RAM3: u8 = 0x0c;

/// The game's symbol tiles (`TX_SYMBOL`, `src/constants/charmaps.asm`), by
/// number.
pub const SYMBOLS: [&str; 0x37] = [
    "SPACE",
    "FIRE",
    "GRASS",
    "LIGHTNING",
    "WATER",
    "FIGHTING",
    "PSYCHIC",
    "COLORLESS",
    "RAINBOW",
    "POISONED",
    "ASLEEP",
    "CONFUSED",
    "PARALYZED",
    "CURSOR_U",
    "REF_MARK",
    "CURSOR_R",
    "HP",
    "Lv",
    "E",
    "No",
    "PLUSPOWER",
    "DEFENDER",
    "HP_OK",
    "HP_NOK",
    "BOX_TOP_L",
    "BOX_TOP_R",
    "BOX_BTM_L",
    "BOX_BTM_R",
    "BOX_TOP",
    "BOX_BOTTOM",
    "BOX_LEFT",
    "BOX_RIGHT",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "DOT",
    "PLUS",
    "MINUS",
    "CROSS",
    "SLASH",
    "CURSOR_D",
    "PRIZE",
    "BOX_ALT_1",
    "BOX_ALT_2",
    "BOX_ALT_3",
    "BOX_ALT_4",
    "POKEMON",
    "CHERRY",
];

/// The game pads names and lines with $70, its full-width blank, also after
/// half-width text, where $70 is the letter 'p' (`CopyCardNameAndLevel`
/// fills a list row with it). The port's fonts therefore draw $70 blank and
/// 'p' at $3C, a code no text needs ('<').
pub const LETTER_P: u8 = 0x3c;
pub const PADDING: u8 = 0x70;

/// The half-width code for a character: the ROM's charmap
/// (`src/constants/charmaps.asm`), but for 'p' ([`LETTER_P`]).
pub fn half_width(c: char) -> Option<u8> {
    Some(match c {
        'p' => LETTER_P,
        ' ' | '!' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | '-' | '.' | '/' | ':' | ';'
        | '=' | '?' | '[' | '\\' | ']' | '^' | '_' | '{' | '}' => c as u8,
        '0'..='9' | 'A'..='Z' | 'a'..='z' => c as u8,
        '"' | '“' | '”' => 0x22,
        '‘' | '’' => 0x27,
        '≠' => 0x23,
        '♂' => 0x24,
        '♀' => 0x25,
        'É' => 0x40,
        'é' => 0x60,
        '¦' => 0x7c,
        '|' => 0x7e,
        '‾' => 0x7f,
        _ => return None,
    })
}

/// A character's code in full-width text, as the engine reads it
/// (`ClassifyTextCharacterPair`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullWidth {
    /// One byte from $60: the ROM's digits and the signs kept with them
    /// (`0_2_digits_kanji1`, read alike whichever kana is set).
    Byte(u8),
    /// `TX_FULLWIDTH4` and this code: the ROM's font of Latin letters and
    /// signs (`4.png`), whose letters the name keyboard types.
    Font4(u8),
}

/// The full-width code for a character: the decomp's charmap where it has
/// the character, so letters, digits and signs are the ones the name
/// keyboard types and a typed name matches the text around it. The font
/// also holds a copy of ASCII that no text uses (`a`-`z` at $66, ' ' to
/// '_' at $85), which gives the quotes, the comma, the period and the
/// brackets; `é` takes the copy of `e`, which the language redraws
/// (`fullwidth.txt`).
pub fn full_width(c: char) -> Option<FullWidth> {
    use FullWidth::{Byte, Font4};
    Some(match c {
        'A'..='Z' => Font4(0x10 + (c as u8 - b'A')),
        'a'..='z' => Font4(0x2a + (c as u8 - b'a')),
        '0'..='9' => Byte(0x60 + (c as u8 - b'0')),
        '+' => Byte(0x6a),
        '-' => Byte(0x6b),
        '/' => Byte(0x6d),
        '!' => Byte(0x6e),
        '?' => Byte(0x6f),
        ' ' => Byte(PADDING),
        '(' => Byte(0x71),
        ')' => Byte(0x72),
        '"' | '“' | '”' => Font4(0x87),
        '&' => Font4(0x5e),
        '\'' | '‘' | '’' => Font4(0x8c),
        '*' => Font4(0x5f),
        ',' => Font4(0x91),
        '.' => Font4(0x93),
        ':' => Font4(0x4e),
        ';' => Font4(0x4f),
        '=' => Font4(0x62),
        '[' => Font4(0xc0),
        ']' => Font4(0xc2),
        '^' => Font4(0x47),
        '_' => Font4(0x57),
        '♀' => Font4(0x49),
        '♂' => Font4(0x4a),
        'é' => Font4(0x6a),
        _ => return None,
    })
}

/// One piece of a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Char(char),
    Line,
    /// `<RAMNAME>`, `<RAMTEXT>` or `<RAMNUM>`: the control code.
    Ram(u8),
    Symbol(u8),
}

/// Splits a text into what the engine draws.
pub fn pieces(text: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            let end = rest.find('>').ok_or("a '<' without its '>'")?;
            let name = &rest[1..end];
            out.push(match name {
                "RAMNAME" => Piece::Ram(TX_RAM1),
                "RAMTEXT" => Piece::Ram(TX_RAM2),
                "RAMNUM" => Piece::Ram(TX_RAM3),
                _ => match SYMBOLS.iter().position(|&s| s == name) {
                    Some(n) => Piece::Symbol(n as u8),
                    None => return Err(format!("unknown token <{name}>")),
                },
            });
            rest = &rest[end + 1..];
            continue;
        }
        out.push(match c {
            '\n' => Piece::Line,
            _ => Piece::Char(c),
        });
        rest = &rest[c.len_utf8()..];
    }
    Ok(out)
}

/// Encodes a text as the game stores it. Half-width: `TX_HALFWIDTH`, the
/// text, `TX_END`. Full-width: the text and `TX_END`, as the ROM's own
/// Japanese texts are (each print starts full-width).
pub fn encode(text: &str, width: Width) -> Result<Vec<u8>, String> {
    let mut out = match width {
        Width::Half => vec![TX_HALFWIDTH],
        Width::Full => Vec::new(),
    };
    for piece in pieces(text)? {
        match piece {
            Piece::Char(c) => match width {
                Width::Half => out.push(
                    half_width(c).ok_or_else(|| format!("{c:?} is not in the half-width font"))?,
                ),
                Width::Full => match full_width(c) {
                    Some(FullWidth::Byte(b)) => out.push(b),
                    Some(FullWidth::Font4(b)) => out.extend([TX_FULLWIDTH4, b]),
                    None => return Err(format!("{c:?} is not in the full-width font")),
                },
            },
            Piece::Line => out.push(TX_LINE),
            Piece::Ram(code) => out.push(code),
            Piece::Symbol(n) => out.extend([TX_SYMBOL, n]),
        }
    }
    out.push(TX_END);
    Ok(out)
}

/// Each line's width in half tiles as the engine lays it out. Half-width:
/// two characters a tile, and a symbol completes the tile before it and
/// takes a whole one. Full-width: a tile each. What the game fills in
/// counts at its likely widest: a player's name 12 (6 full-width letters),
/// a number 3 half-width digits or 3 symbol digits, a text 16 in half-width
/// text (a card's name) and 24 in full-width text, where a half-width
/// card's name of up to 24 letters may stand.
pub fn line_widths(text: &str, width: Width) -> Result<Vec<usize>, String> {
    let full = width == Width::Full;
    let mut widths = vec![0usize];
    for piece in pieces(text)? {
        let w = widths.last_mut().unwrap();
        match piece {
            Piece::Char(_) if full => *w += 2,
            Piece::Char(_) => *w += 1,
            Piece::Line => widths.push(0),
            Piece::Ram(TX_RAM1) => *w += 12,
            Piece::Ram(TX_RAM2) if full => *w += 24,
            Piece::Ram(TX_RAM2) => *w += 16,
            Piece::Ram(_) if full => *w += 6,
            Piece::Ram(_) => *w += 3,
            Piece::Symbol(_) => *w = w.next_multiple_of(2) + 2,
        }
    }
    Ok(widths)
}
