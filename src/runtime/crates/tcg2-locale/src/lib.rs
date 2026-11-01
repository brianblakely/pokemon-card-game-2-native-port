//! The game in other languages.
//!
//! Pokémon Card GB2 prints every text through one table: `TextOffsets`
//! gives each text number a 24-bit offset from the table's own bank, and
//! the game switches to the bank that offset falls in and prints from there
//! (`GetTextOffsetFromTextID`). The ROM's text is Japanese, drawn one
//! character per tile. The same text engine also draws half-width text,
//! two Latin letters per tile from the ROM's own half-width font, as the
//! first game's English release did.
//!
//! A language here is a set of translations keyed by the decomp's text
//! labels (`locales/<tag>/text*.txt`, one file per `src/text/text*.asm`;
//! the format is described in [`text`]). [`localize`] encodes them as
//! half-width text into banks $80-$FF of the ROM image, past the player's
//! 2 MiB but within reach of the game's 8-bit bank switch, and points the
//! table's entries at them. Code, data and the original text keep every
//! byte: the kernel, translated from the same program, runs unchanged, and
//! a text a language leaves out prints as the ROM has it.
//!
//! Every image holds every language's text and differs only in the table,
//! so switching language while the game runs is safe: a text being printed
//! at that moment keeps reading the bytes it started on.

use std::collections::HashMap;
use std::fmt;

pub mod font;
pub mod graphics;
pub mod text;

/// A language the game can be played in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// American English: the port's own translation, and the default.
    #[default]
    EnglishUs,
    /// Japanese: the game's text as the ROM has it.
    Japanese,
}

/// The translation files of each language: (file name, contents).
type Sources = &'static [(&'static str, &'static str)];

macro_rules! locale {
    ($tag:literal) => {
        &[
            locale!(@file $tag, "text1.txt"),
            locale!(@file $tag, "text2.txt"),
            locale!(@file $tag, "text3.txt"),
            locale!(@file $tag, "text4.txt"),
            locale!(@file $tag, "text5.txt"),
            locale!(@file $tag, "text6.txt"),
            locale!(@file $tag, "text7.txt"),
            locale!(@file $tag, "text8.txt"),
            locale!(@file $tag, "text9.txt"),
            locale!(@file $tag, "text10.txt"),
            locale!(@file $tag, "text11.txt"),
            locale!(@file $tag, "text12.txt"),
            locale!(@file $tag, "text13.txt"),
            locale!(@file $tag, "text14.txt"),
            locale!(@file $tag, "text15.txt"),
            locale!(@file $tag, "text16.txt"),
            locale!(@file $tag, "text17.txt"),
        ]
    };
    (@file $tag:literal, $file:literal) => {
        (
            concat!($tag, "/", $file),
            include_str!(concat!("../locales/", $tag, "/", $file)),
        )
    };
}

impl Language {
    /// Every language, in the order a host offers them.
    pub const ALL: [Language; 2] = [Language::EnglishUs, Language::Japanese];

    /// Its BCP 47 tag, as `settings.ini` and `--language` take it.
    pub fn tag(self) -> &'static str {
        match self {
            Language::EnglishUs => "en-US",
            Language::Japanese => "ja",
        }
    }

    /// Its name, in itself.
    pub fn name(self) -> &'static str {
        match self {
            Language::EnglishUs => "English (US)",
            Language::Japanese => "日本語",
        }
    }

    /// A tag or a name: `en-US` (or `en`, `english`), `ja` (or `japanese`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "en-us" | "en" | "english" => Some(Language::EnglishUs),
            "ja" | "ja-jp" | "jp" | "japanese" | "日本語" => Some(Language::Japanese),
            _ => None,
        }
    }

    /// The next language in [`Language::ALL`], for a host's switch key.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&l| l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The language's own half-width font (`font.txt`), if it brings one.
    pub fn font(self) -> Option<&'static str> {
        match self {
            Language::EnglishUs => Some(include_str!("../locales/en-US/font.txt")),
            Language::Japanese => None,
        }
    }

    /// Full-width characters the game's code writes itself, redrawn for
    /// the language (`fullwidth.txt`).
    pub fn full_width(self) -> Option<&'static str> {
        match self {
            Language::EnglishUs => Some(include_str!("../locales/en-US/fullwidth.txt")),
            Language::Japanese => None,
        }
    }

    /// The language's own drawings of pictures with lettering
    /// (`graphics.txt`).
    pub fn graphics(self) -> Option<&'static str> {
        match self {
            Language::EnglishUs => Some(include_str!("../locales/en-US/graphics.txt")),
            Language::Japanese => None,
        }
    }

    /// The translation's files; none for the ROM's own Japanese.
    pub fn sources(self) -> Sources {
        match self {
            Language::EnglishUs => locale!("en-US"),
            Language::Japanese => &[],
        }
    }

    /// Translations of the texts the game keeps outside its text table
    /// (`data.txt`, keyed by `Label` or `Owner.local`).
    pub fn data(self) -> Result<Vec<text::Entry>, Error> {
        match self {
            Language::EnglishUs => {
                text::parse("en-US/data.txt", include_str!("../locales/en-US/data.txt"))
                    .map_err(Error)
            }
            Language::Japanese => Ok(Vec::new()),
        }
    }

    /// Every translated text, parsed.
    pub fn entries(self) -> Result<Vec<text::Entry>, Error> {
        let mut entries = Vec::new();
        for &(file, source) in self.sources() {
            entries.extend(text::parse(file, source).map_err(Error)?);
        }
        Ok(entries)
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// A translation that cannot be laid into the ROM (a bug in its files,
/// which the crate's tests catch first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Bytes per ROM bank.
const BANK: usize = 0x4000;

/// The size of every image [`localize`] makes: 256 banks, all that the
/// game's bank switch (`BankswitchROM` writes the MBC5's low bank register
/// only) can reach.
pub const IMAGE_SIZE: usize = 0x40_0000;

/// Where a label of the decomp's symbols lies: (bank, address).
pub type Labels<'a> = &'a dyn Fn(&str) -> Option<(u16, u16)>;

/// Where a text kept outside the text table lies: (bank, address, its
/// bytes up to the next label).
pub type TextData<'a> = &'a dyn Fn(&str) -> Option<(u16, u16, u16)>;

/// What a language needs to know of the program (the kernel has it, from
/// the decomp's symbols).
pub struct Symbols<'a> {
    /// A global label: (bank, address).
    pub label: Labels<'a>,
    /// A text the game keeps outside its text table, by label (global, or
    /// `Owner.local`).
    pub text_data: TextData<'a>,
}

