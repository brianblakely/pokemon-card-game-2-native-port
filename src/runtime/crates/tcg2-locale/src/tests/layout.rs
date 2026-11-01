//! Where the game prints each text of the text table, as the decomp shows
//! it, and so where a translation fits full-width.
//!
//! A full-width text needs a tile a letter, twice the room of half-width
//! text, so it is used only where the room is known:
//!
//! - The game's text boxes. `PrintScrollableText` (the overworld's
//!   dialogue) prints 18 tiles a line, two lines a page, and the player
//!   turns the pages. The wide box at the bottom (`DrawWideTextBox_*`) holds
//!   two lines of 18, one if a yes/no or two-item menu takes the second
//!   line; the narrow box, lines of 10.
//! - Menu boxes (`menubox_params` and its `textitem`s): an item runs from
//!   its position to the box's border or the next item's cursor. A menu's
//!   items, and its label, are full-width only if all of them fit, so a
//!   menu reads in one size.
//! - The duel's other ways to a box: a coin toss's text (in de), a card
//!   list's prompt and header, and the reason a check hands back with
//!   carry, which the duel prints in the wide box ([`returners`]).
//! - Tables of data: a card's text and Pokédex entry, letters, the
//!   glossary, the practice duel's steps, the special rules
//!   ([`DataTables`]).
//! - A few texts and tables printed at fixed places, measured by hand from
//!   the code that prints them ([`MEASURED`], [`TABLES`], [`TABLE_TEXTS`]).
//!
//! A text the decomp prints anywhere else (a card's name, a screen's
//! title, a text the code computes or stores in a table) stays half-width:
//! its room and what is printed next to it are not known here. One may be
//! made full-width by hand if it fits where the Japanese text stood
//! (no line wider than the Japanese's widest, no more lines). The game's
//! debug menus and routines (`Debug*`), which no player sees, do not count.
//!
//! A text that puts another into itself (`<RAMTEXT>`: a card's, a deck's or
//! a person's name) stays half-width too, unless what it puts in is known
//! and full-width ([`PUT_INTO`]). What it puts in is such a text,
//! half-width, and a line reads in one size. (The other way round would
//! also go wrong: the engine pairs a half-width text's odd last letter with
//! what follows, so a full-width text put into a half-width one would
//! leave that letter after it.)

use super::*;
use std::collections::{BTreeMap, HashSet};

/// Tiles a line in the game's wide text boxes (`DrawWideTextBox`,
/// `PrintScrollableText`: from x = 1, inside the border at x = 19).
const WIDE: usize = 18;
/// The narrow box (`DrawNarrowTextBox`: 12 wide).
const NARROW: usize = 10;

/// Another text put into this one: always half-width.
pub const RAMTEXT: &str = "<RAMTEXT>";

/// How the game prints a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// `PrintScrollableText`: lines of 18, two to a page; with a yes/no
    /// menu over the last page's second line if `then_yes_no`.
    Pages { then_yes_no: bool },
    /// A box printed at once: lines of `width` tiles, at most `lines`.
    Box { width: usize, lines: usize },
    /// An item (or the label) of the menu box numbered `menu`: one line of
    /// `width` tiles.
    Item { width: usize, menu: usize },
    /// Anywhere else.
    Unknown,
}

/// The routines that print the text (id) in `hl` in a known box.
fn routine_place(name: &str) -> Option<Place> {
    let pages = Place::Pages { then_yes_no: false };
    let wide = |lines| Place::Box { width: WIDE, lines };
    let narrow = |lines| Place::Box {
        width: NARROW,
        lines,
    };
    Some(match name {
        "PrintScrollableText_NoTextBoxLabel"
        | "PrintScrollableText_WithTextBoxLabel"
        | "PrintScrollableText_WithTextBoxLabel_NoWait"
        | "PrintScrollableText_NoTextBoxLabelVRAM0"
        | "PrintScrollableText_WithTextBoxLabelVRAM0"
        | "PrintTextInLabelledScrollableTextBox" => pages,
        "PrintScrollableText_WithTextBoxLabelWithYesOrNoMenu" => Place::Pages { then_yes_no: true },
        "DrawWideTextBox_PrintText"
        | "DrawWideTextBox_WaitForInput"
        | "DrawWideTextBox_WaitForInput_ReturnCarry"
        | "DrawWideTextBox_PrintTextNoDelay"
        | "DrawWideTextBox_PrintTextNoDelay_Wait"
        | "PrintTextInWideTextBox" => wide(2),
        "DrawWideTextBox_PrintTextWithYesOrNoMenu"
        | "YesOrNoMenuWithText"
        | "YesOrNoMenuWithText_SetCursorToYes"
        | "TwoItemHorizontalMenu" => wide(1),
        "DrawNarrowTextBox_PrintTextNoDelay" | "DrawNarrowTextBox_WaitForInput" => narrow(2),
        // A card list's prompt, which `DisplayCardList` prints in the narrow
        // box under the list.
        "SetCardListInfoBoxText" | "SetCardListHeaderAndInfoText" | "SelectCardSearchTarget" => {
            narrow(2)
        }
        "YesOrNoMenuWithText_LeftAligned" => narrow(1),
        _ => return None,
    })
}

