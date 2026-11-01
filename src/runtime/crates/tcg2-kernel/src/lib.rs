//! The kernel: Pokémon Card GB2's program, translated to Rust from
//! src/decomp when this crate builds, running on tcg2-core's machine.
//!
//! The translated functions run straight-line native code between control
//! transfers. The dispatcher here resumes them at block starts, takes
//! interrupts between blocks, sleeps the clock through `halt` to the next
//! event, and returns to the host at the end of every frame. The game
//! therefore costs the host only what its own instructions cost, plus one
//! wake-up per frame.

use std::collections::HashMap;
use std::sync::OnceLock;
use tcg2_core::{Fault, Machine, RomError, verify_rom};
pub use tcg2_locale::Language;

pub mod script;

#[allow(clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/kernel.rs"));
}

pub use generated::Func;

/// What the translation found (per-bank instruction counts, warnings).
pub const TRANSLATION_REPORT: &str = include_str!(concat!(env!("OUT_DIR"), "/report.txt"));

const BANKS: usize = 128;

type Table = Box<[Option<Func>; 0x4000]>;

/// The dispatch tables: for each ROM bank, the function that runs each
/// block start.
pub struct Code {
    tables: Vec<Option<Table>>,
}

impl Default for Code {
    fn default() -> Self {
        Self::new()
    }
}

impl Code {
    pub fn new() -> Self {
        let mut tables: Vec<Option<Table>> = (0..BANKS).map(|_| None).collect();
        for &(bank, entries) in generated::BANKS {
            let table = tables[bank].get_or_insert_with(|| {
                vec![None; 0x4000]
                    .into_boxed_slice()
                    .try_into()
                    .expect("table size")
            });
            for &(addr, f) in entries {
                table[addr as usize & 0x3fff] = Some(f);
            }
        }
        Self { tables }
    }

    #[inline(always)]
    fn lookup(&self, bank: usize, pc: u16) -> Option<Func> {
        match &self.tables[bank] {
            Some(table) => table[pc as usize & 0x3fff],
            None => None,
        }
    }

    /// Runs the translated code at `pc` until control leaves its function;
    /// returns where execution continues.
    #[inline]
    pub fn dispatch(&self, m: &mut Machine, pc: u16) -> u16 {
        let bank = match pc {
            0x0000..=0x3fff => 0,
            0x4000..=0x7fff => m.cart.rom_bank as usize % BANKS,
            _ => return self.ram_code(m, pc),
        };
        match self.lookup(bank, pc) {
            Some(f) => f(m, pc),
            None => lost(m, bank as u16, pc),
        }
    }

    /// Code the game runs from RAM: the OAM DMA routine it copies to HRAM
    /// (run as its ROM translation while RAM holds the same bytes) and the
    /// `jp nn` trampolines its interrupt handlers call through.
    #[cold]
    fn ram_code(&self, m: &mut Machine, pc: u16) -> u16 {
        for &(ram, rom, len) in generated::RAM_ROUTINES {
            if (ram..ram + len).contains(&pc) {
                let same = (0..len).all(|i| m.read(ram + i) == m.cart.rom[(rom + i) as usize]);
                if same {
                    let at = rom + (pc - ram);
                    return match self.lookup(0, at) {
                        Some(f) => f(m, at),
                        None => lost(m, 0, at),
                    };
                }
            }
        }
        if m.read(pc) == 0xc3 {
            m.tick(4);
            return m.read16(pc.wrapping_add(1));
        }
        lost(m, 0xffff, pc)
    }
}

/// The translated code reached an address no block starts at.
#[cold]
#[inline(never)]
pub fn lost(m: &mut Machine, bank: u16, pc: u16) -> u16 {
    m.raise_fault(bank, pc, "no translated code at this address");
    pc
}

/// The translated code reached bytes that are no SM83 instruction (the
/// console locks up there).
#[cold]
#[inline(never)]
pub fn invalid(m: &mut Machine, bank: u16, pc: u16) -> u16 {
    m.raise_fault(bank, pc, "not an SM83 instruction (the console hangs here)");
    pc
}

