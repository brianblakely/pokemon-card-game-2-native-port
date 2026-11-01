use super::*;
use crate::text::TX_HALFWIDTH;
use std::path::PathBuf;

/// A text of the decomp's `src/text/text*.asm`: its label, file and lines
/// (as written in the source, tokens included).
struct Original {
    label: String,
    file: String,
    lines: Vec<String>,
}

fn decomp_texts() -> Vec<Original> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../decomp/src/text");
    let mut texts = Vec::new();
    for n in 1..=17 {
        let file = format!("text{n}.asm");
        let source = std::fs::read_to_string(dir.join(&file))
            .unwrap_or_else(|e| panic!("{}: {e}", dir.join(&file).display()));
        for line in source.lines() {
            // `Label:`, perhaps followed by a comment.
            if let Some((label, _)) = line.split_once(':')
                && !label.is_empty()
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                texts.push(Original {
                    label: label.to_string(),
                    file: format!("text{n}.txt"),
                    lines: Vec::new(),
                });
                continue;
            }
            let code = line.trim();
            let Some(text) = texts.last_mut() else {
                continue;
            };
            for (macro_, new_line) in [("textfw ", false), ("text ", false), ("linefw ", true)] {
                if let Some(arg) = code.strip_prefix(macro_) {
                    let arg = arg.split(';').next().unwrap().trim();
                    let quoted = arg.trim_matches('"').to_string();
                    if new_line || text.lines.is_empty() {
                        text.lines.push(quoted);
                    } else {
                        text.lines.last_mut().unwrap().push_str(&quoted);
                    }
                }
            }
        }
    }
    texts
}

/// A Japanese line's width in tiles: a character or symbol each, what the
/// game fills in at the widths `text::line_widths` assumes.
fn japanese_width(line: &str) -> usize {
    let mut width = 0;
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
        {
            width += match &rest[1..end] {
                "RAMNAME" => 6,
                "RAMTEXT" => 8,
                "RAMNUM" => 2,
                _ => 1,
            };
            rest = &rest[end + 1..];
            continue;
        }
        width += 1;
        rest = &rest[c.len_utf8()..];
    }
    width
}