/// Tables of texts the code places at fixed positions (`PlaceTextItems`),
/// with the column each row's room ends at, read from the code: (table,
/// column). An item runs from its position to that column, or to the next
/// item's cursor; a table's texts are full-width only all together.
const TABLES: [(&str, usize); 5] = [
    // The duel's menu, in the wide box at the bottom (border at x = 19).
    ("DuelMenuData", 19),
    // A Pokémon card's page: retreat cost, weakness and resistance symbols
    // from x = 5 (`DisplayCardPage_PokemonOverview`).
    ("CardPageRetreatWRTextData", 5),
    // The same, printed (`RetreatWeakResistData`, rows 70-72).
    ("RetreatWeakResistData", 5),
    // The Shift screen: type, weakness and resistance symbols at x = 14.
    ("ShiftMenuData", 14),
    // The status screen's coin box: its title on the top border from x = 1,
    // the current coin from x = 4, up to the right border at x = 19.
    ("DrawStatusScreenBottomBox", 19),
];

/// Texts the code puts into exactly one other text, as its `<RAMTEXT>`:
/// (that text, the texts). That text may be full-width when all of them
/// are.
pub const PUT_INTO: [(&str, &[&str]); 2] = [
    (
        "GiftCenterServiceAcknowledgementText",
        &[
            "GiftCenterSendCardsText",
            "GiftCenterReceiveCardsText",
            "GiftCenterSendDeckConfigurationText",
            "GiftCenterReceiveDeckConfigurationText",
        ],
    ),
    (
        "ObtainedCoinText",
        &[
            "ChanseyCoinText",
            "GRCoinText",
            "GrassCoinText",
            "FireCoinText",
            "WaterCoinText",
            "LightningCoinText",
            "PsychicCoinText",
            "RockCoinText",
            "GRGrassCoinText",
            "GRLightningCoinText",
            "GRFireCoinText",
            "GRWaterCoinText",
            "GRFightingCoinText",
            "GRPsychicCoinText",
            "GRColorlessCoinText",
            "GRKingCoinText",
            "PonytaCoinText",
            "HorseaCoinText",
            "ArbokCoinText",
            "JigglypuffCoinText",
            "DugtrioCoinText",
            "GengarCoinText",
            "RaichuCoinText",
            "LugiaCoinText",
        ],
    ),
];

/// Tables of texts the code reads by number, by the routine they are in,
/// and the place where it prints each of their texts: none for texts it
/// only puts into another ([`PUT_INTO`]).
const TABLE_TEXTS: [(&str, Option<Place>); 3] = [
    // The Gift Center's items, put into its acknowledgement.
    ("GiftCenter_HandleMenu", None),
    // Coin names: on the status screen at (4, 12), up to the border, and on
    // the Coins screen in the 12 tiles cleared at (4, 6). They are also put
    // into `ObtainedCoinText`.
    (
        "GetCoinName",
        Some(Place::Box {
            width: 12,
            lines: 1,
        }),
    ),
    // The Coins screen's page title, at (6, 8) between the page arrows at
    // x = 0 and 19.
    (
        "CoinMenuPageTitles",
        Some(Place::Box {
            width: 13,
            lines: 1,
        }),
    ),
];

/// References to a text's number that print nothing: (routine, text).
/// `ButterfreeMegaDrainEffect` adds 5 to round, written as `DoneText`.
const NOT_PRINTED: [(&str, &str); 1] = [("ButterfreeMegaDrainEffect", "DoneText")];

/// Texts printed where the scan cannot measure the room, measured by hand
/// from the routine that prints them: (routine, text, place).
const MEASURED: [(&str, &str, Place); 9] = [
    // The hero's gender, at (5, 8) in the 20×12 box, under the portrait
    // (from x = 12 on).
    (
        "ConfirmPlayerNameAndGender",
        "PlayerGenderMaleText",
        Place::Box { width: 7, lines: 1 },
    ),
    (
        "ConfirmPlayerNameAndGender",
        "PlayerGenderFemaleText",
        Place::Box { width: 7, lines: 1 },
    ),
    // The PC screen's title, at (7, 2) in the 20×4 box at the top.
    (
        "DrawMinicomMainScreen",
        "PauseMenuMinicomText",
        Place::Box {
            width: 12,
            lines: 1,
        },
    ),
    // The status screen's coin box: Team GR's coin, at (4, 14).
    (
        "DrawStatusScreenBottomBox",
        "PlayerStatusGRCoinText",
        Place::Box {
            width: 15,
            lines: 1,
        },
    ),
    // The Coins screen: "Current Coin" at (4, 4) in the 20×8 box.
    (
        "ShowSelectedCoinOnCoinMenu",
        "PlayerStatusCurrentCoinText",
        Place::Box {
            width: 15,
            lines: 1,
        },
    ),
    // The Coins screen's prompt, at (1, 2) in the 20×8 box: the coin and its
    // name take the lines below.
    (
        "CoinMenu",
        "PlayerCoinSelectText",
        Place::Box {
            width: 18,
            lines: 1,
        },
    ),
    // The Coins screen after a new coin: the count, where the prompt
    // stands.
    (
        "ShowReceivedCoinOnCoinMenu",
        "ObtainedCoinTotalNumberText",
        Place::Box {
            width: 18,
            lines: 1,
        },
    ),
    (
        "ShowReceivedCoinOnCoinMenu",
        "ObtainedGRCoinPieceTotalNumberText",
        Place::Box {
            width: 18,
            lines: 1,
        },
    ),
    // The settings screen's exit, at (2, 17) under the last box, alone on
    // its row.
    (
        "DrawConfigMenu",
        "ConfigExitText",
        Place::Box {
            width: 18,
            lines: 1,
        },
    ),
];

/// The overworld script commands that print a text (`macros/scripts.asm`),
/// through the routines above.
fn script_place(command: &str) -> Option<Place> {
    Some(match command {
        "print_text"
        | "print_npc_text"
        | "print_variable_text"
        | "print_variable_npc_text"
        | "print_npc_text_instant" => Place::Pages { then_yes_no: false },
        "npc_ask_question" => Place::Pages { then_yes_no: true },
        "ask_question" => Place::Box {
            width: WIDE,
            lines: 1,
        },
        "print_text_wide_textbox" => Place::Box {
            width: WIDE,
            lines: 2,
        },
        _ => return None,
    })
}

