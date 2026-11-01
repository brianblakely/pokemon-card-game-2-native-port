//! Finds the program's code: recursive descent from the vectors and from
//! every label the sources mark as code, following each direct transfer and
//! the far calls' inline targets, and stopping where a call is followed by
//! data its callee consumes (`start_script`).

use crate::decode::{Insn, Op, decode};
use std::collections::{BTreeMap, BTreeSet};

pub const ROM_BANKS: usize = 128;

#[derive(Default, Debug)]
pub struct BankCode {
    pub insns: BTreeMap<u16, Insn>,
    /// Addresses a block starts at: every entry the dispatcher may resume.
    pub starts: BTreeSet<u16>,
}

#[derive(Debug, Default)]
pub struct Program {
    pub banks: Vec<BankCode>,
    pub warnings: Vec<String>,
}

pub fn bank_range(bank: usize) -> (u16, u16) {
    if bank == 0 {
        (0x0000, 0x4000)
    } else {
        (0x4000, 0x8000)
    }
}

pub fn rom_byte(rom: &[u8], bank: usize, addr: u16) -> u8 {
    let offset = if addr < 0x4000 {
        addr as usize
    } else {
        bank * 0x4000 + (addr as usize - 0x4000)
    };
    rom.get(offset).copied().unwrap_or(0xff)
}

fn fetch(rom: &[u8], bank: usize, addr: u16) -> [u8; 3] {
    [
        rom_byte(rom, bank, addr),
        rom_byte(rom, bank, addr.wrapping_add(1)),
        rom_byte(rom, bank, addr.wrapping_add(2)),
    ]
}

/// The ROM bank a transfer to `target` from code in `bank` reaches, if it
/// can be known statically.
pub fn target_bank(bank: usize, target: u16) -> Option<usize> {
    match target {
        0x0000..=0x3fff => Some(0),
        0x4000..=0x7fff if bank != 0 => Some(bank),
        _ => None,
    }
}

/// Where execution continues after an instruction returns to its caller: the
/// far calls `rst $28` (bank, address) and `rst $18` (address in bank 1)
/// return past their inline operands.
pub fn return_address(insn: &Insn, rom: &[u8], bank: usize) -> u16 {
    match insn.op {
        Op::Rst(0x28) => insn.addr.wrapping_add(4),
        Op::Rst(0x18) => insn.addr.wrapping_add(3),
        _ => {
            let _ = (rom, bank);
            insn.next()
        }
    }
}

/// The far target of `rst $28` / `rst $18`.
pub fn far_target(insn: &Insn, rom: &[u8], bank: usize) -> Option<(usize, u16)> {
    let a = insn.addr;
    match insn.op {
        Op::Rst(0x28) => {
            let target_bank = rom_byte(rom, bank, a.wrapping_add(1)) as usize;
            let target = u16::from_le_bytes([
                rom_byte(rom, bank, a.wrapping_add(2)),
                rom_byte(rom, bank, a.wrapping_add(3)),
            ]);
            Some((if target < 0x4000 { 0 } else { target_bank }, target))
        }
        Op::Rst(0x18) => {
            let target = u16::from_le_bytes([
                rom_byte(rom, bank, a.wrapping_add(1)),
                rom_byte(rom, bank, a.wrapping_add(2)),
            ]);
            Some((if target < 0x4000 { 0 } else { 1 }, target))
        }
        _ => None,
    }
}

pub struct Seeds {
    pub code: Vec<(usize, u16, String)>,
    /// Addresses where data begins (calls before them do not return there).
    pub data: BTreeSet<(usize, u16)>,
}

pub fn discover(rom: &[u8], seeds: &Seeds) -> Program {
    let mut program = Program {
        banks: (0..ROM_BANKS).map(|_| BankCode::default()).collect(),
        warnings: Vec::new(),
    };
    let code_starts: BTreeSet<(usize, u16)> = seeds.code.iter().map(|(b, a, _)| (*b, *a)).collect();
    let is_barrier = |bank: usize, addr: u16| {
        seeds.data.contains(&(bank, addr)) && !code_starts.contains(&(bank, addr))
    };
    let mut work: Vec<(usize, u16, String)> = seeds.code.iter().rev().cloned().collect();
    while let Some((bank, start, why)) = work.pop() {
        let (lo, hi) = bank_range(bank);
        if start < lo || start >= hi || bank >= ROM_BANKS {
            program.warnings.push(format!(
                "{bank:02x}:{start:04x} ({why}) is outside its bank's code space"
            ));
            continue;
        }
        if program.banks[bank].insns.contains_key(&start) {
            program.banks[bank].starts.insert(start);
            continue;
        }
        program.banks[bank].starts.insert(start);
        let mut addr = start;
        loop {
            if program.banks[bank].insns.contains_key(&addr) {
                break;
            }
            if addr < lo || addr >= hi {
                program.warnings.push(format!(
                    "code from {bank:02x}:{start:04x} ({why}) runs off its bank at {addr:04x}"
                ));
                break;
            }
            let insn = decode(addr, fetch(rom, bank, addr));
            program.banks[bank].insns.insert(addr, insn);
            let mut targets: Vec<(usize, u16)> = Vec::new();
            match insn.op {
                Op::Jp(_, t) | Op::Jr(_, t) | Op::Call(_, t) => {
                    if let Some(b) = target_bank(bank, t) {
                        targets.push((b, t));
                    }
                }
                Op::Rst(n) => {
                    targets.push((0, n as u16));
                    if let Some(far) = far_target(&insn, rom, bank) {
                        targets.push(far);
                    }
                }
                Op::Invalid(op) => program.warnings.push(format!(
                    "invalid opcode {op:02x} at {bank:02x}:{addr:04x} (from {start:04x}, {why})"
                )),
                _ => {}
            }
            for (b, t) in targets {
                let (tlo, thi) = bank_range(b);
                if t >= tlo && t < thi {
                    work.push((b, t, format!("target of {bank:02x}:{addr:04x}")));
                }
            }
            if insn.is_unconditional_exit() {
                break;
            }
            let next = return_address(&insn, rom, bank);
            if insn.ends_block() {
                let returns_here = matches!(insn.op, Op::Call(..) | Op::Rst(_));
                if returns_here && is_barrier(bank, next) {
                    // The callee consumes the data after the call.
                    break;
                }
                work.push((bank, next, format!("after {bank:02x}:{addr:04x}")));
                break;
            }
            if is_barrier(bank, next) {
                program.warnings.push(format!(
                    "{bank:02x}:{addr:04x} falls through into data at {next:04x}; translating it as code"
                ));
            }
            addr = next;
        }
    }
    program
}

/// A block: straight-line instructions from `start`.
#[derive(Debug)]
pub struct Block {
    pub start: u16,
    pub insns: Vec<Insn>,
    /// Where control goes if the last instruction does not transfer it.
    pub fallthrough: Option<u16>,
}

pub fn blocks(code: &BankCode) -> Vec<Block> {
    let mut out = Vec::new();
    for &start in &code.starts {
        let mut insns = Vec::new();
        let mut addr = start;
        let mut fallthrough = None;
        while let Some(insn) = code.insns.get(&addr) {
            insns.push(*insn);
            if insn.ends_block() || insn.is_unconditional_exit() {
                break;
            }
            addr = insn.next();
            if code.starts.contains(&addr) || !code.insns.contains_key(&addr) {
                fallthrough = Some(addr);
                break;
            }
        }
        out.push(Block {
            start,
            insns,
            fallthrough,
        });
    }
    out
}
