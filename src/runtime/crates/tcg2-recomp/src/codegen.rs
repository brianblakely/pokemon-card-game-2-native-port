//! Writes the program as Rust: one function per global label of each bank,
//! a `match` over its blocks inside a loop, every instruction as the Rust
//! statement that does what it does to the machine (with the original
//! instruction, symbolised, as its comment).
//!
//! Transfers within a function stay in its loop; anything else (calls,
//! returns, computed jumps, other functions, other banks) returns the next
//! address to the kernel's dispatcher, which also takes interrupts and ends
//! frames. Every block start is an entry the dispatcher can resume.

use crate::decode::{Alu, Cb, Cond, Insn, Op, R8, R16};
use crate::discover::{Block, Program, blocks, far_target, rom_byte, target_bank};
use crate::sym::Symbols;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

pub struct Function {
    pub name: String,
    pub label: String,
    pub location: Option<(String, usize)>,
    pub blocks: Vec<Block>,
}

pub fn rust_ident(label: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in label.chars() {
        if c.is_ascii_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = false;
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("f_{out}")
    } else {
        out
    }
}

/// Groups a bank's blocks by the global label each lies under.
pub fn functions(
    program: &Program,
    bank: usize,
    sym: &Symbols,
    locations: &HashMap<String, (String, usize)>,
) -> Vec<Function> {
    let mut groups: BTreeMap<(u16, String), Vec<Block>> = BTreeMap::new();
    for block in blocks(&program.banks[bank]) {
        let (addr, label) = if bank == 0 && block.start < 0x0150 {
            (0x0000, "Vectors".to_string())
        } else {
            match sym.owner(bank as u16, block.start) {
                Some(owner) => (owner.addr, owner.name.clone()),
                None => (block.start, format!("Code_{bank:02x}_{:04x}", block.start)),
            }
        };
        groups.entry((addr, label)).or_default().push(block);
    }
    groups
        .into_iter()
        .map(|((addr, label), blocks)| Function {
            name: format!("{}_{bank:02x}{addr:04x}", rust_ident(&label)),
            location: locations.get(&label).cloned(),
            label,
            blocks,
        })
        .collect()
}

const IO_NAMES: &[(u8, &str)] = &[
    (0x00, "rJOYP"),
    (0x01, "rSB"),
    (0x02, "rSC"),
    (0x04, "rDIV"),
    (0x05, "rTIMA"),
    (0x06, "rTMA"),
    (0x07, "rTAC"),
    (0x0f, "rIF"),
    (0x10, "rNR10"),
    (0x11, "rNR11"),
    (0x12, "rNR12"),
    (0x13, "rNR13"),
    (0x14, "rNR14"),
    (0x16, "rNR21"),
    (0x17, "rNR22"),
    (0x18, "rNR23"),
    (0x19, "rNR24"),
    (0x1a, "rNR30"),
    (0x1b, "rNR31"),
    (0x1c, "rNR32"),
    (0x1d, "rNR33"),
    (0x1e, "rNR34"),
    (0x20, "rNR41"),
    (0x21, "rNR42"),
    (0x22, "rNR43"),
    (0x23, "rNR44"),
    (0x24, "rNR50"),
    (0x25, "rNR51"),
    (0x26, "rNR52"),
    (0x40, "rLCDC"),
    (0x41, "rSTAT"),
    (0x42, "rSCY"),
    (0x43, "rSCX"),
    (0x44, "rLY"),
    (0x45, "rLYC"),
    (0x46, "rDMA"),
    (0x47, "rBGP"),
    (0x48, "rOBP0"),
    (0x49, "rOBP1"),
    (0x4a, "rWY"),
    (0x4b, "rWX"),
    (0x4d, "rSPD"),
    (0x4f, "rVBK"),
    (0x51, "rVDMA_SRC_HIGH"),
    (0x52, "rVDMA_SRC_LOW"),
    (0x53, "rVDMA_DEST_HIGH"),
    (0x54, "rVDMA_DEST_LOW"),
    (0x55, "rVDMA_LEN"),
    (0x56, "rRP"),
    (0x68, "rBGPI"),
    (0x69, "rBGPD"),
    (0x6a, "rOBPI"),
    (0x6b, "rOBPD"),
    (0x70, "rWBK"),
    (0xff, "rIE"),
];

