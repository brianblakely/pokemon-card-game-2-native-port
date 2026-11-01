//! What the decomp's sources say about its bytes: which labels begin code and
//! which begin data, and where code resumes after inline data without a label
//! of its own (`start_script` is `call StartScript` followed by script
//! bytecode; the code after the script's `quit_script` is reached only
//! through the script engine's return).
//!
//! The scanner classifies every statement as code (an SM83 mnemonic or a
//! macro that expands to one first), data, or neither, and writes an
//! annotated copy of each file with a local label at every unlabelled change
//! between the two (`.tcg2_code_N`, `.tcg2_data_N`). Labels emit no bytes,
//! so the annotated build is byte-identical (the build checks its SHA-1)
//! and its symbol file gives each boundary's address.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Code,
    Data,
}

const MNEMONICS: &[&str] = &[
    "ld", "ldh", "ldi", "ldd", "push", "pop", "add", "adc", "sub", "sbc", "and", "xor", "or", "cp",
    "inc", "dec", "daa", "cpl", "ccf", "scf", "nop", "halt", "stop", "di", "ei", "rlca", "rla",
    "rrca", "rra", "rlc", "rl", "rrc", "rr", "sla", "sra", "srl", "swap", "bit", "set", "res",
    "jp", "jr", "call", "ret", "reti", "rst",
];

const DATA_DIRECTIVES: &[&str] = &["db", "dw", "dl", "ds", "incbin"];

/// Directives that emit nothing.
const NEUTRAL: &[&str] = &[
    "def",
    "redef",
    "equ",
    "equs",
    "section",
    "include",
    "if",
    "elif",
    "else",
    "endc",
    "rept",
    "for",
    "endr",
    "break",
    "macro",
    "macro?",
    "endm",
    "shift",
    "pushs",
    "pops",
    "assert",
    "static_assert",
    "charmap",
    "newcharmap",
    "setcharmap",
    "pushc",
    "popc",
    "purge",
    "export",
    "opt",
    "pusho",
    "popo",
    "print",
    "println",
    "warn",
    "fail",
    "rsreset",
    "rsset",
    "rb",
    "rw",
    "union",
    "nextu",
    "endu",
    "load",
    "endl",
    "align",
    "endsection",
    "fragment",
];

/// The statements of one line, with its label.
struct Line<'a> {
    label: Option<&'a str>,
    /// The first word of the statement, lowercased, if any.
    word: Option<String>,
    second: Option<String>,
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            ';' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Blanks `/* ... */` comments, keeping every line (and line numbers).
fn strip_block_comments(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_comment = false;
    for raw in text.lines() {
        let mut out = String::new();
        let mut rest = raw;
        loop {
            if in_comment {
                match rest.find("*/") {
                    Some(end) => {
                        rest = &rest[end + 2..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else {
                let code = strip_comment(rest);
                match code.find("/*") {
                    Some(start) => {
                        out.push_str(&rest[..start]);
                        rest = &rest[start + 2..];
                        in_comment = true;
                    }
                    None => {
                        out.push_str(rest);
                        break;
                    }
                }
            }
        }
        lines.push(out);
    }
    lines
}

fn parse_line(raw: &str) -> Line<'_> {
    let line = strip_comment(raw);
    let mut rest = line;
    let mut label = None;
    let starts_label = line.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '.');
    if starts_label {
        let end = line
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '#' | '$'))
            })
            .unwrap_or(line.len());
        let name = &line[..end];
        let after = &line[end..];
        if let Some(stripped) = after.strip_prefix("::").or_else(|| after.strip_prefix(':')) {
            label = Some(name);
            rest = stripped;
        } else if name.starts_with('.') {
            label = Some(name);
            rest = after;
        }
    }
    let mut words = rest
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase());
    let word = words.next();
    // `NAME EQU value` puts the directive second.
    let second = words.next();
    Line {
        label,
        word,
        second,
    }
}

/// How a statement's first word reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    Emits(Kind),
    /// Emits nothing (directives, constants, labels in macro bodies).
    Neutral,
    /// Not a known instruction, directive or macro.
    Unknown,
}

fn is_macro_keyword(word: &str) -> bool {
    word.eq_ignore_ascii_case("macro") || word.eq_ignore_ascii_case("macro?")
}

