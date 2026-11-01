//! The SM83 instruction set: decoding, cycle counts and control flow.

/// 8-bit operands in opcode order; `MemHl` is `[hl]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R8 {
    B,
    C,
    D,
    E,
    H,
    L,
    MemHl,
    A,
}

impl R8 {
    fn from(bits: u8) -> Self {
        [R8::B, R8::C, R8::D, R8::E, R8::H, R8::L, R8::MemHl, R8::A][(bits & 7) as usize]
    }
    pub fn name(self) -> &'static str {
        ["b", "c", "d", "e", "h", "l", "[hl]", "a"][self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R16 {
    Bc,
    De,
    Hl,
    /// SP in `ld rr, nn` / `inc` / `dec` / `add hl`; AF in `push` / `pop`.
    SpOrAf,
}

impl R16 {
    fn from(bits: u8) -> Self {
        [R16::Bc, R16::De, R16::Hl, R16::SpOrAf][(bits & 3) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alu {
    Add,
    Adc,
    Sub,
    Sbc,
    And,
    Xor,
    Or,
    Cp,
}

impl Alu {
    fn from(bits: u8) -> Self {
        [
            Alu::Add,
            Alu::Adc,
            Alu::Sub,
            Alu::Sbc,
            Alu::And,
            Alu::Xor,
            Alu::Or,
            Alu::Cp,
        ][(bits & 7) as usize]
    }
    pub fn name(self) -> &'static str {
        ["add", "adc", "sub", "sbc", "and", "xor", "or", "cp"][self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    Nz,
    Z,
    Nc,
    C,
}

impl Cond {
    fn from(bits: u8) -> Self {
        [Cond::Nz, Cond::Z, Cond::Nc, Cond::C][(bits & 3) as usize]
    }
    pub fn name(self) -> &'static str {
        ["nz", "z", "nc", "c"][self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cb {
    Rlc,
    Rrc,
    Rl,
    Rr,
    Sla,
    Sra,
    Swap,
    Srl,
    Bit(u8),
    Res(u8),
    Set(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Nop,
    Stop,
    Halt,
    Di,
    Ei,
    Ld(R8, R8),
    LdImm(R8, u8),
    /// `ld a, [bc]` / `ld a, [de]`.
    LdAInd(R16),
    LdIndA(R16),
    LdAHli,
    LdAHld,
    LdHliA,
    LdHldA,
    LdAMem(u16),
    LdMemA(u16),
    LdhAN(u8),
    LdhNA(u8),
    LdhAC,
    LdhCA,
    Ld16(R16, u16),
    LdMemSp(u16),
    LdSpHl,
    LdHlSpE(i8),
    Push(R16),
    Pop(R16),
    Alu(Alu, R8),
    AluImm(Alu, u8),
    Inc(R8),
    Dec(R8),
    Inc16(R16),
    Dec16(R16),
    AddHl(R16),
    AddSpE(i8),
    Rlca,
    Rla,
    Rrca,
    Rra,
    Daa,
    Cpl,
    Scf,
    Ccf,
    Cb(Cb, R8),
    Jp(Option<Cond>, u16),
    JpHl,
    Jr(Option<Cond>, u16),
    Call(Option<Cond>, u16),
    Ret(Option<Cond>),
    Reti,
    Rst(u8),
    Invalid(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub addr: u16,
    pub op: Op,
    pub len: u16,
    /// M-cycles when a conditional transfer is not taken (or always).
    pub cycles: u32,
    /// Extra M-cycles when a conditional transfer is taken.
    pub taken: u32,
}

impl Insn {
    /// A store to the MBC's ROM bank registers: in bank 1+ code, the next
    /// instruction is fetched from the new bank.
    pub fn switches_rom_bank(&self) -> bool {
        self.addr >= 0x4000 && matches!(self.op, Op::LdMemA(0x2000..=0x3fff))
    }

    pub fn next(&self) -> u16 {
        self.addr.wrapping_add(self.len)
    }

    /// Ends straight-line flow: nothing falls through to the next address.
    pub fn is_unconditional_exit(&self) -> bool {
        matches!(
            self.op,
            Op::Jp(None, _)
                | Op::JpHl
                | Op::Jr(None, _)
                | Op::Ret(None)
                | Op::Reti
                | Op::Invalid(_)
        )
    }

    /// Ends a block (control may leave the straight line here).
    pub fn ends_block(&self) -> bool {
        self.switches_rom_bank()
            || matches!(
                self.op,
                Op::Jp(..)
                    | Op::JpHl
                    | Op::Jr(..)
                    | Op::Call(..)
                    | Op::Ret(..)
                    | Op::Reti
                    | Op::Rst(_)
                    | Op::Halt
                    | Op::Stop
                    | Op::Invalid(_)
            )
    }
}

/// Decodes the instruction at `addr` from `bytes` (at least three bytes;
/// past the end of the code reads as $FF).
pub fn decode(addr: u16, bytes: [u8; 3]) -> Insn {
    let [op, b1, b2] = bytes;
    let nn = u16::from_le_bytes([b1, b2]);
    let rel = addr.wrapping_add(2).wrapping_add(b1 as i8 as i16 as u16);
    let hl_cost = |r: R8, base: u32, mem: u32| if r == R8::MemHl { mem } else { base };
    let (o, len, cycles, taken) = match op {
        0x00 => (Op::Nop, 1, 1, 0),
        0x10 => (Op::Stop, 2, 1, 0),
        0x76 => (Op::Halt, 1, 1, 0),
        0xf3 => (Op::Di, 1, 1, 0),
        0xfb => (Op::Ei, 1, 1, 0),
        0x01 | 0x11 | 0x21 | 0x31 => (Op::Ld16(R16::from(op >> 4), nn), 3, 3, 0),
        0x02 => (Op::LdIndA(R16::Bc), 1, 2, 0),
        0x12 => (Op::LdIndA(R16::De), 1, 2, 0),
        0x22 => (Op::LdHliA, 1, 2, 0),
        0x32 => (Op::LdHldA, 1, 2, 0),
        0x0a => (Op::LdAInd(R16::Bc), 1, 2, 0),
        0x1a => (Op::LdAInd(R16::De), 1, 2, 0),
        0x2a => (Op::LdAHli, 1, 2, 0),
        0x3a => (Op::LdAHld, 1, 2, 0),
        0x03 | 0x13 | 0x23 | 0x33 => (Op::Inc16(R16::from(op >> 4)), 1, 2, 0),
        0x0b | 0x1b | 0x2b | 0x3b => (Op::Dec16(R16::from(op >> 4)), 1, 2, 0),
        0x09 | 0x19 | 0x29 | 0x39 => (Op::AddHl(R16::from(op >> 4)), 1, 2, 0),
        _ if op & 0xc7 == 0x04 => {
            let r = R8::from(op >> 3);
            (Op::Inc(r), 1, hl_cost(r, 1, 3), 0)
        }
        _ if op & 0xc7 == 0x05 => {
            let r = R8::from(op >> 3);
            (Op::Dec(r), 1, hl_cost(r, 1, 3), 0)
        }
        _ if op & 0xc7 == 0x06 => {
            let r = R8::from(op >> 3);
            (Op::LdImm(r, b1), 2, hl_cost(r, 2, 3), 0)
        }
        0x07 => (Op::Rlca, 1, 1, 0),
        0x0f => (Op::Rrca, 1, 1, 0),
        0x17 => (Op::Rla, 1, 1, 0),
        0x1f => (Op::Rra, 1, 1, 0),
        0x27 => (Op::Daa, 1, 1, 0),
        0x2f => (Op::Cpl, 1, 1, 0),
        0x37 => (Op::Scf, 1, 1, 0),
        0x3f => (Op::Ccf, 1, 1, 0),
        0x08 => (Op::LdMemSp(nn), 3, 5, 0),
        0x18 => (Op::Jr(None, rel), 2, 3, 0),
        0x20 | 0x28 | 0x30 | 0x38 => (Op::Jr(Some(Cond::from(op >> 3)), rel), 2, 2, 1),
        0x40..=0x7f => {
            let (d, s) = (R8::from(op >> 3), R8::from(op));
            let c = if d == R8::MemHl || s == R8::MemHl {
                2
            } else {
                1
            };
            (Op::Ld(d, s), 1, c, 0)
        }
        0x80..=0xbf => {
            let s = R8::from(op);
            (Op::Alu(Alu::from(op >> 3), s), 1, hl_cost(s, 1, 2), 0)
        }
        0xc0 | 0xc8 | 0xd0 | 0xd8 => (Op::Ret(Some(Cond::from(op >> 3))), 1, 2, 3),
        0xc9 => (Op::Ret(None), 1, 4, 0),
        0xd9 => (Op::Reti, 1, 4, 0),
        0xc1 | 0xd1 | 0xe1 | 0xf1 => (Op::Pop(R16::from(op >> 4)), 1, 3, 0),
        0xc5 | 0xd5 | 0xe5 | 0xf5 => (Op::Push(R16::from(op >> 4)), 1, 4, 0),
        0xc2 | 0xca | 0xd2 | 0xda => (Op::Jp(Some(Cond::from(op >> 3)), nn), 3, 3, 1),
        0xc3 => (Op::Jp(None, nn), 3, 4, 0),
        0xc4 | 0xcc | 0xd4 | 0xdc => (Op::Call(Some(Cond::from(op >> 3)), nn), 3, 3, 3),
        0xcd => (Op::Call(None, nn), 3, 6, 0),
        _ if op & 0xc7 == 0xc6 => (Op::AluImm(Alu::from(op >> 3), b1), 2, 2, 0),
        _ if op & 0xc7 == 0xc7 => (Op::Rst(op & 0x38), 1, 4, 0),
        0xcb => {
            let r = R8::from(b1);
            let n = (b1 >> 3) & 7;
            let cb = match b1 >> 6 {
                0 => [
                    Cb::Rlc,
                    Cb::Rrc,
                    Cb::Rl,
                    Cb::Rr,
                    Cb::Sla,
                    Cb::Sra,
                    Cb::Swap,
                    Cb::Srl,
                ][n as usize],
                1 => Cb::Bit(n),
                2 => Cb::Res(n),
                _ => Cb::Set(n),
            };
            let c = match (cb, r) {
                (Cb::Bit(_), R8::MemHl) => 3,
                (_, R8::MemHl) => 4,
                _ => 2,
            };
            (Op::Cb(cb, r), 2, c, 0)
        }
        0xe0 => (Op::LdhNA(b1), 2, 3, 0),
        0xf0 => (Op::LdhAN(b1), 2, 3, 0),
        0xe2 => (Op::LdhCA, 1, 2, 0),
        0xf2 => (Op::LdhAC, 1, 2, 0),
        0xe8 => (Op::AddSpE(b1 as i8), 2, 4, 0),
        0xf8 => (Op::LdHlSpE(b1 as i8), 2, 3, 0),
        0xe9 => (Op::JpHl, 1, 1, 0),
        0xf9 => (Op::LdSpHl, 1, 2, 0),
        0xea => (Op::LdMemA(nn), 3, 4, 0),
        0xfa => (Op::LdAMem(nn), 3, 4, 0),
        _ => (Op::Invalid(op), 1, 1, 0),
    };
    Insn {
        addr,
        op: o,
        len,
        cycles,
        taken,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_opcode_decodes_with_its_length() {
        let invalid = [
            0xd3, 0xdb, 0xdd, 0xe3, 0xe4, 0xeb, 0xec, 0xed, 0xf4, 0xfc, 0xfd,
        ];
        let three = [
            0x01, 0x11, 0x21, 0x31, 0x08, 0xc2, 0xc3, 0xc4, 0xca, 0xcc, 0xcd, 0xd2, 0xd4, 0xda,
            0xdc, 0xea, 0xfa,
        ];
        let two = [
            0x06, 0x0e, 0x10, 0x16, 0x18, 0x1e, 0x20, 0x26, 0x28, 0x2e, 0x30, 0x36, 0x38, 0x3e,
            0xc6, 0xcb, 0xce, 0xd6, 0xde, 0xe0, 0xe6, 0xe8, 0xee, 0xf0, 0xf6, 0xf8, 0xfe,
        ];
        for op in 0..=255u8 {
            let insn = decode(0x4000, [op, 0x34, 0x12]);
            let expected = if three.contains(&op) {
                3
            } else if two.contains(&op) {
                2
            } else {
                1
            };
            assert_eq!(insn.len, expected, "opcode {op:02x}");
            assert_eq!(
                matches!(insn.op, Op::Invalid(_)),
                invalid.contains(&op),
                "opcode {op:02x}"
            );
        }
        assert_eq!(decode(0x100, [0x18, 0xfe, 0]).op, Op::Jr(None, 0x100));
        assert_eq!(
            decode(0x100, [0xcb, 0x7e, 0]).op,
            Op::Cb(Cb::Bit(7), R8::MemHl)
        );
        assert_eq!(decode(0x100, [0xcb, 0x7e, 0]).cycles, 3);
        assert_eq!(decode(0x100, [0xff, 0, 0]).op, Op::Rst(0x38));
        assert_eq!(decode(0x100, [0xfe, 9, 0]).op, Op::AluImm(Alu::Cp, 9));
    }
}