/// The ROM image the game runs on in `language`: the player's `rom`
/// (already checked to be the one the kernel was translated from), every
/// language's text in the banks past it, the text table pointing at
/// `language`'s, and `language`'s font and texts kept outside the table.
pub fn localize(rom: &[u8], language: Language, symbols: &Symbols) -> Result<Vec<u8>, Error> {
    if rom.len() > IMAGE_SIZE / 2 || !rom.len().is_multiple_of(BANK) {
        return Err(Error(format!("a ROM of {} bytes", rom.len())));
    }
    let labels = symbols.label;
    let table = Table::read(rom, labels)?;
    let mut image = rom.to_vec();
    image.resize(IMAGE_SIZE, 0xff);
    let mut layout = Layout {
        at: rom.len(),
        placed: HashMap::new(),
    };
    for each in Language::ALL {
        let mut pointers = Vec::new();
        let mut seen = HashMap::new();
        for entry in each.entries()? {
            if let Some(line) = seen.insert(entry.label.clone(), entry.line) {
                return Err(Error(format!(
                    "{}:{}: [{}] is translated twice (also at line {line})",
                    entry.file, entry.line, entry.label
                )));
            }
            let where_ = || format!("{}:{}: [{}]", entry.file, entry.line, entry.label);
            let ids = table
                .ids(&entry.label, labels)
                .map_err(|e| Error(format!("{}: {e}", where_())))?;
            let bytes =
                text::encode(&entry.text).map_err(|e| Error(format!("{}: {e}", where_())))?;
            let target = layout.place(&mut image, &bytes)?;
            pointers.extend(ids.iter().map(|&id| (id, target)));
        }
        if each == language {
            for (id, target) in pointers {
                table.point(&mut image, id, target);
            }
        }
    }
    if let Some(source) = language.font() {
        let font = font::parse(source).map_err(|e| Error(format!("{language}/font.txt: {e}")))?;
        let (bank, addr) =
            labels("HalfWidthFont").ok_or_else(|| Error("no HalfWidthFont label".into()))?;
        let at = rom_offset(bank, addr);
        image[at..at + font::SIZE].copy_from_slice(&font);
    }
    if let Some(source) = language.full_width() {
        let glyphs = font::parse_full_width(source)
            .map_err(|e| Error(format!("{language}/fullwidth.txt: {e}")))?;
        let (bank, addr) =
            labels("FullWidthFonts").ok_or_else(|| Error("no FullWidthFonts label".into()))?;
        let fonts = rom_offset(bank, addr);
        for (c, glyph) in glyphs {
            let at = fonts + font::full_width_offset(c).expect("checked when parsed");
            image[at..at + 8].copy_from_slice(&glyph);
        }
    }
    if let Some(source) = language.graphics() {
        let pictures =
            graphics::parse(source).map_err(|e| Error(format!("{language}/graphics.txt: {e}")))?;
        for picture in pictures {
            let bad = || Error(format!("{language}/graphics.txt: [{}]", picture.label));
            let (name, offset) = match picture.label.split_once('+') {
                Some((name, n)) => (name, n.parse::<usize>().map_err(|_| bad())?),
                None => (picture.label.as_str(), 0),
            };
            let (bank, addr) = labels(name).ok_or_else(bad)?;
            let at = rom_offset(bank, addr) + offset;
            picture
                .draw(&mut image[at..])
                .map_err(|e| Error(format!("{language}/graphics.txt: [{}]: {e}", picture.label)))?;
        }
    }
    // Texts outside the table: each replaces the bytes of its label, which
    // are cleared first; `[Label+N]` writes from N bytes in.
    let mut cleared = HashMap::new();
    for entry in language.data()? {
        let where_ = format!("{}:{}: [{}]", entry.file, entry.line, entry.label);
        let (name, offset) = match entry.label.split_once('+') {
            Some((name, n)) => (
                name,
                n.parse::<usize>()
                    .map_err(|_| Error(format!("{where_}: bad offset")))?,
            ),
            None => (entry.label.as_str(), 0),
        };
        let (bank, addr, size) = (symbols.text_data)(name)
            .ok_or_else(|| Error(format!("{where_}: no such text outside the text table")))?;
        let bytes = text::encode(&entry.text).map_err(|e| Error(format!("{where_}: {e}")))?;
        let size = size as usize;
        if offset + bytes.len() > size {
            return Err(Error(format!(
                "{where_}: {} bytes, but the original has {}",
                offset + bytes.len(),
                size
            )));
        }
        let at = rom_offset(bank, addr);
        if cleared.insert(name.to_string(), ()).is_none() {
            image[at..at + size].fill(0);
        }
        image[at + offset..at + offset + bytes.len()].copy_from_slice(&bytes);
    }
    Ok(image)
}