/// A source line without its comment, split into words (commas split too).
fn words(line: &str) -> Vec<&str> {
    line.split(';')
        .next()
        .unwrap()
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .collect()
}

/// Every place each text label is printed, read from the decomp's code and
/// data (everything but `text/`).
pub fn places(texts: &HashSet<String>) -> HashMap<String, Vec<Place>> {
    let sources = sources();
    let returners = returners(&sources);
    let mut places: HashMap<String, Vec<Place>> = HashMap::new();
    let mut menus = 0;
    for (file, lines) in &sources {
        let paged = paged_lists(lines);
        let mut tables = DataTables::default();
        let mut routine = "";
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            if !line.starts_with(['.', ' ', '\t'])
                && let Some((label, _)) = line.split_once(':')
                && !label.is_empty()
                && !label.contains(char::is_whitespace)
            {
                routine = label;
            }
            let w = words(line);
            let Some(&op) = w.first() else {
                i += 1;
                continue;
            };
            // The game's debug menus and routines: no player sees them.
            if routine.starts_with("Debug") {
                i += 1;
                continue;
            }
            // Tables of data whose texts' places are known.
            if let Some(found) = tables.places(file, routine, &w) {
                for (text, place) in found {
                    if texts.contains(text) {
                        places.entry(text.to_string()).or_default().push(place);
                    }
                }
                i += 1;
                continue;
            }
            // A menu box and its items.
            if op == "menubox_params" && w.len() >= 14 {
                let box_drawn = w[1] != "FALSE";
                // The label is the last argument (one may be an expression).
                let (width, label) = (w[2].parse::<usize>().unwrap(), *w.last().unwrap());
                let menu = menus;
                menus += 1;
                if label.starts_with("Debug") {
                    // A debug menu: skip it and its items.
                    i += 1;
                    while i < lines.len() && words(&lines[i]).first() == Some(&"textitem") {
                        i += 1;
                    }
                    continue;
                }
                if box_drawn && texts.contains(label) {
                    // On the top border, after a corner and a blank, before a
                    // blank, a border tile and a corner (DrawLabeledTextBox).
                    places
                        .entry(label.to_string())
                        .or_default()
                        .push(Place::Item {
                            width: width - 5,
                            menu,
                        });
                }
                let mut items = Vec::new();
                i += 1;
                while i < lines.len() {
                    let w = words(&lines[i]);
                    match w.first() {
                        Some(&"textitem") => {
                            // A position given by a constant: no room known,
                            // so the menu stays half-width.
                            let (x, y) = (w[1].parse::<usize>(), w[2].parse::<usize>());
                            let (x, y) = (x.unwrap_or(usize::MAX), y.unwrap_or(usize::MAX));
                            items.push((x, y, w[3].to_string()));
                            i += 1;
                        }
                        Some(&"textitems_end") | None => break,
                        _ => break,
                    }
                }
                for (x, y, label) in &items {
                    // To the border, or to the cursor of the next item on
                    // the row (a tile before it).
                    let end = items
                        .iter()
                        .filter(|(nx, ny, _)| ny == y && nx > x)
                        .map(|(nx, _, _)| nx - 1)
                        .min()
                        // A menu that draws no box of its own runs to its
                        // end.
                        .unwrap_or(if box_drawn { width - 1 } else { width });
                    if texts.contains(label) {
                        places.entry(label.clone()).or_default().push(Place::Item {
                            width: end.saturating_sub(*x),
                            menu,
                        });
                    }
                }
                continue;
            }
            // A table of texts at fixed places whose room is known.
            if op == "textitem"
                && let Some(&(_, edge)) = TABLES.iter().find(|(t, _)| *t == routine)
            {
                let menu = menus;
                menus += 1;
                let mut items = Vec::new();
                while i < lines.len() {
                    let w = words(&lines[i]);
                    if w.first() != Some(&"textitem") {
                        break;
                    }
                    let (x, y) = (
                        w[1].parse::<usize>().unwrap(),
                        w[2].parse::<usize>().unwrap(),
                    );
                    items.push((x, y, w[3].to_string()));
                    i += 1;
                }
                for (x, y, label) in &items {
                    let end = items
                        .iter()
                        .filter(|(nx, ny, _)| ny == y && nx > x)
                        .map(|(nx, _, _)| nx - 1)
                        .chain([edge])
                        .min()
                        .unwrap();
                    places.entry(label.clone()).or_default().push(Place::Item {
                        width: end - x,
                        menu,
                    });
                }
                continue;
            }
            // Script commands.
            if let Some(place) = script_place(op) {
                for arg in &w[1..] {
                    if texts.contains(*arg) {
                        places
                            .entry(arg.to_string())
                            .or_default()
                            .push(place.clone());
                    }
                }
                i += 1;
                continue;
            }
            let refs: Vec<&str> = w[1..]
                .iter()
                .map(|a| a.strip_suffix('_').unwrap_or(a))
                .filter(|a| texts.contains(*a))
                .filter(|a| !NOT_PRINTED.contains(&(routine, *a)))
                .collect();
            if refs.is_empty() {
                i += 1;
                continue;
            }
            // A table the code reads by number.
            if op == "tx"
                && let Some((_, place)) = TABLE_TEXTS.iter().find(|(r, _)| *r == routine)
            {
                if let Some(place) = place {
                    for r in &refs {
                        places.entry(r.to_string()).or_default().push(place.clone());
                    }
                }
                i += 1;
                continue;
            }
            // `ldtx hl, Text`: where the code goes from there with the text.
            let measured = MEASURED
                .iter()
                .find(|(r, t, _)| *r == routine && refs.contains(t))
                .map(|(_, _, place)| place.clone());
            let reached = if let Some(place) = measured {
                vec![place]
            } else if op == "ldtx" && w.get(1) == Some(&"hl") {
                let followed = follow(lines, i + 1);
                let mut reached = followed.places;
                if followed.returned {
                    reached.push(if returners.contains(routine) {
                        Place::Box {
                            width: WIDE,
                            lines: 2,
                        }
                    } else {
                        Place::Unknown
                    });
                }
                reached
            } else if op == "ldtx" && w.get(1) == Some(&"de") {
                follow_de(lines, i + 1)
            } else if paged.contains(&i) {
                vec![Place::Pages { then_yes_no: false }]
            } else {
                vec![Place::Unknown]
            };
            for r in refs {
                places
                    .entry(r.to_string())
                    .or_default()
                    .extend(reached.iter().cloned());
            }
            i += 1;
        }
    }
    places
}