fn word_kind(word: &str, second: Option<&str>, macros: &HashMap<String, Word>) -> Word {
    if MNEMONICS.contains(&word) {
        return Word::Emits(Kind::Code);
    }
    if DATA_DIRECTIVES.contains(&word) {
        return Word::Emits(Kind::Data);
    }
    if NEUTRAL.contains(&word) || word.starts_with('\\') || word.contains(':') || word.contains('=')
    {
        return Word::Neutral;
    }
    if let Some(second) = second
        && matches!(
            second,
            "equ" | "equs" | "=" | "set" | "rb" | "rw" | "rl" | "macro" | "macro?"
        )
    {
        return Word::Neutral;
    }
    macros.get(word).copied().unwrap_or(Word::Unknown)
}

/// A macro's name and its body's statements (first and second word).
type MacroBody = (String, Vec<(String, Option<String>)>);

/// Classifies every macro by the first statement of its body that emits
/// bytes (`MACRO name` and `MACRO? name` alike).
pub fn classify_macros(files: &[(String, String)]) -> HashMap<String, Word> {
    let mut bodies: Vec<MacroBody> = Vec::new();
    for (_, text) in files {
        let mut current: Option<MacroBody> = None;
        for line in strip_block_comments(text) {
            let line = strip_comment(&line).trim().to_string();
            let words: Vec<String> = line
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|w| !w.is_empty())
                .map(|w| w.to_ascii_lowercase())
                .collect();
            if current.is_none() {
                if words.first().map(|w| is_macro_keyword(w)).unwrap_or(false)
                    && let Some(name) = words.get(1)
                {
                    current = Some((name.clone(), Vec::new()));
                }
                continue;
            }
            if words.first().map(|w| w == "endm").unwrap_or(false) {
                bodies.push(current.take().unwrap());
                continue;
            }
            if let Some(first) = words.first() {
                current
                    .as_mut()
                    .unwrap()
                    .1
                    .push((first.clone(), words.get(1).cloned()));
            }
        }
    }
    let mut kinds: HashMap<String, Word> = HashMap::new();
    loop {
        let mut changed = false;
        for (name, words) in &bodies {
            if kinds.contains_key(name) {
                continue;
            }
            let mut result = Some(Word::Neutral);
            for (word, second) in words {
                match word_kind(word, second.as_deref(), &kinds) {
                    Word::Emits(kind) => {
                        result = Some(Word::Emits(kind));
                        break;
                    }
                    Word::Neutral => {}
                    Word::Unknown => {
                        // A macro not classified yet (or never defined).
                        let defined = bodies.iter().any(|(n, _)| n == word);
                        result = if defined { None } else { Some(Word::Unknown) };
                        break;
                    }
                }
            }
            if let Some(kind) = result {
                kinds.insert(name.clone(), kind);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (name, _) in &bodies {
        kinds.entry(name.clone()).or_insert(Word::Unknown);
    }
    kinds
}

pub struct Scan {
    /// The annotated copy of each file (same relative path).
    pub files: Vec<(String, String)>,
    /// Labels (as the symbol file names them) and the kind of what follows.
    pub labels: HashMap<String, Kind>,
    /// Injected labels and the kind of what follows each.
    pub injected: HashMap<String, Kind>,
    /// Where each global label is defined (file, 1-based line).
    pub locations: HashMap<String, (String, usize)>,
    /// Labels, global or local, of text kept outside `text/` (the text
    /// table's files): their first statement is a text macro.
    pub text: HashSet<String>,
}

/// The decomp's text macros (`macros/text.asm`).
const TEXT_MACROS: [&str; 4] = ["text", "textfw", "katakana", "hiragana"];

/// Scans `files` (relative path, contents), with `macros` classified.
pub fn scan(files: &[(String, String)], macros: &HashMap<String, Word>) -> Scan {
    let mut out = Scan {
        files: Vec::new(),
        labels: HashMap::new(),
        injected: HashMap::new(),
        locations: HashMap::new(),
        text: HashSet::new(),
    };
    let mut counter = 0usize;
    for (path, text) in files {
        let mut annotated = String::with_capacity(text.len() + 1024);
        let mut global: Option<String> = None;
        // Labels waiting for the first statement that emits bytes.
        let mut pending: Vec<String> = Vec::new();
        let mut previous: Option<Kind> = None;
        let mut in_macro = false;
        let mut repeat_depth = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        let clean = strip_block_comments(text);
        for (number, raw) in text.lines().enumerate() {
            let line = parse_line(&clean[number]);
            let word = line.word.as_deref();
            if in_macro {
                if word == Some("endm") {
                    in_macro = false;
                }
                annotated.push_str(raw);
                annotated.push('\n');
                continue;
            }
            match word {
                Some("macro") | Some("macro?") => in_macro = true,
                Some("rept") | Some("for") => repeat_depth += 1,
                Some("endr") => repeat_depth = repeat_depth.saturating_sub(1),
                Some("section") => {
                    global = None;
                    previous = None;
                    pending.clear();
                }
                _ => {}
            }
            if let Some(label) = line.label {
                let full = if let Some(local) = label.strip_prefix('.') {
                    match &global {
                        Some(g) => format!("{g}.{local}"),
                        None => label.to_string(),
                    }
                } else {
                    if !label.contains('.') {
                        global = Some(label.to_string());
                        out.locations
                            .insert(label.to_string(), (path.clone(), number + 1));
                    }
                    label.to_string()
                };
                pending.push(full);
            }
            let class = match word {
                Some(w) => word_kind(w, line.second.as_deref(), macros),
                None => Word::Neutral,
            };
            if class == Word::Unknown {
                // Something emits bytes we cannot classify: its labels are
                // neither seeds nor barriers, and no boundary is inferred
                // across it.
                pending.clear();
                previous = None;
            }
            if let Word::Emits(kind) = class {
                let unlabelled = pending.is_empty();
                let text =
                    word.is_some_and(|w| TEXT_MACROS.contains(&w)) && !path.starts_with("text/");
                for label in pending.drain(..) {
                    if text {
                        out.text.insert(label.clone());
                    }
                    out.labels.entry(label).or_insert(kind);
                }
                if unlabelled
                    && previous.is_some()
                    && previous != Some(kind)
                    && repeat_depth == 0
                    && let Some(g) = &global
                {
                    let name = match kind {
                        Kind::Code => format!("tcg2_code_{counter}"),
                        Kind::Data => format!("tcg2_data_{counter}"),
                    };
                    counter += 1;
                    if seen.insert(name.clone()) {
                        annotated.push('.');
                        annotated.push_str(&name);
                        annotated.push_str(":\n");
                        out.injected.insert(format!("{g}.{name}"), kind);
                    }
                }
                previous = Some(kind);
            }
            annotated.push_str(raw);
            annotated.push('\n');
        }
        out.files.push((path.clone(), annotated));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_after_an_inline_script_gets_a_label() {
        let macros_src =
            "MACRO start_script\n\tcall StartScript\nENDM\nMACRO quit_script\n\tdb $3c\nENDM\n"
                .to_string();
        let file =
            "Func:\n\txor a\n\tstart_script\n\tquit_script\n\tld a, 1\n\tret\n.Table\n\tdb 1, 2\n"
                .to_string();
        let files = vec![
            ("macros.asm".to_string(), macros_src),
            ("f.asm".to_string(), file),
        ];
        let macros = classify_macros(&files);
        assert_eq!(macros.get("start_script"), Some(&Word::Emits(Kind::Code)));
        assert_eq!(macros.get("quit_script"), Some(&Word::Emits(Kind::Data)));
        let scan = scan(&files[1..], &macros);
        assert_eq!(scan.labels.get("Func"), Some(&Kind::Code));
        assert_eq!(scan.labels.get("Func.Table"), Some(&Kind::Data));
        let mut injected: Vec<_> = scan.injected.iter().collect();
        injected.sort();
        assert_eq!(
            injected,
            vec![
                (&"Func.tcg2_code_1".to_string(), &Kind::Code),
                (&"Func.tcg2_data_0".to_string(), &Kind::Data)
            ]
        );
        assert!(
            scan.files[0]
                .1
                .contains(".tcg2_data_0:\n\tquit_script\n.tcg2_code_1:\n\tld a, 1")
        );
    }

    #[test]
    fn text_outside_the_text_table_is_listed() {
        let macros_src = "MACRO textfw\n\tdb \\1\nENDM\n".to_string();
        let file = "Func:\n\tret\n.Suffix\n\ttextfw \"x\"\n\tdb 0\nTable:\n\tdb 1\n".to_string();
        let texts = "Hand:\n\ttextfw \"y\"\n".to_string();
        let files = vec![
            ("macros.asm".to_string(), macros_src),
            ("f.asm".to_string(), file),
            ("text/t.asm".to_string(), texts),
        ];
        let macros = classify_macros(&files);
        let scan = scan(&files[1..], &macros);
        assert_eq!(scan.text, HashSet::from(["Func.Suffix".to_string()]));
    }
}