struct Gen<'a> {
    rom: &'a [u8],
    sym: &'a Symbols,
    bank: usize,
    /// The block starts of the function being written.
    local: BTreeSet<u16>,
    out: String,
    pending: u32,
}

fn reg(r: R8) -> &'static str {
    match r {
        R8::B => "m.cpu.b",
        R8::C => "m.cpu.c",
        R8::D => "m.cpu.d",
        R8::E => "m.cpu.e",
        R8::H => "m.cpu.h",
        R8::L => "m.cpu.l",
        R8::A => "m.cpu.a",
        R8::MemHl => unreachable!("[hl] is a memory access"),
    }
}

fn pair(rr: R16, af: bool) -> &'static str {
    match rr {
        R16::Bc => "bc",
        R16::De => "de",
        R16::Hl => "hl",
        R16::SpOrAf => {
            if af {
                "af"
            } else {
                "sp"
            }
        }
    }
}

fn cond(c: Cond) -> &'static str {
    match c {
        Cond::Nz => "!m.cpu.zf()",
        Cond::Z => "m.cpu.zf()",
        Cond::Nc => "!m.cpu.cf()",
        Cond::C => "m.cpu.cf()",
    }
}

fn alu(op: Alu) -> &'static str {
    op.name()
}

fn cb_name(op: Cb) -> &'static str {
    match op {
        Cb::Rlc => "rlc",
        Cb::Rrc => "rrc",
        Cb::Rl => "rl",
        Cb::Rr => "rr",
        Cb::Sla => "sla",
        Cb::Sra => "sra",
        Cb::Swap => "swap",
        Cb::Srl => "srl",
        Cb::Bit(_) => "bit",
        Cb::Res(_) => "res",
        Cb::Set(_) => "set",
    }
}