/// What a card's text field is (`data/cards*.asm`).
#[derive(Clone, Copy, PartialEq)]
enum CardField {
    /// An attack's name, text and its continuation, after its `energy`.
    Attack,
    /// A Trainer's or an Energy's rules and their continuation, after the
    /// card's effect commands.
    Rules,
    /// A Pokémon's Pokédex entry, after its `weight`.
    Pokedex,
}

/// Tables of data the scan reads row by row, and where the game prints
/// their texts:
/// - A card's text on its page (`PrintAttackOrCardDescription`: from
///   (1, 11), lines of 18 up to the frame, moved up a line when there are
///   7) and on the screens for a used attack or Trainer (the same routine,
///   above the wide box). Its Pokédex entry: three lines of 18 from
///   (1, 14). Names are not known here: they are put into other texts.
/// - A letter (`data/mail.asm`): its sender and subject after "From:" and
///   "Subject:" in the mailbox's list (12 tiles from x = 7), its pages in
///   the reading screen's body (`PrintMailBodyPage`: six lines of 18,
///   double-spaced from (1, 6)).
/// - The glossary's topics (`explanation`): the title at (x, 2) over the
///   box, up to the screen's edge, the explanation single-spaced from
///   (1, 5) in the 20×14 box (twelve lines of 18).
/// - The practice duel's steps (`practicetext`): Dr. Mason's text in the
///   dialogue box, and the step at (1, y), three rows from the next step
///   (two lines of 18).
/// - A special rule (`ShowSpecialRuleDescription`): its title at (3, 4) in
///   the 20×6 box (16 tiles), its description double-spaced from (1, 8) in
///   the 20×12 box (five lines of 18).
#[derive(Default)]
struct DataTables {
    card: Option<(CardField, usize)>,
}

impl DataTables {
    /// The texts of a table's row `w` and their places, if it is one.
    fn places<'a>(
        &mut self,
        file: &str,
        routine: &str,
        w: &[&'a str],
    ) -> Option<Vec<(&'a str, Place)>> {
        let lines = |width, lines| Place::Box { width, lines };
        let op = *w.first()?;
        match file {
            "data/cards1.asm" | "data/cards2.asm" => {
                let field = self.card.take();
                match op {
                    "energy" => self.card = Some((CardField::Attack, 0)),
                    "weight" => self.card = Some((CardField::Pokedex, 0)),
                    "dw" if w.get(1).is_some_and(|c| c.ends_with("EffectCommands")) => {
                        self.card = Some((CardField::Rules, 0));
                    }
                    "tx" | "dw" => {
                        let (field, n) = field?;
                        // The block's fields in order: an attack's name, text
                        // and its continuation; rules and their
                        // continuation; a Pokédex entry.
                        let card = lines(WIDE, 7);
                        let fields = match field {
                            CardField::Attack => vec![None, Some(card.clone()), Some(card)],
                            CardField::Rules => vec![Some(card.clone()), Some(card)],
                            CardField::Pokedex => vec![Some(lines(WIDE, 3))],
                        };
                        if n + 1 < fields.len() {
                            self.card = Some((field, n + 1));
                        }
                        // A name: not known here.
                        let place = fields[n].clone()?;
                        return Some(if op == "tx" {
                            vec![(w[1], place)]
                        } else {
                            Vec::new()
                        });
                    }
                    _ => {}
                }
                None
            }
            "data/mail.asm" => match op {
                "tx" => Some(vec![(w[1], lines(12, 1))]),
                "mail" => Some(vec![(w[1], lines(WIDE, 6))]),
                _ => None,
            },
            "engine/glossary.asm" if op == "explanation" => {
                let x: usize = w[1].parse().ok()?;
                Some(vec![(w[2], lines(20 - x, 1)), (w[3], lines(WIDE, 12))])
            }
            "data/duel/practice_text.asm" if op == "practicetext" => Some(vec![
                (w[2], Place::Pages { then_yes_no: false }),
                (w[3], lines(WIDE, 2)),
            ]),
            _ if routine == "ShowSpecialRuleDescription" && op == "tx" => {
                Some(vec![(w[1], lines(16, 1)), (w[2], lines(WIDE, 5))])
            }
            _ => None,
        }
    }
}

