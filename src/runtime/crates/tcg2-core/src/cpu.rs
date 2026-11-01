//! The SM83's register file and the flag semantics of its arithmetic.
//!
//! The kernel's translated code keeps the original program's registers here
//! and calls these helpers for every instruction that sets flags, so the
//! flags match the console's bit for bit (including H and N, which `daa` and
//! `push af` expose).

pub const FLAG_Z: u8 = 0x80;
pub const FLAG_N: u8 = 0x40;
pub const FLAG_H: u8 = 0x20;
pub const FLAG_C: u8 = 0x10;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    /// The address the dispatcher resumes at (the translated code itself
    /// keeps its position in Rust control flow between dispatches).
    pub pc: u16,
    pub ime: bool,
    pub halted: bool,
}

#[inline(always)]
fn z(v: u8) -> u8 {
    if v == 0 { FLAG_Z } else { 0 }
}

#[inline(always)]
fn fl(set: bool, flag: u8) -> u8 {
    if set { flag } else { 0 }
}

impl Cpu {
    #[inline(always)]
    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    #[inline(always)]
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    #[inline(always)]
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    #[inline(always)]
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }
    #[inline(always)]
    pub fn set_af(&mut self, v: u16) {
        [self.a, self.f] = v.to_be_bytes();
        self.f &= 0xf0;
    }
    #[inline(always)]
    pub fn set_bc(&mut self, v: u16) {
        [self.b, self.c] = v.to_be_bytes();
    }
    #[inline(always)]
    pub fn set_de(&mut self, v: u16) {
        [self.d, self.e] = v.to_be_bytes();
    }
    #[inline(always)]
    pub fn set_hl(&mut self, v: u16) {
        [self.h, self.l] = v.to_be_bytes();
    }

    #[inline(always)]
    pub fn zf(&self) -> bool {
        self.f & FLAG_Z != 0
    }
    #[inline(always)]
    pub fn cf(&self) -> bool {
        self.f & FLAG_C != 0
    }
    #[inline(always)]
    fn carry_in(&self) -> u8 {
        (self.f >> 4) & 1
    }

    // 8-bit arithmetic on A.

    #[inline(always)]
    pub fn add(&mut self, v: u8) {
        let a = self.a;
        let (r, carry) = a.overflowing_add(v);
        self.f = z(r) | fl((a & 0xf) + (v & 0xf) > 0xf, FLAG_H) | fl(carry, FLAG_C);
        self.a = r;
    }
    #[inline(always)]
    pub fn adc(&mut self, v: u8) {
        let a = self.a;
        let c = self.carry_in();
        let r16 = a as u16 + v as u16 + c as u16;
        let r = r16 as u8;
        self.f = z(r) | fl((a & 0xf) + (v & 0xf) + c > 0xf, FLAG_H) | fl(r16 > 0xff, FLAG_C);
        self.a = r;
    }
    #[inline(always)]
    pub fn sub(&mut self, v: u8) {
        let a = self.a;
        let r = a.wrapping_sub(v);
        self.f = z(r) | FLAG_N | fl((a & 0xf) < (v & 0xf), FLAG_H) | fl(a < v, FLAG_C);
        self.a = r;
    }
    #[inline(always)]
    pub fn sbc(&mut self, v: u8) {
        let a = self.a;
        let c = self.carry_in();
        let r = a.wrapping_sub(v).wrapping_sub(c);
        self.f = z(r)
            | FLAG_N
            | fl((a & 0xf) < (v & 0xf) + c, FLAG_H)
            | fl((a as u16) < v as u16 + c as u16, FLAG_C);
        self.a = r;
    }
    #[inline(always)]
    pub fn and(&mut self, v: u8) {
        self.a &= v;
        self.f = z(self.a) | FLAG_H;
    }
    #[inline(always)]
    pub fn xor(&mut self, v: u8) {
        self.a ^= v;
        self.f = z(self.a);
    }
    #[inline(always)]
    pub fn or(&mut self, v: u8) {
        self.a |= v;
        self.f = z(self.a);
    }
    #[inline(always)]
    pub fn cp(&mut self, v: u8) {
        let a = self.a;
        self.f =
            z(a.wrapping_sub(v)) | FLAG_N | fl((a & 0xf) < (v & 0xf), FLAG_H) | fl(a < v, FLAG_C);
    }
    #[inline(always)]
    pub fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.f = (self.f & FLAG_C) | z(r) | fl(v & 0xf == 0xf, FLAG_H);
        r
    }
    #[inline(always)]
    pub fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.f = (self.f & FLAG_C) | z(r) | FLAG_N | fl(v & 0xf == 0, FLAG_H);
        r
    }

    // 16-bit arithmetic.

    #[inline(always)]
    pub fn add_hl(&mut self, v: u16) {
        let hl = self.hl();
        let (r, carry) = hl.overflowing_add(v);
        self.f =
            (self.f & FLAG_Z) | fl((hl & 0xfff) + (v & 0xfff) > 0xfff, FLAG_H) | fl(carry, FLAG_C);
        self.set_hl(r);
    }
    /// SP plus a signed offset (`add sp, e` and `ld hl, sp+e`): Z and N clear,
    /// H and C from the unsigned low byte addition.
    #[inline(always)]
    pub fn sp_offset(&mut self, e: i8) -> u16 {
        let sp = self.sp;
        let u = e as u8 as u16;
        self.f = fl((sp & 0xf) + (u & 0xf) > 0xf, FLAG_H) | fl((sp & 0xff) + u > 0xff, FLAG_C);
        sp.wrapping_add(e as i16 as u16)
    }

    // Rotates of A (Z always clear).

    #[inline(always)]
    pub fn rlca(&mut self) {
        let a = self.a;
        self.a = a.rotate_left(1);
        self.f = fl(a & 0x80 != 0, FLAG_C);
    }
    #[inline(always)]
    pub fn rla(&mut self) {
        let a = self.a;
        self.a = (a << 1) | self.carry_in();
        self.f = fl(a & 0x80 != 0, FLAG_C);
    }
    #[inline(always)]
    pub fn rrca(&mut self) {
        let a = self.a;
        self.a = a.rotate_right(1);
        self.f = fl(a & 1 != 0, FLAG_C);
    }
    #[inline(always)]
    pub fn rra(&mut self) {
        let a = self.a;
        self.a = (a >> 1) | (self.carry_in() << 7);
        self.f = fl(a & 1 != 0, FLAG_C);
    }

    // The CB-prefixed shifts and rotates.

    #[inline(always)]
    pub fn rlc(&mut self, v: u8) -> u8 {
        let r = v.rotate_left(1);
        self.f = z(r) | fl(v & 0x80 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn rrc(&mut self, v: u8) -> u8 {
        let r = v.rotate_right(1);
        self.f = z(r) | fl(v & 1 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn rl(&mut self, v: u8) -> u8 {
        let r = (v << 1) | self.carry_in();
        self.f = z(r) | fl(v & 0x80 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn rr(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (self.carry_in() << 7);
        self.f = z(r) | fl(v & 1 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn sla(&mut self, v: u8) -> u8 {
        let r = v << 1;
        self.f = z(r) | fl(v & 0x80 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn sra(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (v & 0x80);
        self.f = z(r) | fl(v & 1 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn swap(&mut self, v: u8) -> u8 {
        let r = v.rotate_left(4);
        self.f = z(r);
        r
    }
    #[inline(always)]
    pub fn srl(&mut self, v: u8) -> u8 {
        let r = v >> 1;
        self.f = z(r) | fl(v & 1 != 0, FLAG_C);
        r
    }
    #[inline(always)]
    pub fn bit(&mut self, n: u8, v: u8) {
        self.f = (self.f & FLAG_C) | FLAG_H | z(v & (1 << n));
    }

    // Accumulator and carry adjustments.

    #[inline(always)]
    pub fn daa(&mut self) {
        let n = self.f & FLAG_N != 0;
        let h = self.f & FLAG_H != 0;
        let mut carry = self.cf();
        let mut adjust = 0u8;
        if h || (!n && self.a & 0xf > 9) {
            adjust |= 0x06;
        }
        if carry || (!n && self.a > 0x99) {
            adjust |= 0x60;
            carry = true;
        }
        self.a = if n {
            self.a.wrapping_sub(adjust)
        } else {
            self.a.wrapping_add(adjust)
        };
        self.f = z(self.a) | (self.f & FLAG_N) | fl(carry, FLAG_C);
    }
    #[inline(always)]
    pub fn cpl(&mut self) {
        self.a = !self.a;
        self.f |= FLAG_N | FLAG_H;
    }
    #[inline(always)]
    pub fn scf(&mut self) {
        self.f = (self.f & FLAG_Z) | FLAG_C;
    }
    #[inline(always)]
    pub fn ccf(&mut self) {
        self.f = (self.f & FLAG_Z) | ((self.f & FLAG_C) ^ FLAG_C);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daa_after_bcd_addition_and_subtraction() {
        let mut cpu = Cpu {
            a: 0x45,
            ..Cpu::default()
        };
        cpu.add(0x38);
        cpu.daa();
        assert_eq!((cpu.a, cpu.f), (0x83, 0));
        let mut cpu = Cpu {
            a: 0x83,
            ..Cpu::default()
        };
        cpu.sub(0x38);
        cpu.daa();
        assert_eq!((cpu.a, cpu.f & !FLAG_N), (0x45, 0));
        let mut cpu = Cpu {
            a: 0x99,
            ..Cpu::default()
        };
        cpu.add(0x01);
        cpu.daa();
        assert_eq!((cpu.a, cpu.f), (0x00, FLAG_Z | FLAG_C));
    }

    #[test]
    fn half_carries() {
        let mut cpu = Cpu {
            a: 0x0f,
            ..Cpu::default()
        };
        cpu.add(1);
        assert_eq!(cpu.f, FLAG_H);
        cpu.f = FLAG_C;
        cpu.a = 0x10;
        cpu.sbc(0);
        assert_eq!((cpu.a, cpu.f), (0x0f, FLAG_N | FLAG_H));
        cpu.sp = 0x00ff;
        assert_eq!(cpu.sp_offset(1), 0x0100);
        assert_eq!(cpu.f, FLAG_H | FLAG_C);
        cpu.sp = 0x0000;
        assert_eq!(cpu.sp_offset(-1), 0xffff);
        assert_eq!(cpu.f, 0);
    }
}