/// Every translated block start: (bank, address).
pub fn entries() -> impl Iterator<Item = (usize, u16)> {
    generated::BANKS
        .iter()
        .flat_map(|&(bank, entries)| entries.iter().map(move |&(addr, _)| (bank, addr)))
}

/// Routines the game runs from RAM as copies of ROM code: (RAM address, ROM
/// address in bank 0, length).
pub fn ram_routines() -> &'static [(u16, u16, u16)] {
    generated::RAM_ROUTINES
}

/// A global label's location: (bank, address).
pub fn label(name: &str) -> Option<(u16, u16)> {
    static BY_NAME: OnceLock<HashMap<&'static str, (u16, u16)>> = OnceLock::new();
    let by_name = BY_NAME.get_or_init(|| {
        let mut map = HashMap::with_capacity(generated::LABELS.len());
        for &(bank, addr, name) in generated::LABELS {
            map.entry(name).or_insert((bank as u16, addr));
        }
        map
    });
    by_name.get(name).copied()
}

/// Calls one of the game's own routines from where it waits for the next
/// frame, as if that wait had called it: the routine runs as the next code
/// and returns into the wait. Only bank 0 routines (which reach other banks
/// through the game's own far calls, as `CallStartupDebugMenu` does).
/// `Routine,a=02,bc=0601` sets registers (hex) first, for a routine that
/// takes arguments: `EmptyScreenAndLoadScene,a=02` shows scene 2.
/// For tests and tools: the game's debug menus, say.
pub fn call_routine(m: &mut Machine, call: &str) -> Result<(), String> {
    let mut parts = call.split(',');
    let name = parts.next().unwrap_or_default();
    let (_, addr) = label(name).ok_or(format!("no routine {name}"))?;
    if addr >= 0x4000 {
        return Err(format!("{name} is not in bank 0"));
    }
    if !m.cpu.halted || !m.cpu.ime {
        return Err("the game is not waiting for a frame".into());
    }
    for register in parts {
        let bad = || format!("{register:?}: expected a register, '=', a hex value");
        let (r, value) = register.split_once('=').ok_or_else(bad)?;
        let value = u16::from_str_radix(value, 16).map_err(|_| bad())?;
        let c = &mut m.cpu;
        let (high, low) = ((value >> 8) as u8, value as u8);
        match r {
            "a" | "b" | "c" | "d" | "e" | "h" | "l" if value > 0xff => return Err(bad()),
            "a" => c.a = low,
            "b" => c.b = low,
            "c" => c.c = low,
            "d" => c.d = low,
            "e" => c.e = low,
            "h" => c.h = low,
            "l" => c.l = low,
            "bc" => (c.b, c.c) = (high, low),
            "de" => (c.d, c.e) = (high, low),
            "hl" => (c.h, c.l) = (high, low),
            _ => return Err(bad()),
        }
    }
    m.cpu.halted = false;
    let resume = m.cpu.pc;
    m.push(resume);
    m.cpu.pc = addr;
    m.interrupt_check();
    Ok(())
}

/// The global label at or before a ROM address, for messages.
pub fn describe(bank: u16, pc: u16) -> String {
    let bank = if pc < 0x4000 { 0 } else { bank };
    let best = generated::LABELS
        .iter()
        .filter(|&&(b, a, _)| b as u16 == bank && a <= pc && (pc < 0x4000) == (a < 0x4000))
        .max_by_key(|&&(_, a, _)| a);
    match best {
        Some(&(_, a, name)) if a == pc => format!("{bank:02x}:{pc:04x} ({name})"),
        Some(&(_, a, name)) => format!("{bank:02x}:{pc:04x} ({name}+{:#x})", pc - a),
        None => format!("{bank:02x}:{pc:04x}"),
    }
}

impl std::fmt::Display for KernelFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the game stopped at {}: {}",
            describe(self.0.bank, self.0.pc),
            self.0.reason
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelFault(pub Fault);

impl std::error::Error for KernelFault {}