/// The decomp's sources (everything but `text/`), in order: lines without
/// their comments, continued lines (`\\` at the end) joined.
fn sources() -> Vec<(String, Vec<String>)> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../decomp/src");
    let mut files = Vec::new();
    let mut dirs = vec![src.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path != src.join("text") {
                    dirs.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "asm") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
        .iter()
        .map(|path| {
            let name = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let mut lines = Vec::new();
            let mut joined = String::new();
            for raw in std::fs::read_to_string(path).unwrap().lines() {
                let code = raw.split(';').next().unwrap().trim_end();
                if let Some(head) = code.strip_suffix('\\') {
                    joined.push_str(head);
                    joined.push(' ');
                    continue;
                }
                joined.push_str(code);
                lines.push(std::mem::take(&mut joined));
            }
            (name, lines)
        })
        .collect()
}

/// The routine a line starts, if any (`Name:` or `Name::`).
fn global_label(line: &str) -> Option<&str> {
    let (label, _) = line.split_once(':')?;
    (!line.starts_with(['.', ' ', '\t'])
        && !label.is_empty()
        && !label.contains(char::is_whitespace))
    .then_some(label)
}

/// The routines that hand a text back in hl (`ret`) only to code that
/// prints it in the duel's wide box, or not at all.
///
/// - The checks the duel runs before an attack, a Trainer card or a Pokémon
///   Power (`EFFECTCMDTYPE_INITIAL_EFFECT_1` and `_2`) say with carry why it
///   can't be done. `UseAttackOrPokemonPower`, `UsePokemonPower` and the
///   Trainer and Metronome paths print that text with
///   `DrawWideTextBox_WaitForInput`; the other paths and the AI only read
///   the carry.
/// - Any other routine qualifies if every call of it prints the text so,
///   drops it, or hands it on from a routine that qualifies. Where the call
///   tests the carry next, only the path with carry, the one that holds the
///   text, counts.
///
/// A routine reached any other way (a pointer, falling into it) does not.
fn returners(sources: &[(String, Vec<String>)]) -> HashSet<String> {
    enum Use {
        Check {
            handled: bool,
        },
        Call {
            file: usize,
            line: usize,
            caller: String,
            tail: bool,
        },
        Other,
    }
    let labels: HashSet<&str> = sources
        .iter()
        .flat_map(|(_, lines)| lines)
        .filter_map(|l| global_label(l))
        .collect();
    let mut uses: HashMap<String, Vec<Use>> = HashMap::new();
    for (file, (_, lines)) in sources.iter().enumerate() {
        let mut caller = "";
        let mut falls = false;
        for (line, code) in lines.iter().enumerate() {
            if let Some(label) = global_label(code) {
                if falls {
                    uses.entry(label.to_string()).or_default().push(Use::Other);
                }
                caller = label;
                continue;
            }
            let w = words(code);
            let Some(&op) = w.first() else { continue };
            if op.starts_with('.') {
                continue;
            }
            falls = !(matches!(op, "ret" | "reti") && w.len() == 1
                || matches!(op, "jp" | "jr") && w.len() == 2);
            if op == "dbw" && w.get(1).is_some_and(|t| t.starts_with("EFFECTCMDTYPE_")) {
                if let Some(&routine) = w.get(2) {
                    let handled = matches!(
                        w[1],
                        "EFFECTCMDTYPE_INITIAL_EFFECT_1" | "EFFECTCMDTYPE_INITIAL_EFFECT_2"
                    );
                    uses.entry(routine.to_string())
                        .or_default()
                        .push(Use::Check { handled });
                }
                continue;
            }
            if matches!(op, "call" | "farcall" | "bank1call" | "jp" | "jr") {
                let target = *w.last().unwrap();
                if labels.contains(target) {
                    uses.entry(target.to_string()).or_default().push(Use::Call {
                        file,
                        line,
                        caller: caller.to_string(),
                        tail: matches!(op, "jp" | "jr"),
                    });
                }
                continue;
            }
            for t in w[1..].iter().filter(|t| labels.contains(*t)) {
                uses.entry(t.to_string()).or_default().push(Use::Other);
            }
        }
    }
    let wide = Place::Box {
        width: WIDE,
        lines: 2,
    };
    let mut ok: HashSet<String> = uses.keys().cloned().collect();
    loop {
        let before = ok.clone();
        ok.retain(|routine| {
            uses[routine].iter().all(|u| match u {
                Use::Check { handled } => *handled,
                Use::Call {
                    tail: true, caller, ..
                } => before.contains(caller),
                Use::Call {
                    file, line, caller, ..
                } => {
                    let after = after_call(&sources[*file].1, *line);
                    after.places.iter().all(|p| *p == wide)
                        && (!after.returned || before.contains(caller))
                }
                Use::Other => false,
            })
        });
        if ok.len() == before.len() {
            return ok;
        }
    }
}

/// Where the text a routine hands back in hl goes after the call at
/// `lines[at]`: on the path with carry, if the next instruction tests it.
fn after_call(lines: &[String], at: usize) -> Followed {
    let unknown = || Followed {
        places: vec![Place::Unknown],
        returned: false,
    };
    let Some(next) = (at + 1..lines.len()).find(|&j| !words(&lines[j]).is_empty()) else {
        return unknown();
    };
    match words(&lines[next]).as_slice() {
        ["ret", "c"] => Followed {
            places: Vec::new(),
            returned: true,
        },
        ["ret", "nc"] | ["jr" | "jp", "nc", _] => follow(lines, next + 1),
        ["jr" | "jp", "c", target] if target.starts_with('.') => {
            local_label(lines, next, target).map_or_else(unknown, |to| follow(lines, to))
        }
        ["jr" | "jp", "c", target] => Followed {
            places: vec![routine_place(target).unwrap_or(Place::Unknown)],
            returned: false,
        },
        _ => follow(lines, next),
    }
}