/// The text table (`TextOffsets`): entry N, 4 bytes at N * 4 from the start
/// of the table's bank, holds the text's offset from that same point (3
/// bytes, little-endian; the 4th is unused).
struct Table {
    /// The ROM offset of the table's bank, where entries and offsets count
    /// from.
    origin: usize,
    /// Text numbers by the ROM offset of the text they print.
    ids: HashMap<usize, Vec<usize>>,
}

fn rom_offset(bank: u16, addr: u16) -> usize {
    match addr {
        0x0000..=0x3fff => addr as usize,
        _ => bank as usize * BANK + (addr as usize - 0x4000),
    }
}

impl Table {
    fn read(rom: &[u8], labels: Labels) -> Result<Self, Error> {
        let (bank, addr) =
            labels("TextOffsets").ok_or_else(|| Error("no TextOffsets label".into()))?;
        // GetTextOffsetFromTextID counts entries from the start of the
        // table's bank.
        if addr != 0x4000 {
            return Err(Error(format!(
                "TextOffsets is at {bank:02x}:{addr:04x}, not at the start of its bank"
            )));
        }
        let origin = rom_offset(bank, addr);
        // The entries end where the first text begins. Entry 0 is no text.
        let mut end = rom.len();
        let mut ids: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut id = 1;
        while origin + 4 * id + 4 <= end {
            let e = origin + 4 * id;
            let offset = rom[e] as usize | (rom[e + 1] as usize) << 8 | (rom[e + 2] as usize) << 16;
            if offset != 0 {
                let target = origin + offset;
                end = end.min(target);
                ids.entry(target).or_default().push(id);
            }
            id += 1;
        }
        Ok(Self { origin, ids })
    }

    /// The numbers of the text a label marks.
    fn ids(&self, label: &str, labels: Labels) -> Result<&[usize], String> {
        let (bank, addr) = labels(label).ok_or("no such label in the decomp")?;
        self.ids
            .get(&rom_offset(bank, addr))
            .map(Vec::as_slice)
            .ok_or_else(|| "this label is no text of the text table".into())
    }

    fn point(&self, image: &mut [u8], id: usize, target: usize) {
        let offset = target - self.origin;
        let e = self.origin + 4 * id;
        image[e..e + 3].copy_from_slice(&[offset as u8, (offset >> 8) as u8, (offset >> 16) as u8]);
    }
}

/// Lays encoded texts one after another from bank $80, never across a
/// bank's end (the game reads a text within one mapped bank), sharing the
/// bytes of identical texts.
struct Layout {
    at: usize,
    placed: HashMap<Vec<u8>, usize>,
}

impl Layout {
    fn place(&mut self, image: &mut [u8], bytes: &[u8]) -> Result<usize, Error> {
        if let Some(&at) = self.placed.get(bytes) {
            return Ok(at);
        }
        let bank_end = (self.at / BANK + 1) * BANK;
        if self.at + bytes.len() > bank_end {
            self.at = bank_end;
        }
        if bytes.len() > BANK || self.at + bytes.len() > IMAGE_SIZE {
            return Err(Error("the translations do not fit in the ROM image".into()));
        }
        let at = self.at;
        image[at..at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
        self.placed.insert(bytes.to_vec(), at);
        Ok(at)
    }
}

#[cfg(test)]
mod tests;