/// No English line is wider than the widest text box (18 tiles), unless
/// the Japanese text was too, and no wider than that.
#[test]
fn translations_fit_their_boxes() {
    let originals = decomp_texts();
    let by_label: HashMap<&str, &Original> =
        originals.iter().map(|o| (o.label.as_str(), o)).collect();
    let mut problems = Vec::new();
    for language in Language::ALL {
        for entry in language.entries().unwrap() {
            let Some(original) = by_label.get(entry.label.as_str()) else {
                continue;
            };
            let japanese = original
                .lines
                .iter()
                .map(|l| japanese_width(l))
                .max()
                .unwrap_or(0);
            let limit = 36.max(2 * japanese);
            for (n, width) in text::line_widths(&entry.text)
                .unwrap()
                .into_iter()
                .enumerate()
            {
                if width > limit {
                    problems.push(format!(
                        "{}:{}: [{}] line {}: {width} half tiles (at most {limit})",
                        entry.file,
                        entry.line,
                        entry.label,
                        n + 1
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Texts every language keeps as the ROM has them: the naming screen's
/// keyboards, whose keys type what `KeyboardData` says (kana, full-width
/// letters) whatever their labels read.
const KEPT: [&str; 4] = [
    "HiraganaKeyboardText",
    "KatakanaKeyboardText",
    "UppercaseKeyboardText",
    "LowercaseKeyboardText",
];

/// Kana and kanji: a text holding any needs translating.
fn is_japanese(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}')
        || "「」・ー々".contains(c)
}

#[test]
fn every_translation_encodes() {
    for language in Language::ALL {
        let entries = language.entries().unwrap();
        for entry in &entries {
            let bytes = text::encode(&entry.text).unwrap_or_else(|e| {
                panic!("{}:{}: [{}]: {e}", entry.file, entry.line, entry.label)
            });
            assert!(bytes.len() <= 0x4000);
        }
    }
}

#[test]
fn translations_match_the_decomp() {
    let originals = decomp_texts();
    let by_label: HashMap<&str, &Original> =
        originals.iter().map(|o| (o.label.as_str(), o)).collect();
    for language in Language::ALL {
        if language.sources().is_empty() {
            continue;
        }
        let entries = language.entries().unwrap();
        let mut seen = HashMap::new();
        let mut problems = Vec::new();
        for entry in &entries {
            let at = format!("{}:{}: [{}]", entry.file, entry.line, entry.label);
            if let Some(line) = seen.insert(entry.label.as_str(), entry.line) {
                problems.push(format!("{at}: also at line {line}"));
            }
            match by_label.get(entry.label.as_str()) {
                None => problems.push(format!("{at}: no such text in src/decomp/src/text")),
                Some(original) if !entry.file.ends_with(&format!("/{}", original.file)) => {
                    problems.push(format!("{at}: belongs in {}", original.file))
                }
                _ => {}
            }
        }
        let missing: Vec<&str> = originals
            .iter()
            .filter(|o| o.lines.iter().any(|l| l.chars().any(is_japanese)))
            .filter(|o| !seen.contains_key(o.label.as_str()) && !KEPT.contains(&o.label.as_str()))
            .map(|o| o.label.as_str())
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "{language}: {} Japanese texts are not translated, starting with {:?}",
                missing.len(),
                &missing[..missing.len().min(8)]
            ));
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}

#[test]
fn every_font_parses() {
    for language in Language::ALL {
        if let Some(source) = language.font() {
            let font = font::parse(source).unwrap();
            // A space is blank; 'A' is not.
            assert!(font[..8].iter().all(|&b| b == 0));
            assert!(
                font[(b'A' - 0x20) as usize * 8..][..8]
                    .iter()
                    .any(|&b| b != 0)
            );
            // Nothing in the gap column or the right half.
            assert!(font.iter().all(|&b| b & 0x1f == 0));
            // The game's padding is blank; 'p' is where the encoder puts it.
            let glyph = |code: u8| &font[(code - 0x20) as usize * 8..][..8];
            assert!(glyph(text::PADDING).iter().all(|&b| b == 0));
            assert!(glyph(text::LETTER_P).iter().any(|&b| b != 0));
        } else {
            assert!(
                language.sources().is_empty(),
                "{language} encodes text but has no font"
            );
        }
    }
    assert!(font::parse("20  \n....\n").is_err());
    for language in Language::ALL {
        if let Some(source) = language.full_width() {
            assert!(!font::parse_full_width(source).unwrap().is_empty());
        }
    }
    assert!(font::parse_full_width("あ\n").is_err());
}

/// Each drawing is drawn over a picture the decomp INCBINs at its label: one
/// of the same size, or, laid out tile by tile, within it.
#[test]
fn every_picture_fits_its_original() {
    let decomp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../decomp/src");
    let gfx = std::fs::read_to_string(decomp.join("gfx.asm")).unwrap();
    for language in Language::ALL {
        let Some(source) = language.graphics() else {
            continue;
        };
        for picture in graphics::parse(source).unwrap() {
            let (name, offset) = match picture.label.split_once('+') {
                Some((name, n)) => (name, n.parse::<usize>().unwrap()),
                None => (picture.label.as_str(), 0),
            };
            // The label's data in order (INCBINs, and words such as a
            // length before one), up to the INCBIN at the offset.
            let mut lines = gfx
                .lines()
                .skip_while(|l| !l.starts_with(&format!("{name}:")));
            lines
                .next()
                .unwrap_or_else(|| panic!("no {name} in gfx.asm"));
            let mut at = 0;
            let mut found = None;
            for line in lines.take_while(|l| !l.contains("::")) {
                let code = line.split(';').next().unwrap().trim();
                if let Some(words) = code.strip_prefix("dw ") {
                    at += 2 * words.split(',').count();
                    continue;
                }
                let Some(file) = code
                    .strip_prefix("INCBIN \"")
                    .and_then(|l| l.strip_suffix(".2bpp\""))
                else {
                    continue;
                };
                let png = std::fs::read(decomp.join(format!("{file}.png"))).unwrap();
                let size =
                    |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
                let (width, height) = (size(16), size(20));
                if at == offset {
                    found = Some((file.to_string(), width, height));
                    break;
                }
                at += width * height / 4;
            }
            let (file, width, height) =
                found.unwrap_or_else(|| panic!("{language}: no picture at [{}]", picture.label));
            if picture.tiles.iter().enumerate().all(|(i, &t)| t == Some(i)) {
                assert_eq!(
                    (picture.width, picture.height),
                    (width, height),
                    "{language}: [{}] is {file}.png's size",
                    picture.label
                );
            }
            for &tile in picture.tiles.iter().flatten() {
                assert!(
                    tile < width * height / 64,
                    "{language}: [{}] draws tile {tile:02x}, past {file}.png",
                    picture.label
                );
            }
        }
    }
}

#[test]
fn pictures() {
    // Two blocks drawn into tiles 1 and 0, keeping one pixel.
    let source = String::from("[Pic] 16x8\ntiles 01 00\n?:::::::########\n")
        + &"::::::::########\n".repeat(6)
        + "::::::::.......x\n";
    let mut rom = [0xaa; 48];
    graphics::parse(&source).unwrap()[0].draw(&mut rom).unwrap();
    let mut expected = [0xaa; 48]; // tile 2 untouched
    expected[..14].fill(0xff); // tile 0: black
    expected[14..16].copy_from_slice(&[0x00, 0x01]); // then white and a 2
    expected[16..18].copy_from_slice(&[0xff, 0x80]); // tile 1: the kept 3, then 1s
    for row in expected[18..32].chunks_exact_mut(2) {
        row.copy_from_slice(&[0xff, 0x00]);
    }
    assert_eq!(rom, expected);

    let rows = |row: &str| format!("{row}\n").repeat(8);
    let parses = |source: String| graphics::parse(&source).is_ok();
    assert!(!parses(format!(
        "[Pic] 8x8\ntiles 00 01\n{}",
        rows("........")
    )));
    assert!(!parses(format!(
        "[Pic] 8x8\ntiles --\n{}",
        rows("#???????")
    )));
    assert!(!parses(format!(
        "[Pic] 16x8\ntiles 00 00\n{}",
        rows("#.......?.......")
    )));
    assert!(!parses(format!(
        "[Pic] 8x8\n{}[Pic] 8x8\n{}",
        rows("#......."),
        rows("........")
    )));
    assert!(parses(format!(
        "[Pic] 16x8\ntiles 00 00\n{}",
        rows("#.......#.......")
    )));
    assert!(parses(format!(
        "[Pic] 16x8\ntiles 00 --\n{}",
        rows("#.......????????")
    )));
}

#[test]
fn encoding() {
    assert_eq!(text::encode("Hi").unwrap(), [TX_HALFWIDTH, b'H', b'i', 0]);
    assert_eq!(
        text::encode("<RAMNAME> é\n<FIRE>“p”").unwrap(),
        [6, 0x09, b' ', 0x60, 0x0a, 0x05, 0x01, 0x22, 0x3c, 0x22, 0]
    );
    assert!(text::encode("100%").is_err());
    assert!(text::encode("<Nope>").is_err());
    assert_eq!(text::line_widths("abc<FIRE>d\nxy<RAMNUM>").unwrap(), [7, 5]);
}

#[test]
fn parsing() {
    let entries = text::parse(
        "t.txt",
        "# comment\n\n[A]\nOne  \nTwo\n\n\n[B]\n` padded `\n# inside\n\n[C]\n",
    )
    .unwrap();
    let got: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| (e.label.as_str(), e.text.as_str()))
        .collect();
    assert_eq!(got, [("A", "One\nTwo"), ("B", " padded "), ("C", "")]);
    let entries = text::parse("t.txt", "[A]\n[Basic Energy Card]\nText\n").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "[Basic Energy Card]\nText");
    assert!(text::parse("t.txt", "stray\n[A]\n").is_err());
    assert!(text::parse("t.txt", "[A]\n`half\n").is_err());
}

/// A ROM with the table in bank $37 and two texts in bank $38, the second
/// one numbered twice.
#[test]
fn localize_points_the_table_at_the_translation() {
    let mut rom = vec![0u8; 0x20_0000];
    let origin = 0x37 * 0x4000;
    let first = 0x38 * 0x4000;
    for (id, target) in [(1, first), (2, first + 3), (3, first + 3)] {
        let offset = target - origin;
        rom[origin + 4 * id..origin + 4 * id + 3].copy_from_slice(&[
            offset as u8,
            (offset >> 8) as u8,
            (offset >> 16) as u8,
        ]);
    }
    let labels = |name: &str| match name {
        "TextOffsets" => Some((0x37, 0x4000)),
        "HandText" => Some((0x38, 0x4000)),
        "CheckText" => Some((0x38, 0x4003)),
        _ => Some((0x38, 0x7000)),
    };
    let table = Table::read(&rom, &labels).unwrap();
    assert_eq!(table.ids("CheckText", &labels).unwrap(), [2, 3]);
    let symbols = Symbols {
        label: &labels,
        text_data: &|_| None,
    };
    let japanese = localize(&rom, Language::Japanese, &symbols);
    // The real translation names labels this ROM does not have.
    assert!(japanese.is_err());

    let mut image = rom.clone();
    image.resize(IMAGE_SIZE, 0xff);
    let mut layout = Layout {
        at: rom.len(),
        placed: HashMap::new(),
    };
    let bytes = text::encode("Check").unwrap();
    let at = layout.place(&mut image, &bytes).unwrap();
    assert_eq!(at, 0x80 * 0x4000);
    for &id in table.ids("CheckText", &labels).unwrap() {
        table.point(&mut image, id, at);
    }
    for id in [2, 3] {
        let e = origin + 4 * id;
        let offset =
            image[e] as usize | (image[e + 1] as usize) << 8 | (image[e + 2] as usize) << 16;
        // As GetTextOffsetFromTextID decodes it.
        let bank = 0x37 + (offset >> 14);
        let addr = 0x4000 | (offset & 0x3fff);
        assert_eq!((bank, addr), (0x80, 0x4000));
        assert_eq!(&image[at..at + bytes.len()], bytes.as_slice());
    }
    // Text 1 still prints the ROM's text.
    assert_eq!(&image[origin + 4..origin + 7], &rom[origin + 4..origin + 7]);
    // A text never runs past its bank's end.
    layout.at = 0x81 * 0x4000 - 2;
    assert_eq!(layout.place(&mut image, &[1, 2, 3]).unwrap(), 0x81 * 0x4000);
}