/// The places where the text in de, from `lines[start]` on, is printed: a
/// coin toss's (`TossCoin…`: two lines of 18 in the box at the bottom) or a
/// card list's header (one line of 18). Unknown on any other path.
fn follow_de(lines: &[String], start: usize) -> Vec<Place> {
    for line in &lines[start..] {
        let n = words(line);
        let Some(&first) = n.first() else { continue };
        let writes_de = n.get(1).is_some_and(|r| matches!(*r, "de" | "d" | "e"));
        match first {
            "call" | "farcall" | "bank1call" | "jp" if n.len() == 2 => {
                return vec![match n[1] {
                    "TossCoin"
                    | "TossCoinATimes"
                    | "TossCoin_Bank1a"
                    | "TossCoinATimes_Bank1a"
                    | "Serial_TossCoin"
                    | "Serial_TossCoinATimes" => Place::Box {
                        width: WIDE,
                        lines: 2,
                    },
                    "SetCardListHeaderAndInfoText" | "SelectCardSearchTarget" => Place::Box {
                        width: WIDE,
                        lines: 1,
                    },
                    _ => Place::Unknown,
                }];
            }
            "ld" | "lb" | "ldtx" | "pop" | "inc" | "dec" if writes_de => return Vec::new(),
            "ld" | "ldh" | "ldtx" | "lb" | "cp" | "and" | "or" | "xor" | "inc" | "dec" | "add"
            | "sub" | "scf" | "ccf" => {}
            _ => return vec![Place::Unknown],
        }
    }
    vec![Place::Unknown]
}

/// Routines that take no text and leave hl alone, so the text in it is
/// printed after them.
const LEAVE_HL: [&str; 2] = ["LoadCardDataToBuffer1_FromDeckIndex", "SwapTurn"];

/// The CPU's instructions and the decomp's macros for them (`lb`, `ldtx`,
/// calls): anything else on a path (another macro) may do anything.
const INSTRUCTIONS: [&str; 50] = [
    "adc",
    "add",
    "and",
    "bank1call",
    "bit",
    "call",
    "ccf",
    "cp",
    "cpl",
    "daa",
    "dec",
    "di",
    "ei",
    "farcall",
    "halt",
    "inc",
    "jp",
    "jr",
    "lb",
    "ld",
    "ldd",
    "ldh",
    "ldi",
    "ldtx",
    "nop",
    "or",
    "pop",
    "push",
    "res",
    "ret",
    "reti",
    "rl",
    "rla",
    "rlc",
    "rlca",
    "rr",
    "rra",
    "rrc",
    "rrca",
    "rst",
    "sbc",
    "scf",
    "set",
    "sla",
    "sra",
    "srl",
    "stop",
    "sub",
    "swap",
    "xor",
];

/// Where the text in hl goes from `lines[start]` on.
struct Followed {
    /// The places paths print it at: every path through the code (local
    /// jumps followed both ways) up to a call of a printing routine, or up
    /// to an instruction that changes hl (the text is not printed on that
    /// path). Unknown if a path calls anything else, leaves the routine
    /// another way, or keeps the text (`ld a, l`, `push hl`, `wTxRam*`: the
    /// game may put a kept text into another, `<RAMTEXT>`).
    places: Vec<Place>,
    /// Whether a path returns it to the caller (`ret`).
    returned: bool,
}

fn follow(lines: &[String], start: usize) -> Followed {
    let mut reached = Vec::new();
    let mut returned = false;
    let mut seen = HashSet::new();
    let mut todo = vec![start];
    while let Some(mut i) = todo.pop() {
        loop {
            if !seen.insert(i) || i >= lines.len() || reached.len() > 16 {
                break;
            }
            let line = lines[i].trim();
            let n = words(line);
            i += 1;
            let Some(&first) = n.first() else { continue };
            // Labels: go on through them.
            if first.ends_with(':') || (line.starts_with('.') && n.len() == 1) {
                continue;
            }
            if !INSTRUCTIONS.contains(&first) {
                reached.push(Place::Unknown);
                break;
            }
            match first {
                "call" | "farcall" | "bank1call" | "jp" | "jr" => {
                    let target = *n.last().unwrap();
                    let conditional = n.len() == 3;
                    if target.starts_with('.') && matches!(first, "jr" | "jp") {
                        let Some(to) = local_label(lines, i - 1, target) else {
                            reached.push(Place::Unknown);
                            break;
                        };
                        todo.push(to);
                        if conditional {
                            continue;
                        }
                        break;
                    }
                    if !conditional && LEAVE_HL.contains(&target) {
                        continue;
                    }
                    let place = routine_place(target);
                    // `call z, Print…` prints it on one path; the other goes on.
                    if conditional && let Some(place) = place {
                        reached.push(place);
                        continue;
                    }
                    reached.push(place.unwrap_or(Place::Unknown));
                    break;
                }
                "ret" => {
                    returned = true;
                    // `ret c`: the other path goes on.
                    if n.len() == 2 {
                        continue;
                    }
                    break;
                }
                "reti" | "rst" => {
                    reached.push(Place::Unknown);
                    break;
                }
                _ => {}
            }
            let writes_hl = matches!(first, "ld" | "ldtx" | "pop" | "add" | "inc" | "dec")
                && n.get(1)
                    .is_some_and(|r| r.contains("hl") && !r.starts_with('['))
                || line.contains("[hli]")
                || line.contains("[hld]");
            if writes_hl && !line.contains("TxRam") {
                break;
            }
            let keeps = (first == "ld" && matches!(n.get(2), Some(&"l") | Some(&"h")))
                || first == "push" && n.get(1) == Some(&"hl")
                || line.contains("TxRam");
            if keeps {
                reached.push(Place::Unknown);
                break;
            }
        }
    }
    Followed {
        places: reached,
        returned,
    }
}

