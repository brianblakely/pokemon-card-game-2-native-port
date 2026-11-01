//! tcg2-recomp: the decomp, translated to Rust.
//!
//! [`prepare`] builds an annotated copy of src/decomp (labels at every
//! code/data boundary; byte-identical, checked against `rom.sha1`) and reads
//! back its ROM and symbols. [`translate`] finds the program's code from
//! those and writes it as Rust modules (see `codegen`). The kernel's build
//! script runs both; `tcg2-recomp` (the binary) does the same for
//! inspection.

pub mod codegen;
pub mod decode;
pub mod discover;
pub mod source;
pub mod sym;

use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use discover::{ROM_BANKS, Seeds};
use source::Kind;
use sym::Symbols;

pub struct Prepared {
    pub rom: Vec<u8>,
    pub sym: Symbols,
    pub scan: source::Scan,
}

fn sources(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, root, out)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("asm" | "inc")
        ) {
            let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, text));
        }
    }
    Ok(())
}

/// Mirrors `from` into `to` (skipping build outputs that the annotated build
/// makes itself), keeping modification times so make sees what is current.
fn mirror(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src = entry.path();
        let name = entry.file_name();
        let dst = to.join(&name);
        let name = name.to_string_lossy();
        if name.starts_with('.') && name != ".rgbds-version" {
            continue;
        }
        if src.is_dir() {
            mirror(&src, &dst)?;
            continue;
        }
        if matches!(
            src.extension().and_then(|e| e.to_str()),
            Some("o" | "gbc" | "sym" | "map")
        ) {
            continue;
        }
        let meta = fs::metadata(&src).map_err(|e| e.to_string())?;
        let same = fs::metadata(&dst)
            .map(|d| d.len() == meta.len() && d.modified().ok() == meta.modified().ok())
            .unwrap_or(false);
        if same {
            continue;
        }
        fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        if let Ok(time) = meta.modified() {
            let file = fs::File::options()
                .write(true)
                .open(&dst)
                .map_err(|e| e.to_string())?;
            file.set_modified(time).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Builds the annotated decomp in `work` and reads its ROM and symbols.
pub fn prepare(decomp: &Path, work: &Path) -> Result<Prepared, String> {
    let src_root = decomp.join("src");
    let mut files = Vec::new();
    sources(&src_root, &src_root, &mut files)?;
    let macros = source::classify_macros(&files);
    let scan = source::scan(&files, &macros);

    mirror(decomp, work)?;
    for (rel, text) in &scan.files {
        let path = work.join("src").join(rel);
        let current = fs::read_to_string(&path).unwrap_or_default();
        if current != *text {
            fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let output = Command::new(std::env::var("MAKE").unwrap_or_else(|_| "make".into()))
        .arg("-C")
        .arg(work)
        .arg(format!("-j{jobs}"))
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .output()
        .map_err(|e| {
            format!("running make for the decomp (is RGBDS installed? use the Nix shell): {e}")
        })?;
    if !output.status.success() {
        return Err(format!(
            "the annotated decomp build failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let rom = fs::read(work.join("poketcg2.gbc")).map_err(|e| format!("annotated ROM: {e}"))?;
    let sym_text =
        fs::read_to_string(work.join("poketcg2.sym")).map_err(|e| format!("symbols: {e}"))?;
    let sym = Symbols::parse(&sym_text)?;
    Ok(Prepared { rom, sym, scan })
}

/// A routine the game copies from ROM into RAM and runs there: its RAM
/// label, its ROM label and the label just past it in ROM.
pub struct RamRoutine<'a> {
    pub ram: &'a str,
    pub rom: &'a str,
    pub rom_end: &'a str,
}

/// The game's RAM routines: the OAM DMA routine CopyDMAFunction copies to
/// HRAM (hDMAFunction) from DMA, which ends where JumpToFunctionInTable
/// begins.
pub const RAM_ROUTINES: [RamRoutine<'static>; 1] = [RamRoutine {
    ram: "hDMAFunction",
    rom: "DMA",
    rom_end: "JumpToFunctionInTable",
}];

pub struct Translation {
    pub files: Vec<(String, String)>,
    pub report: String,
}

pub fn seeds(prepared: &Prepared) -> Seeds {
    let mut code = Vec::new();
    for (vector, name) in [
        (0x00, "rst $00"),
        (0x08, "rst $08"),
        (0x10, "rst $10"),
        (0x18, "rst $18"),
        (0x20, "rst $20"),
        (0x28, "rst $28"),
        (0x30, "rst $30"),
        (0x38, "rst $38"),
        (0x40, "VBlank interrupt"),
        (0x48, "LCD interrupt"),
        (0x50, "timer interrupt"),
        (0x58, "serial interrupt"),
        (0x60, "joypad interrupt"),
        (0x100, "entry point"),
    ] {
        code.push((0usize, vector as u16, name.to_string()));
    }
    let mut data = BTreeSet::new();
    let mut labels: Vec<(&String, &Kind)> = prepared
        .scan
        .labels
        .iter()
        .chain(prepared.scan.injected.iter())
        .collect();
    labels.sort();
    for (name, kind) in labels {
        let Some(symbol) = prepared.sym.get(name) else {
            continue;
        };
        if symbol.addr >= 0x8000 || symbol.bank as usize >= ROM_BANKS {
            continue;
        }
        let bank = if symbol.addr < 0x4000 {
            0
        } else {
            symbol.bank as usize
        };
        match kind {
            Kind::Code => code.push((bank, symbol.addr, name.clone())),
            Kind::Data => {
                data.insert((bank, symbol.addr));
            }
        }
    }
    Seeds { code, data }
}

pub fn translate(prepared: &Prepared, ram_routines: &[RamRoutine]) -> Result<Translation, String> {
    let seeds = seeds(prepared);
    let program = discover::discover(&prepared.rom, &seeds);
    let mut files = Vec::new();
    let mut report = String::new();
    let mut root = String::new();
    root.push_str(
        "// The program, translated from src/decomp by tcg2-recomp. Generated; do not edit.\n\n",
    );
    root.push_str("/// A translated function: runs from a block start until control leaves it,\n");
    root.push_str("/// returning the address where execution continues.\n");
    root.push_str("pub type Func = fn(&mut tcg2_core::Machine, u16) -> u16;\n\n");
    let mut bank_list = Vec::new();
    let (mut total_insns, mut total_blocks, mut total_functions) = (0, 0, 0);
    for bank in 0..ROM_BANKS {
        let code = &program.banks[bank];
        if code.starts.is_empty() {
            continue;
        }
        let functions = codegen::functions(&program, bank, &prepared.sym, &prepared.scan.locations);
        let blocks: usize = functions.iter().map(|f| f.blocks.len()).sum();
        let _ = writeln!(
            report,
            "bank {bank:02x}: {} instructions, {blocks} blocks, {} functions",
            code.insns.len(),
            functions.len()
        );
        total_insns += code.insns.len();
        total_blocks += blocks;
        total_functions += functions.len();
        let module = codegen::bank_module(&prepared.rom, &prepared.sym, bank, &functions);
        let file = format!("bank_{bank:02x}.rs");
        let _ = writeln!(
            root,
            "#[allow(unused_mut, unused_macros, unreachable_code, clippy::all)]\npub mod bank_{bank:02x} {{\n    include!(concat!(env!(\"OUT_DIR\"), \"/{file}\"));\n}}"
        );
        files.push((file, module));
        bank_list.push(bank);
    }
    let _ = writeln!(
        report,
        "total: {total_insns} instructions, {total_blocks} blocks, {total_functions} functions in {} banks",
        bank_list.len()
    );
    root.push_str("\n/// Each bank with code and its dispatch entries.\n");
    root.push_str("pub static BANKS: &[(usize, &[(u16, Func)])] = &[\n");
    for bank in &bank_list {
        let _ = writeln!(root, "    (0x{bank:02x}, bank_{bank:02x}::ENTRIES),");
    }
    root.push_str("];\n\n");

    root.push_str("/// Routines the game copies into RAM and runs there: RAM address, ROM\n");
    root.push_str("/// address (bank 0) and length. The dispatcher runs the ROM translation\n");
    root.push_str("/// while the RAM still holds the same bytes.\n");
    root.push_str("pub static RAM_ROUTINES: &[(u16, u16, u16)] = &[\n");
    for routine in ram_routines {
        let find = |name: &str| {
            prepared
                .sym
                .get(name)
                .ok_or(format!("no symbol {name} for a RAM routine"))
        };
        let (ram, rom, end) = (
            find(routine.ram)?,
            find(routine.rom)?,
            find(routine.rom_end)?,
        );
        if rom.addr >= 0x4000 || end.addr <= rom.addr {
            return Err(format!("RAM routine {} must lie in bank 0", routine.rom));
        }
        let _ = writeln!(
            root,
            "    (0x{:04x}, 0x{:04x}, 0x{:04x}), // {} <- {}",
            ram.addr,
            rom.addr,
            end.addr - rom.addr,
            routine.ram,
            routine.rom
        );
    }
    root.push_str("];\n\n");

    root.push_str("/// The program's global labels (bank, address, name), for diagnostics.\n");
    root.push_str("pub static LABELS: &[(u8, u16, &str)] = &[\n");
    for symbol in &prepared.sym.all {
        if symbol.addr < 0x8000 && symbol.is_global() {
            let _ = writeln!(
                root,
                "    (0x{:02x}, 0x{:04x}, {:?}),",
                symbol.bank, symbol.addr, symbol.name
            );
        }
    }
    root.push_str("];\n\n");

    root.push_str(
        "/// Text the game keeps outside its text table (a deck name's suffix, default\n",
    );
    root.push_str("/// names): bank, address, bytes up to the next label, and label.\n");
    root.push_str("pub static TEXT_DATA: &[(u8, u16, u16, &str)] = &[\n");
    let mut text: Vec<&String> = prepared.scan.text.iter().collect();
    text.sort();
    for name in text {
        let symbol = prepared
            .sym
            .get(name)
            .ok_or(format!("no symbol for the text {name}"))?;
        let bank_end = if symbol.addr < 0x4000 { 0x4000 } else { 0x8000 };
        let end = prepared
            .sym
            .all
            .iter()
            .filter(|s| s.bank == symbol.bank && s.addr > symbol.addr && s.addr < bank_end)
            .map(|s| s.addr)
            .min()
            .unwrap_or(bank_end);
        let _ = writeln!(
            root,
            "    (0x{:02x}, 0x{:04x}, {}, {:?}),",
            symbol.bank,
            symbol.addr,
            end - symbol.addr,
            symbol.name
        );
    }
    root.push_str("];\n");
    files.push(("kernel.rs".to_string(), root));

    if !program.warnings.is_empty() {
        let _ = writeln!(report, "{} warnings:", program.warnings.len());
        for w in &program.warnings {
            let _ = writeln!(report, "  {w}");
        }
    }
    Ok(Translation { files, report })
}

/// The repository's src/decomp, from a crate directory inside src/runtime.
pub fn decomp_dir(manifest_dir: &Path) -> PathBuf {
    manifest_dir.join("../../../decomp")
}