impl<'a> Gen<'a> {
    fn line(&mut self, text: &str) {
        self.out.push_str("                ");
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// Adds `cycles` and brings the clock up to date before an access whose
    /// timing a device observes.
    fn timed(&mut self, cycles: u32) {
        let n = self.pending + cycles;
        if n > 0 {
            self.line(&format!("m.tick({n});"));
        }
        self.pending = 0;
    }

    fn rom_name(&self, bank: Option<usize>, addr: u16) -> String {
        bank.and_then(|b| self.sym.name_at(b as u16, addr).map(str::to_string))
            .unwrap_or_else(|| format!("${addr:04x}"))
    }

    fn mem_name(&self, addr: u16) -> String {
        if addr >= 0xff00
            && let Some((_, name)) = IO_NAMES.iter().find(|(r, _)| 0xff00 | *r as u16 == addr)
        {
            return name.to_string();
        }
        if addr < 0x8000 {
            return self.rom_name(target_bank(self.bank, addr), addr);
        }
        self.sym
            .ram_name(addr)
            .map(str::to_string)
            .unwrap_or_else(|| format!("${addr:04x}"))
    }

    fn disasm(&self, insn: &Insn) -> String {
        let r = |r: R8| r.name();
        match insn.op {
            Op::Nop => "nop".into(),
            Op::Stop => "stop".into(),
            Op::Halt => "halt".into(),
            Op::Di => "di".into(),
            Op::Ei => "ei".into(),
            Op::Ld(d, s) => format!("ld {}, {}", r(d), r(s)),
            Op::LdImm(d, n) => format!("ld {}, ${n:02x}", r(d)),
            Op::LdAInd(rr) => format!("ld a, [{}]", pair(rr, false)),
            Op::LdIndA(rr) => format!("ld [{}], a", pair(rr, false)),
            Op::LdAHli => "ld a, [hli]".into(),
            Op::LdAHld => "ld a, [hld]".into(),
            Op::LdHliA => "ld [hli], a".into(),
            Op::LdHldA => "ld [hld], a".into(),
            Op::LdAMem(nn) => format!("ld a, [{}]", self.mem_name(nn)),
            Op::LdMemA(nn) => format!("ld [{}], a", self.mem_name(nn)),
            Op::LdhAN(n) => format!("ldh a, [{}]", self.mem_name(0xff00 | n as u16)),
            Op::LdhNA(n) => format!("ldh [{}], a", self.mem_name(0xff00 | n as u16)),
            Op::LdhAC => "ld a, [$ff00+c]".into(),
            Op::LdhCA => "ld [$ff00+c], a".into(),
            Op::Ld16(rr, nn) => match self.sym.ram_name(nn).filter(|_| nn >= 0x8000) {
                // A RAM address with exactly one name is almost surely that
                // variable; ROM-range values are often plain numbers.
                Some(name) => format!("ld {}, {name}", pair(rr, false)),
                None => format!("ld {}, ${nn:04x}", pair(rr, false)),
            },
            Op::LdMemSp(nn) => format!("ld [{}], sp", self.mem_name(nn)),
            Op::LdSpHl => "ld sp, hl".into(),
            Op::LdHlSpE(e) => format!("ld hl, sp{e:+}"),
            Op::Push(rr) => format!("push {}", pair(rr, true)),
            Op::Pop(rr) => format!("pop {}", pair(rr, true)),
            Op::Alu(op, s) => format!("{} {}", op.name(), r(s)),
            Op::AluImm(op, n) => format!("{} ${n:02x}", op.name()),
            Op::Inc(x) => format!("inc {}", r(x)),
            Op::Dec(x) => format!("dec {}", r(x)),
            Op::Inc16(rr) => format!("inc {}", pair(rr, false)),
            Op::Dec16(rr) => format!("dec {}", pair(rr, false)),
            Op::AddHl(rr) => format!("add hl, {}", pair(rr, false)),
            Op::AddSpE(e) => format!("add sp, {e}"),
            Op::Rlca => "rlca".into(),
            Op::Rla => "rla".into(),
            Op::Rrca => "rrca".into(),
            Op::Rra => "rra".into(),
            Op::Daa => "daa".into(),
            Op::Cpl => "cpl".into(),
            Op::Scf => "scf".into(),
            Op::Ccf => "ccf".into(),
            Op::Cb(op, x) => match op {
                Cb::Bit(n) | Cb::Res(n) | Cb::Set(n) => format!("{} {n}, {}", cb_name(op), r(x)),
                _ => format!("{} {}", cb_name(op), r(x)),
            },
            Op::Jp(c, t) => self.transfer("jp", c, t),
            Op::Jr(c, t) => self.transfer("jr", c, t),
            Op::Call(c, t) => self.transfer("call", c, t),
            Op::JpHl => "jp hl".into(),
            Op::Ret(None) => "ret".into(),
            Op::Ret(Some(c)) => format!("ret {}", c.name()),
            Op::Reti => "reti".into(),
            Op::Rst(n) => match far_target(insn, self.rom, self.bank) {
                Some((b, t)) if n == 0x28 => {
                    format!("farcall {} (rst $28)", self.rom_name(Some(b), t))
                }
                Some((b, t)) => format!("bank1call {} (rst $18)", self.rom_name(Some(b), t)),
                None => format!("rst ${n:02x}"),
            },
            Op::Invalid(op) => format!("db ${op:02x} (not an instruction)"),
        }
    }

    fn transfer(&self, mnemonic: &str, c: Option<Cond>, t: u16) -> String {
        let name = self.rom_name(target_bank(self.bank, t), t);
        match c {
            Some(c) => format!("{mnemonic} {}, {name}", c.name()),
            None => format!("{mnemonic} {name}"),
        }
    }

    fn is_local(&self, target: u16) -> bool {
        target_bank(self.bank, target) == Some(self.bank) && self.local.contains(&target)
    }

    fn goto(&self, target: u16) -> String {
        if self.is_local(target) {
            format!("local!(m, pc, 0x{target:04x});")
        } else {
            format!("return 0x{target:04x};")
        }
    }

    fn read_static(&self, nn: u16) -> (String, bool) {
        match nn {
            0x0000..=0x3fff => (format!("m.cart.rom[0x{nn:04x}]"), false),
            0x4000..=0x7fff => (
                format!("m.cart.rom[m.cart.rom_offset + 0x{:04x}]", nn - 0x4000),
                false,
            ),
            0xc000..=0xcfff => (format!("m.wram[0x{:04x}]", nn - 0xc000), false),
            0xd000..=0xdfff => (
                format!("m.wram[m.wram_offset + 0x{:04x}]", nn - 0xd000),
                false,
            ),
            0xff80..=0xfffe => (format!("m.hram[0x{:02x}]", nn - 0xff80), false),
            _ => (format!("m.read(0x{nn:04x})"), true),
        }
    }

    fn write_static(&self, nn: u16, value: &str) -> (String, bool) {
        match nn {
            0xc000..=0xcfff => (format!("m.wram[0x{:04x}] = {value};", nn - 0xc000), false),
            0xd000..=0xdfff => (
                format!("m.wram[m.wram_offset + 0x{:04x}] = {value};", nn - 0xd000),
                false,
            ),
            0xff80..=0xfffe => (format!("m.hram[0x{:02x}] = {value};", nn - 0xff80), false),
            _ => (format!("m.write(0x{nn:04x}, {value});"), true),
        }
    }

    /// A read-modify-write of `[hl]`.
    fn rmw(&mut self, cycles: u32, modify: &str) {
        self.timed(cycles);
        self.line(&format!(
            "{{ let hl = m.cpu.hl(); let v = m.read(hl); {modify} m.write(hl, v); }}"
        ));
    }

    fn straight(&mut self, insn: &Insn) {
        let c = insn.cycles;
        match insn.op {
            Op::Nop => self.pending += c,
            Op::Ld(d, s) => {
                if d == R8::MemHl {
                    self.timed(c);
                    self.line(&format!("m.write(m.cpu.hl(), {});", reg(s)));
                } else if s == R8::MemHl {
                    self.timed(c);
                    self.line(&format!("{} = m.read(m.cpu.hl());", reg(d)));
                } else {
                    self.pending += c;
                    if d != s {
                        self.line(&format!("{} = {};", reg(d), reg(s)));
                    }
                }
            }
            Op::LdImm(d, n) => {
                if d == R8::MemHl {
                    self.timed(c);
                    self.line(&format!("m.write(m.cpu.hl(), 0x{n:02x});"));
                } else {
                    self.pending += c;
                    self.line(&format!("{} = 0x{n:02x};", reg(d)));
                }
            }
            Op::LdAInd(rr) => {
                self.timed(c);
                self.line(&format!("m.cpu.a = m.read(m.cpu.{}());", pair(rr, false)));
            }
            Op::LdIndA(rr) => {
                self.timed(c);
                self.line(&format!("m.write(m.cpu.{}(), m.cpu.a);", pair(rr, false)));
            }
            Op::LdAHli | Op::LdAHld => {
                self.timed(c);
                self.line("m.cpu.a = m.read(m.cpu.hl());");
                let step = if insn.op == Op::LdAHli { "add" } else { "sub" };
                self.line(&format!("m.cpu.set_hl(m.cpu.hl().wrapping_{step}(1));"));
            }
            Op::LdHliA | Op::LdHldA => {
                self.timed(c);
                self.line("m.write(m.cpu.hl(), m.cpu.a);");
                let step = if insn.op == Op::LdHliA { "add" } else { "sub" };
                self.line(&format!("m.cpu.set_hl(m.cpu.hl().wrapping_{step}(1));"));
            }
            Op::LdAMem(nn) => {
                let (expr, timed) = self.read_static(nn);
                if timed {
                    self.timed(c)
                } else {
                    self.pending += c
                }
                self.line(&format!("m.cpu.a = {expr};"));
            }
            Op::LdMemA(nn) => {
                let (stmt, timed) = self.write_static(nn, "m.cpu.a");
                if timed {
                    self.timed(c)
                } else {
                    self.pending += c
                }
                self.line(&stmt);
            }
            Op::LdhAN(n) => {
                if n >= 0x80 && n != 0xff {
                    self.pending += c;
                    self.line(&format!("m.cpu.a = m.hram[0x{:02x}];", n - 0x80));
                } else {
                    self.timed(c);
                    self.line(&format!("m.cpu.a = m.read_ff(0x{n:02x});"));
                }
            }
            Op::LdhNA(n) => {
                if n >= 0x80 && n != 0xff {
                    self.pending += c;
                    self.line(&format!("m.hram[0x{:02x}] = m.cpu.a;", n - 0x80));
                } else {
                    self.timed(c);
                    self.line(&format!("m.write_ff(0x{n:02x}, m.cpu.a);"));
                }
            }
            Op::LdhAC => {
                self.timed(c);
                self.line("m.cpu.a = m.read_ff(m.cpu.c);");
            }
            Op::LdhCA => {
                self.timed(c);
                self.line("m.write_ff(m.cpu.c, m.cpu.a);");
            }
            Op::Ld16(rr, nn) => {
                self.pending += c;
                match rr {
                    R16::SpOrAf => self.line(&format!("m.cpu.sp = 0x{nn:04x};")),
                    _ => self.line(&format!("m.cpu.set_{}(0x{nn:04x});", pair(rr, false))),
                }
            }
            Op::LdMemSp(nn) => {
                self.timed(c);
                self.line(&format!(
                    "{{ let sp = m.cpu.sp; m.write(0x{nn:04x}, sp as u8); m.write(0x{:04x}, (sp >> 8) as u8); }}",
                    nn.wrapping_add(1)
                ));
            }
            Op::LdSpHl => {
                self.pending += c;
                self.line("m.cpu.sp = m.cpu.hl();");
            }
            Op::LdHlSpE(e) => {
                self.pending += c;
                self.line(&format!(
                    "{{ let v = m.cpu.sp_offset({e}); m.cpu.set_hl(v); }}"
                ));
            }
            Op::Push(rr) => {
                self.pending += c;
                self.line(&format!("m.push(m.cpu.{}());", pair(rr, true)));
            }
            Op::Pop(rr) => {
                self.pending += c;
                self.line(&format!(
                    "{{ let v = m.pop(); m.cpu.set_{}(v); }}",
                    pair(rr, true)
                ));
            }
            Op::Alu(op, s) => {
                if s == R8::MemHl {
                    self.timed(c);
                    self.line(&format!(
                        "{{ let v = m.read(m.cpu.hl()); m.cpu.{}(v); }}",
                        alu(op)
                    ));
                } else {
                    self.pending += c;
                    self.line(&format!("m.cpu.{}({});", alu(op), reg(s)));
                }
            }
            Op::AluImm(op, n) => {
                self.pending += c;
                self.line(&format!("m.cpu.{}(0x{n:02x});", alu(op)));
            }
            Op::Inc(x) | Op::Dec(x) => {
                let f = if matches!(insn.op, Op::Inc(_)) {
                    "inc8"
                } else {
                    "dec8"
                };
                if x == R8::MemHl {
                    self.rmw(c, &format!("let v = m.cpu.{f}(v);"));
                } else {
                    self.pending += c;
                    self.line(&format!("{} = m.cpu.{f}({});", reg(x), reg(x)));
                }
            }
            Op::Inc16(rr) | Op::Dec16(rr) => {
                self.pending += c;
                let step = if matches!(insn.op, Op::Inc16(_)) {
                    "add"
                } else {
                    "sub"
                };
                match rr {
                    R16::SpOrAf => self.line(&format!("m.cpu.sp = m.cpu.sp.wrapping_{step}(1);")),
                    _ => {
                        let p = pair(rr, false);
                        self.line(&format!("m.cpu.set_{p}(m.cpu.{p}().wrapping_{step}(1));"))
                    }
                }
            }
            Op::AddHl(rr) => {
                self.pending += c;
                match rr {
                    R16::SpOrAf => self.line("m.cpu.add_hl(m.cpu.sp);"),
                    _ => self.line(&format!("m.cpu.add_hl(m.cpu.{}());", pair(rr, false))),
                }
            }
            Op::AddSpE(e) => {
                self.pending += c;
                self.line(&format!("m.cpu.sp = m.cpu.sp_offset({e});"));
            }
            Op::Rlca | Op::Rla | Op::Rrca | Op::Rra | Op::Daa | Op::Cpl | Op::Scf | Op::Ccf => {
                self.pending += c;
                let name = self.disasm(insn);
                self.line(&format!("m.cpu.{name}();"));
            }
            Op::Cb(op, x) => {
                let bit = match op {
                    Cb::Bit(n) | Cb::Res(n) | Cb::Set(n) => 1u8 << n,
                    _ => 0,
                };
                match (op, x) {
                    (Cb::Bit(n), R8::MemHl) => {
                        self.timed(c);
                        self.line(&format!(
                            "{{ let v = m.read(m.cpu.hl()); m.cpu.bit({n}, v); }}"
                        ));
                    }
                    (Cb::Bit(n), _) => {
                        self.pending += c;
                        self.line(&format!("m.cpu.bit({n}, {});", reg(x)));
                    }
                    (Cb::Res(_), R8::MemHl) => self.rmw(c, &format!("let v = v & 0x{:02x};", !bit)),
                    (Cb::Set(_), R8::MemHl) => self.rmw(c, &format!("let v = v | 0x{bit:02x};")),
                    (Cb::Res(_), _) => {
                        self.pending += c;
                        self.line(&format!("{} &= 0x{:02x};", reg(x), !bit));
                    }
                    (Cb::Set(_), _) => {
                        self.pending += c;
                        self.line(&format!("{} |= 0x{bit:02x};", reg(x)));
                    }
                    (_, R8::MemHl) => self.rmw(c, &format!("let v = m.cpu.{}(v);", cb_name(op))),
                    _ => {
                        self.pending += c;
                        self.line(&format!("{} = m.cpu.{}({});", reg(x), cb_name(op), reg(x)));
                    }
                }
            }
            Op::Di => {
                self.pending += c;
                self.line("m.di();");
            }
            Op::Ei => {
                self.pending += c;
                self.line("m.ei();");
            }
            _ => unreachable!("{:?} ends a block", insn.op),
        }
    }

    fn terminator(&mut self, insn: &Insn) {
        let c = insn.cycles;
        let p = self.pending;
        self.pending = 0;
        let next = insn.next();
        match insn.op {
            Op::Jp(None, t) | Op::Jr(None, t) => {
                self.line(&format!("m.tick({});", p + c));
                let g = self.goto(t);
                self.line(&g);
            }
            Op::Jp(Some(k), t) | Op::Jr(Some(k), t) => {
                let g = self.goto(t);
                self.line(&format!(
                    "if {} {{ m.tick({}); {g} }}",
                    cond(k),
                    p + c + insn.taken
                ));
                self.line(&format!("m.tick({});", p + c));
                let g = self.goto(next);
                self.line(&g);
            }
            Op::Call(None, t) => {
                self.line(&format!("m.tick({});", p + c));
                self.line(&format!("m.push(0x{next:04x});"));
                let g = self.goto(t);
                self.line(&g);
            }
            Op::Call(Some(k), t) => {
                let g = self.goto(t);
                self.line(&format!(
                    "if {} {{ m.tick({}); m.push(0x{next:04x}); {g} }}",
                    cond(k),
                    p + c + insn.taken
                ));
                self.line(&format!("m.tick({});", p + c));
                let g = self.goto(next);
                self.line(&g);
            }
            Op::Ret(None) => {
                self.line(&format!("m.tick({});", p + c));
                self.line("return m.pop();");
            }
            Op::Ret(Some(k)) => {
                self.line(&format!(
                    "if {} {{ m.tick({}); return m.pop(); }}",
                    cond(k),
                    p + c + insn.taken
                ));
                self.line(&format!("m.tick({});", p + c));
                let g = self.goto(next);
                self.line(&g);
            }
            Op::Reti => {
                self.line(&format!("m.tick({});", p + c));
                self.line("m.ei();");
                self.line("return m.pop();");
            }
            Op::JpHl => {
                self.line(&format!("m.tick({});", p + c));
                self.line("return m.cpu.hl();");
            }
            Op::Rst(n) => {
                // The CPU pushes the address after the `rst`; the far-call
                // handlers read their operands there and return past them.
                self.line(&format!("m.tick({});", p + c));
                self.line(&format!("m.push(0x{next:04x});"));
                self.line(&format!("return 0x{:04x};", n as u16));
            }
            Op::Halt => {
                self.line(&format!("m.tick({});", p + c));
                self.line("m.halt();");
                self.line(&format!("return 0x{next:04x};"));
            }
            Op::Stop => {
                self.line(&format!("m.tick({});", p + c));
                self.line("m.stop();");
                self.line(&format!("return 0x{:04x};", insn.addr.wrapping_add(2)));
            }
            Op::Invalid(_) => {
                self.line(&format!("m.tick({});", p + c));
                self.line(&format!(
                    "return crate::invalid(m, 0x{:02x}, 0x{:04x});",
                    self.bank, insn.addr
                ));
            }
            _ => unreachable!(),
        }
    }

    fn block(&mut self, block: &Block) {
        let _ = writeln!(self.out, "            0x{:04x} => {{", block.start);
        let labels: Vec<String> = self
            .sym
            .at(self.bank as u16, block.start)
            .map(|s| s.name.clone())
            .collect();
        for label in labels {
            if !label.contains(".tcg2_") {
                self.line(&format!("// {label}:"));
            }
        }
        self.pending = 0;
        for insn in &block.insns {
            let text = self.disasm(insn);
            self.line(&format!("// {text}"));
            if insn.switches_rom_bank() {
                // The store, then the dispatcher continues in the new bank.
                self.straight(insn);
                let p = std::mem::take(&mut self.pending);
                if p > 0 {
                    self.line(&format!("m.tick({p});"));
                }
                self.line(&format!("return 0x{:04x};", insn.next()));
            } else if insn.ends_block() || insn.is_unconditional_exit() {
                self.terminator(insn);
            } else {
                self.straight(insn);
            }
        }
        if let Some(next) = block.fallthrough {
            let p = std::mem::take(&mut self.pending);
            if p > 0 {
                self.line(&format!("m.tick({p});"));
            }
            let g = self.goto(next);
            self.line(&g);
        }
        self.out.push_str("            }\n");
    }
}

/// Writes one bank's module: its functions and its dispatch entries.
pub fn bank_module(rom: &[u8], sym: &Symbols, bank: usize, functions: &[Function]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// Bank ${bank:02x} of the program, translated from src/decomp by tcg2-recomp."
    );
    let _ = writeln!(out, "// Generated; do not edit.\n");
    out.push_str("use tcg2_core::Machine;\n\n");
    out.push_str(
        "macro_rules! local {\n    ($m:ident, $pc:ident, $t:expr) => {{\n        $pc = $t;\n        if $m.clock >= $m.next_event {\n            return $pc;\n        }\n        continue;\n    }};\n}\n\n",
    );
    for f in functions {
        match &f.location {
            Some((file, line)) => {
                let _ = writeln!(out, "/// `{}` (src/decomp/src/{file}:{line})", f.label);
            }
            None => {
                let _ = writeln!(out, "/// `{}`", f.label);
            }
        }
        let _ = writeln!(
            out,
            "pub fn {}(m: &mut Machine, mut pc: u16) -> u16 {{",
            f.name
        );
        out.push_str("    loop {\n        match pc {\n");
        let mut g = Gen {
            rom,
            sym,
            bank,
            local: f.blocks.iter().map(|b| b.start).collect(),
            out: String::new(),
            pending: 0,
        };
        for block in &f.blocks {
            g.block(block);
        }
        out.push_str(&g.out);
        let _ = writeln!(
            out,
            "            _ => return crate::lost(m, 0x{bank:02x}, pc),"
        );
        out.push_str("        }\n    }\n}\n\n");
    }
    out.push_str("/// Every block start of this bank and the function that runs it.\n");
    out.push_str("pub static ENTRIES: &[(u16, super::Func)] = &[\n");
    for f in functions {
        for b in &f.blocks {
            let _ = writeln!(out, "    (0x{:04x}, {}),", b.start, f.name);
        }
    }
    out.push_str("];\n");
    let _ = rom_byte;
    out
}