/// The lines of the routine around line `at`: from its global label to the
/// next.
fn routine(lines: &[String], at: usize) -> (usize, usize) {
    let is_global = |l: &str| {
        let l = l.trim_end();
        !l.starts_with(['.', ' ', '\t']) && !l.is_empty() && l.split(':').count() > 1
    };
    let begin = (0..=at).rev().find(|&j| is_global(&lines[j])).unwrap_or(0);
    let end = (at + 1..lines.len())
        .find(|&j| is_global(&lines[j]))
        .unwrap_or(lines.len());
    (begin, end)
}

/// The lines of `tx` lists that `PrintScrollableTextFromList` prints, each
/// text in pages of its own: the lists a routine that calls it loads into
/// hl, directly or through a table of pointers (`dw .List`).
fn paged_lists(lines: &[String]) -> HashSet<usize> {
    // The `tx` lines of the list at `label`, or of the lists its `dw`s name.
    fn mark(lines: &[String], at: usize, label: &str, out: &mut HashSet<usize>, depth: usize) {
        let Some(start) = local_label(lines, at, label) else {
            return;
        };
        for j in start + 1..lines.len() {
            match words(&lines[j]).as_slice() {
                ["tx", ..] => {
                    out.insert(j);
                }
                ["dw", list] if list.starts_with('.') && depth == 0 => mark(lines, j, list, out, 1),
                _ => break,
            }
        }
    }
    let mut out = HashSet::new();
    for (i, line) in lines.iter().enumerate() {
        let w = words(line);
        if !matches!(
            w.as_slice(),
            ["call" | "farcall", "PrintScrollableTextFromList"]
        ) {
            continue;
        }
        let (begin, end) = routine(lines, i);
        for j in begin..end {
            if let ["ld", "hl", list] = words(&lines[j]).as_slice()
                && list.starts_with('.')
            {
                mark(lines, j, list, &mut out, 0);
            }
        }
    }
    out
}

/// The line of local label `target` (`.name`) in the routine around line
/// `from`.
fn local_label(lines: &[String], from: usize, target: &str) -> Option<usize> {
    let (begin, end) = routine(lines, from);
    (begin..end).find(|&j| {
        let l = lines[j].trim_end();
        l == target || l.strip_prefix(target).is_some_and(|r| r.starts_with(':'))
    })
}

/// The full-width lines of a text that fits all of `places`, if any: the
/// text's words flowed into lines of the box's width, each paragraph (after
/// a blank line) starting a page. A text whose spacing is part of it (a
/// cursor's column, padding: lines with leading, trailing or runs of
/// spaces) keeps its lines, and each word the tile it starts on
/// ([`columns`]). None for a place not known.
/// A line that is a card's type in brackets.
fn is_tag(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(']')
}