/// Why the game cannot start.
#[derive(Debug)]
pub enum LoadError {
    /// Not the ROM the kernel was translated from.
    Rom(RomError),
    /// A translation that does not fit the ROM (a bug its tests catch).
    Language(tcg2_locale::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Rom(e) => e.fmt(f),
            LoadError::Language(e) => write!(f, "the translation: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// The ROM image the game runs on in `language`: the player's ROM with
/// every translation laid into the banks past it (`tcg2_locale::localize`).
pub fn localized_rom(rom: &[u8], language: Language) -> Result<Vec<u8>, tcg2_locale::Error> {
    let text_data = |name: &str| {
        generated::TEXT_DATA
            .iter()
            .find(|&&(_, _, _, n)| n == name)
            .map(|&(bank, addr, size, _)| (bank as u16, addr, size))
    };
    let symbols = tcg2_locale::Symbols {
        label: &label,
        text_data: &text_data,
    };
    tcg2_locale::localize(rom, language, &symbols)
}

/// The game: the machine and the code that runs on it.
pub struct Kernel {
    pub machine: Machine,
    pub code: Code,
    /// The player's ROM, as dumped: each language's image is made from it.
    rom: Box<[u8]>,
    language: Language,
}

impl Kernel {
    /// Boots the game from the player's ROM (checked to be the one the
    /// kernel was translated from), in `language`.
    pub fn new(rom: Vec<u8>, sample_rate: u32, language: Language) -> Result<Self, LoadError> {
        verify_rom(&rom).map_err(LoadError::Rom)?;
        let image = localized_rom(&rom, language).map_err(LoadError::Language)?;
        Ok(Self {
            machine: Machine::new(image.into(), sample_rate),
            code: Code::new(),
            rom: rom.into(),
            language,
        })
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Switches the game's text to `language`. What is on screen stays;
    /// every text printed from now on (the rest of one being printed
    /// included) is in the new language.
    pub fn set_language(&mut self, language: Language) -> Result<(), tcg2_locale::Error> {
        if language != self.language {
            let image = localized_rom(&self.rom, language)?;
            self.machine.cart.replace_rom(image.into());
            self.language = language;
        }
        Ok(())
    }

    /// Runs the game until its next frame ends (vertical blank begins, or a
    /// frame's time passes with the LCD off).
    pub fn run_frame(&mut self) -> Result<(), KernelFault> {
        let Self {
            machine: m, code, ..
        } = self;
        loop {
            if m.clock >= m.next_event {
                if let Some(fault) = m.fault.take() {
                    return Err(KernelFault(fault));
                }
                m.handle_events();
                if m.frame_ready {
                    m.frame_ready = false;
                    // The blank's interrupt is taken as the next frame starts,
                    // after the host has set that frame's input.
                    m.interrupt_check();
                    return Ok(());
                }
                m.take_interrupt();
            }
            if m.cpu.halted {
                m.sleep();
                continue;
            }
            let pc = m.cpu.pc;
            m.cpu.pc = code.dispatch(m, pc);
        }
    }

    /// The buttons held for the frames that follow (`tcg2_core::buttons`).
    pub fn set_buttons(&mut self, buttons: u8) {
        self.machine.buttons = buttons;
    }

    /// The last frame's picture: 160x144 RGBA8.
    pub fn framebuffer(&self) -> &[u8] {
        self.machine.ppu.framebuffer()
    }

    /// Whether the picture changed since the last call.
    pub fn take_picture_changed(&mut self) -> bool {
        self.machine.ppu.take_frame_changed()
    }

    /// Moves the sound rendered so far (interleaved stereo) to `out`.
    pub fn drain_audio(&mut self, out: &mut Vec<f32>) {
        self.machine.apu.drain(out);
    }

    /// The battery-backed save RAM.
    pub fn sram(&self) -> &[u8] {
        &self.machine.cart.sram
    }

    pub fn load_sram(&mut self, bytes: &[u8]) {
        self.machine.cart.load_sram(bytes);
    }

    /// Whether save RAM changed since the last call.
    pub fn take_sram_dirty(&mut self) -> bool {
        std::mem::take(&mut self.machine.cart.sram_dirty)
    }
}