pub fn full_width_lines(text: &str, places: &[Place]) -> Option<Vec<String>> {
    if places.is_empty() || places.contains(&Place::Unknown) || text.contains(RAMTEXT) {
        return None;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    if lines
        .iter()
        .any(|l| l.starts_with(' ') || l.ends_with(' ') || l.contains("  "))
    {
        let lines = lines
            .iter()
            .map(|l| columns(l))
            .collect::<Option<Vec<_>>>()?;
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        return fits(&borrowed, places).then_some(lines);
    }
    let width = |s: &str| text::line_widths(s, Width::Full).unwrap()[0] / 2;
    // A menu's item or label: the line as it is.
    if places.iter().any(|p| matches!(p, Place::Item { .. })) {
        let room = places
            .iter()
            .map(|p| match p {
                Place::Item { width, .. } | Place::Box { width, .. } => *width,
                Place::Pages { .. } => WIDE,
                Place::Unknown => 0,
            })
            .min()?;
        return (lines.len() == 1 && width(lines[0]) <= room).then(|| vec![lines[0].to_string()]);
    }
    let room = places
        .iter()
        .map(|p| match p {
            Place::Box { width, .. } => *width,
            _ => WIDE,
        })
        .min()?;
    let most = places
        .iter()
        .map(|p| match p {
            Place::Box { lines, .. } => *lines,
            _ => usize::MAX,
        })
        .min()?;
    let yes_no = places.contains(&Place::Pages { then_yes_no: true });
    let mut out: Vec<String> = Vec::new();
    for (n, paragraph) in lines.split(|l| l.is_empty()).enumerate() {
        if paragraph.is_empty() {
            return None;
        }
        // Each paragraph starts a page.
        if n > 0 {
            if most != usize::MAX {
                return None;
            }
            if out.len() % 2 == 1 {
                out.push(String::new());
            }
        }
        // A card's type in brackets ("[Special Energy Card]") keeps a line
        // of its own; the lines around it flow.
        for (tag, run) in paragraph
            .chunk_by(|a, b| is_tag(a) == is_tag(b))
            .map(|run| (is_tag(run[0]), run))
        {
            if tag {
                for line in run {
                    if width(line) > room {
                        return None;
                    }
                    out.push(line.to_string());
                }
            } else {
                out.extend(flow(run, room, width)?);
            }
        }
    }
    // The yes/no menu covers the last page's second line.
    let fits = out.len() <= most && !(yes_no && out.len().is_multiple_of(2));
    fits.then_some(out)
}

/// A half-width line made full-width with each word on the tile it starts
/// on (two spaces of half-width text are one of full-width), and as wide as
/// it was at least, so padding still clears what it covered. None if a word
/// starts mid-tile, or would run into the next.
pub fn columns(line: &str) -> Option<String> {
    let width = |s: &str| text::line_widths(s, Width::Full).unwrap()[0] / 2;
    let mut out = String::new();
    let mut at = 0; // tiles of `out`
    let mut half = 0; // half tiles of `line` before the word
    for (n, word) in line.split(' ').enumerate() {
        if n > 0 {
            half += 1; // the space before it
        }
        if word.is_empty() {
            continue;
        }
        if half % 2 == 1 {
            return None;
        }
        let start = half / 2;
        let gap = usize::from(!out.trim_end().is_empty());
        if start < at + gap && !(start == at && gap == 0) {
            return None;
        }
        out.extend(std::iter::repeat_n(' ', start - at));
        out.push_str(word);
        at = start + width(word);
        half += text::line_widths(word, Width::Half).unwrap()[0];
    }
    let end = half.div_ceil(2);
    out.extend(std::iter::repeat_n(' ', end.saturating_sub(at)));
    Some(out)
}

/// A paragraph's words in lines of at most `room` tiles: as few lines as
/// possible, then the evenest, breaking at the end of a sentence or of one
/// of the translation's own lines where that costs no extra line.
fn flow(paragraph: &[&str], room: usize, width: impl Fn(&str) -> usize) -> Option<Vec<String>> {
    // Words, and how well a line ends after each.
    let mut tokens: Vec<(&str, usize)> = Vec::new();
    for line in paragraph {
        let mut split = line.split(' ').peekable();
        while let Some(word) = split.next() {
            if width(word) > room {
                return None;
            }
            let bonus = if word.ends_with(['.', '!', '?']) {
                // The end of a sentence: worth a line 5 tiles short.
                25
            } else if split.peek().is_none() || word.ends_with([',', ':', ';']) {
                // The end of one of the translation's lines, or a clause.
                9
            } else {
                0
            };
            tokens.push((word, bonus));
        }
    }
    // best[i]: (lines, cost, next break) setting tokens[i..].
    let n = tokens.len();
    let mut best: Vec<Option<(usize, i64, usize)>> = vec![None; n + 1];
    best[n] = Some((0, 0, n));
    for i in (0..n).rev() {
        let mut used = 0;
        for j in i + 1..=n {
            used += width(tokens[j - 1].0) + usize::from(j > i + 1);
            if used > room {
                break;
            }
            let Some((lines, cost, _)) = best[j] else {
                continue;
            };
            // The last line may be short; any other pays for its slack.
            let slack = if j == n { 0 } else { (room - used) as i64 };
            let cost = cost + slack * slack - tokens[j - 1].1 as i64;
            let candidate = (lines + 1, cost, j);
            if best[i].is_none_or(|b| (candidate.0, candidate.1) < (b.0, b.1)) {
                best[i] = Some(candidate);
            }
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let (_, _, j) = best[i]?;
        let line: Vec<&str> = tokens[i..j].iter().map(|t| t.0).collect();
        out.push(line.join(" "));
        i = j;
    }
    Some(out)
}

/// Whether full-width `lines` fit `places` as they are.
pub fn fits(lines: &[&str], places: &[Place]) -> bool {
    let width = |s: &str| text::line_widths(s, Width::Full).unwrap()[0] / 2;
    places.iter().all(|p| match p {
        Place::Pages { then_yes_no } => {
            lines.iter().all(|l| width(l) <= WIDE)
                && !(*then_yes_no && lines.len().is_multiple_of(2))
        }
        Place::Box {
            width: room,
            lines: most,
        } => lines.len() <= *most && lines.iter().all(|l| width(l) <= *room),
        Place::Item { width: room, .. } => lines.len() == 1 && width(lines[0]) <= *room,
        Place::Unknown => false,
    })
}

/// The menus whose every item and label fits full-width, by number. Menus
/// that share a text go together: the text has one width.
pub fn menus_that_fit(
    entries: &[&text::Entry],
    places: &HashMap<String, Vec<Place>>,
) -> HashSet<usize> {
    let by_label: HashMap<&str, &text::Entry> =
        entries.iter().map(|e| (e.label.as_str(), *e)).collect();
    // Union-find over menus.
    let mut parent: BTreeMap<usize, usize> = BTreeMap::new();
    fn root(parent: &mut BTreeMap<usize, usize>, m: usize) -> usize {
        let p = *parent.entry(m).or_insert(m);
        if p == m {
            return m;
        }
        let r = root(parent, p);
        parent.insert(m, r);
        r
    }
    let mut fit: BTreeMap<usize, bool> = BTreeMap::new();
    for (label, ps) in places {
        let menus: Vec<usize> = ps
            .iter()
            .filter_map(|p| match p {
                Place::Item { menu, .. } => Some(*menu),
                _ => None,
            })
            .collect();
        let Some(&first) = menus.first() else {
            continue;
        };
        // A text the language leaves as the ROM has it stays as it is; one
        // already full-width fits as it is written.
        let ok = by_label.get(label.as_str()).is_none_or(|e| match e.width {
            Width::Full => fits(&e.text.split('\n').collect::<Vec<_>>(), ps),
            Width::Half => full_width_lines(&e.text, ps).is_some(),
        });
        for &m in &menus {
            *fit.entry(m).or_insert(true) &= ok;
            let (a, b) = (root(&mut parent, first), root(&mut parent, m));
            parent.insert(a, b);
        }
    }
    let mut component: BTreeMap<usize, bool> = BTreeMap::new();
    for (&m, &ok) in &fit {
        let r = root(&mut parent, m);
        *component.entry(r).or_insert(true) &= ok;
    }
    fit.keys()
        .copied()
        .filter(|&m| component[&root(&mut parent, m)])
        .collect()
}
