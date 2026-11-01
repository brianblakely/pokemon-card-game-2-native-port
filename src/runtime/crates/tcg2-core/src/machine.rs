//! The machine the kernel runs on: the SM83's registers and address space,
//! and the devices behind it, all advanced by one clock.
//!
//! The clock counts units of 1/8388608 s (a CPU clock at double speed, half
//! of one at normal speed). Translated code adds each instruction's cycles;
//! devices never step on their own but are computed from the clock when the
//! game touches them, and the moments they interrupt are events on it.

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::ppu::{FRAME_UNITS, Ppu};
use crate::timer::Timer;

pub const CLOCK_HZ: u64 = 8_388_608;

pub const INT_VBLANK: u8 = 0x01;
pub const INT_STAT: u8 = 0x02;
pub const INT_TIMER: u8 = 0x04;
pub const INT_SERIAL: u8 = 0x08;
pub const INT_JOYPAD: u8 = 0x10;

/// Host-side button bits (`Machine::buttons`).
pub mod buttons {
    pub const A: u8 = 0x01;
    pub const B: u8 = 0x02;
    pub const SELECT: u8 = 0x04;
    pub const START: u8 = 0x08;
    pub const RIGHT: u8 = 0x10;
    pub const LEFT: u8 = 0x20;
    pub const UP: u8 = 0x40;
    pub const DOWN: u8 = 0x80;
}

/// The program reached code the kernel does not have: an address no block
/// starts at, or bytes that are no instruction (the console would hang).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub bank: u16,
    pub pc: u16,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct Serial {
    pub sb: u8,
    pub sc: u8,
    pub done_at: u64,
}

#[derive(Clone)]
pub struct Machine {
    pub cpu: Cpu,
    pub clock: u64,
    /// The earliest clock at which a device needs attention or an interrupt
    /// may be taken; translated code returns to the dispatcher at it.
    pub next_event: u64,
    /// log2 of clock units per M-cycle: 3 at normal speed, 2 at double.
    pub speed_shift: u32,
    pub key1_prepare: bool,
    pub cart: Cartridge,
    pub wram: Box<[u8; 0x8000]>,
    pub wram_offset: usize,
    pub wbk: u8,
    pub hram: [u8; 0x80],
    pub ie: u8,
    pub iflag: u8,
    pub ppu: Ppu,
    pub apu: Apu,
    pub timer: Timer,
    pub serial: Serial,
    pub joyp_select: u8,
    /// The buttons held now (`buttons::*`), set by the host between frames.
    pub buttons: u8,
    pub rp: u8,
    /// Registers without behaviour, kept as written.
    pub io: [u8; 0x80],
    /// A frame ended (vertical blank began, or a frame's time passed with the
    /// LCD off); the run loop hands control back to the host.
    pub frame_ready: bool,
    pub last_frame_end: u64,
    pub frames: u64,
    pub fault: Option<Fault>,
}

impl Machine {
    /// The state the CGB boot ROM leaves for a CGB-only cartridge.
    pub fn new(rom: std::sync::Arc<[u8]>, sample_rate: u32) -> Self {
        let mut m = Self {
            cpu: Cpu {
                a: 0x11,
                f: 0x80,
                b: 0x00,
                c: 0x00,
                d: 0xff,
                e: 0x56,
                h: 0x00,
                l: 0x0d,
                sp: 0xfffe,
                pc: 0x0100,
                ime: false,
                halted: false,
            },
            clock: 0,
            next_event: 0,
            speed_shift: 3,
            key1_prepare: false,
            cart: Cartridge::new(rom),
            wram: Box::new([0; 0x8000]),
            wram_offset: 0x1000,
            wbk: 1,
            hram: [0; 0x80],
            ie: 0,
            iflag: 0xe1,
            ppu: Ppu::new(),
            apu: Apu::new(sample_rate),
            timer: Timer::default(),
            serial: Serial {
                sb: 0,
                sc: 0x7e,
                done_at: u64::MAX,
            },
            joyp_select: 0x30,
            buttons: 0,
            rp: 0,
            io: [0xff; 0x80],
            frame_ready: false,
            last_frame_end: 0,
            frames: 0,
            fault: None,
        };
        m.iflag &= 0x1f;
        m.recompute_next_event();
        m
    }

    /// Adds `mcycles` of CPU time.
    #[inline(always)]
    pub fn tick(&mut self, mcycles: u32) {
        self.clock += (mcycles as u64) << self.speed_shift;
    }

    /// A memory read. Work RAM bank 0 (the stack and most variables) is
    /// read in place; everything else takes one call.
    #[inline(always)]
    pub fn read(&mut self, addr: u16) -> u8 {
        if addr & 0xf000 == 0xc000 {
            self.wram[addr as usize & 0xfff]
        } else {
            self.read_other(addr)
        }
    }

    #[inline(never)]
    fn read_other(&mut self, addr: u16) -> u8 {
        match addr >> 12 {
            0x0..=0x3 => self.cart.rom[addr as usize],
            0x4..=0x7 => self.cart.rom[self.cart.rom_offset + (addr as usize & 0x3fff)],
            0x8 | 0x9 => self.ppu.read_vram(addr),
            0xa | 0xb => self.cart.read_sram(addr),
            0xc => self.wram[addr as usize & 0xfff],
            0xd => self.wram[self.wram_offset + (addr as usize & 0xfff)],
            _ => self.read_high(addr),
        }
    }

    /// A memory write; work RAM bank 0 in place, everything else one call.
    #[inline(always)]
    pub fn write(&mut self, addr: u16, value: u8) {
        if addr & 0xf000 == 0xc000 {
            self.wram[addr as usize & 0xfff] = value;
        } else {
            self.write_other(addr, value);
        }
    }

    #[inline(never)]
    fn write_other(&mut self, addr: u16, value: u8) {
        match addr >> 12 {
            0x0..=0x7 => {
                let bank = self.cart.rom_bank;
                self.cart.write_register(addr, value);
                if self.cart.rom_bank != bank {
                    // Code at $4000-$7FFF now comes from another bank: the
                    // dispatcher takes over at the next block boundary.
                    self.next_event = self.clock;
                }
            }
            0x8 | 0x9 => self.ppu.write_vram(addr, value, self.clock),
            0xa | 0xb => self.cart.write_sram(addr, value),
            0xc => self.wram[addr as usize & 0xfff] = value,
            0xd => self.wram[self.wram_offset + (addr as usize & 0xfff)] = value,
            _ => self.write_high(addr, value),
        }
    }

    #[inline]
    pub fn read16(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr);
        let hi = self.read(addr.wrapping_add(1));
        u16::from_le_bytes([lo, hi])
    }

    #[inline(always)]
    pub fn push(&mut self, value: u16) {
        let sp = self.cpu.sp.wrapping_sub(2);
        if sp & 0xf000 == 0xc000 && sp & 0xfff != 0xfff {
            let i = sp as usize & 0xfff;
            [self.wram[i + 1], self.wram[i]] = value.to_be_bytes();
            self.cpu.sp = sp;
        } else {
            self.push_other(value);
        }
    }

    #[inline(never)]
    fn push_other(&mut self, value: u16) {
        let [hi, lo] = value.to_be_bytes();
        self.cpu.sp = self.cpu.sp.wrapping_sub(1);
        self.write(self.cpu.sp, hi);
        self.cpu.sp = self.cpu.sp.wrapping_sub(1);
        self.write(self.cpu.sp, lo);
    }

    #[inline(always)]
    pub fn pop(&mut self) -> u16 {
        let sp = self.cpu.sp;
        if sp & 0xf000 == 0xc000 && sp & 0xfff != 0xfff {
            let i = sp as usize & 0xfff;
            self.cpu.sp = sp.wrapping_add(2);
            u16::from_le_bytes([self.wram[i], self.wram[i + 1]])
        } else {
            self.pop_other()
        }
    }

    #[inline(never)]
    fn pop_other(&mut self) -> u16 {
        let lo = self.read(self.cpu.sp);
        self.cpu.sp = self.cpu.sp.wrapping_add(1);
        let hi = self.read(self.cpu.sp);
        self.cpu.sp = self.cpu.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    #[inline(never)]
    fn read_high(&mut self, addr: u16) -> u8 {
        match addr {
            0xe000..=0xfdff => self.read(addr - 0x2000),
            0xfe00..=0xfe9f => self.ppu.oam[addr as usize - 0xfe00],
            0xfea0..=0xfeff => 0x00,
            0xff00..=0xff7f => self.io_read(addr as u8),
            0xff80..=0xfffe => self.hram[addr as usize - 0xff80],
            _ => self.ie,
        }
    }

    #[inline(never)]
    fn write_high(&mut self, addr: u16, value: u8) {
        match addr {
            0xe000..=0xfdff => self.write(addr - 0x2000, value),
            0xfe00..=0xfe9f => self
                .ppu
                .write_oam(addr as usize - 0xfe00, value, self.clock),
            0xfea0..=0xfeff => {}
            0xff00..=0xff7f => self.io_write(addr as u8, value),
            0xff80..=0xfffe => self.hram[addr as usize - 0xff80] = value,
            _ => {
                self.ie = value;
                self.interrupt_check();
            }
        }
    }

    /// `ldh a, [n]` / `ld a, [$ff00+c]`.
    #[inline(always)]
    pub fn read_ff(&mut self, low: u8) -> u8 {
        if low >= 0x80 {
            if low == 0xff {
                self.ie
            } else {
                self.hram[low as usize - 0x80]
            }
        } else {
            self.io_read(low)
        }
    }

    #[inline(always)]
    pub fn write_ff(&mut self, low: u8, value: u8) {
        if low >= 0x80 && low != 0xff {
            self.hram[low as usize - 0x80] = value;
        } else {
            self.write_high(0xff00 | low as u16, value);
        }
    }

    pub fn io_read(&mut self, reg: u8) -> u8 {
        match reg {
            0x00 => {
                let mut low = 0x0f;
                if self.joyp_select & 0x10 == 0 {
                    low &= !(self.buttons >> 4);
                }
                if self.joyp_select & 0x20 == 0 {
                    low &= !(self.buttons & 0x0f);
                }
                0xc0 | self.joyp_select | low
            }
            0x01 => self.serial.sb,
            0x02 => self.serial.sc | 0x7c,
            0x04 => self.timer.div(self.clock),
            0x05 => self.timer.read_tima(self.clock),
            0x06 => self.timer.tma,
            0x07 => self.timer.tac,
            0x0f => self.iflag | 0xe0,
            0x10..=0x3f => self.apu.read(reg, self.clock),
            0x40..=0x45 | 0x47..=0x4b | 0x68..=0x6b => self.ppu.read_register(reg, self.clock),
            0x4d => 0x7e | if self.speed_shift == 2 { 0x80 } else { 0 } | self.key1_prepare as u8,
            0x4f => 0xfe | self.ppu.vbk,
            0x55 => 0xff,
            // No infrared light is ever received.
            0x56 => (self.rp & 0xc1) | 0x3e,
            0x70 => 0xf8 | self.wbk,
            _ => self.io[reg as usize],
        }
    }

    pub fn io_write(&mut self, reg: u8, value: u8) {
        match reg {
            0x00 => self.joyp_select = value & 0x30,
            0x01 => self.serial.sb = value,
            0x02 => {
                self.serial.sc = value;
                // Only an internally clocked transfer ends with no partner:
                // the cable's line idles high, so $FF arrives.
                self.serial.done_at = if value & 0x81 == 0x81 {
                    let bit = if value & 0x02 != 0 { 32 } else { 1024 };
                    let bit = if self.speed_shift == 2 { bit / 2 } else { bit };
                    self.clock + 8 * bit
                } else {
                    u64::MAX
                };
                self.recompute_next_event();
            }
            0x04..=0x07 => {
                self.timer.write(reg, value, self.clock);
                self.recompute_next_event();
            }
            0x0f => {
                self.iflag = value & 0x1f;
                self.interrupt_check();
            }
            0x10..=0x3f => self.apu.write(reg, value, self.clock),
            0x46 => self.oam_dma(value),
            0x40..=0x45 | 0x47..=0x4b | 0x68..=0x6b => {
                self.ppu.write_register(reg, value, self.clock);
                self.recompute_next_event();
            }
            0x4d => self.key1_prepare = value & 1 != 0,
            0x4f => self.ppu.vbk = value & 1,
            0x51..=0x54 => self.io[reg as usize] = value,
            0x55 => self.vram_dma(value),
            0x56 => self.rp = value,
            0x70 => {
                self.wbk = (value & 7).max(1);
                self.wram_offset = self.wbk as usize * 0x1000;
            }
            _ => self.io[reg as usize] = value,
        }
    }

    fn oam_dma(&mut self, page: u8) {
        self.io[0x46] = page;
        let base = (page as u16) << 8;
        let mut data = [0u8; 0xa0];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.read(base + i as u16);
        }
        self.ppu.write_oam_block(&data, self.clock);
    }

    /// $FF55: general-purpose VRAM DMA (and HBlank DMA, done at once).
    fn vram_dma(&mut self, value: u8) {
        let src = u16::from_be_bytes([self.io[0x51], self.io[0x52]]) & 0xfff0;
        let dst = 0x8000 | (u16::from_be_bytes([self.io[0x53], self.io[0x54]]) & 0x1ff0);
        let len = (value as u16 & 0x7f) * 16 + 16;
        for i in 0..len {
            let byte = self.read(src.wrapping_add(i));
            self.ppu
                .write_vram(0x8000 | (dst.wrapping_add(i) & 0x1fff), byte, self.clock);
        }
        self.tick(len as u32 / 2);
    }

    /// Makes the dispatcher look at interrupts at its next boundary.
    #[inline(always)]
    pub fn interrupt_check(&mut self) {
        self.next_event = self.clock;
    }

    #[inline(always)]
    pub fn ei(&mut self) {
        self.cpu.ime = true;
        self.interrupt_check();
    }

    #[inline(always)]
    pub fn di(&mut self) {
        self.cpu.ime = false;
    }

    /// `halt`: the dispatcher sleeps the CPU until an enabled interrupt is
    /// requested.
    #[inline(always)]
    pub fn halt(&mut self) {
        self.cpu.halted = true;
        self.interrupt_check();
    }

    /// `stop`: on the CGB with KEY1 armed, the speed switch.
    pub fn stop(&mut self) {
        if self.key1_prepare {
            self.key1_prepare = false;
            self.speed_shift = if self.speed_shift == 3 { 2 } else { 3 };
            // The switch takes 2050 M-cycles at the new speed.
            self.tick(2050);
            self.timer.set_speed(self.speed_shift == 2, self.clock);
        }
        self.recompute_next_event();
    }

    pub fn recompute_next_event(&mut self) {
        let mut next = self
            .ppu
            .next_event()
            .min(self.timer.next_overflow)
            .min(self.serial.done_at);
        if !self.ppu.lcd_on() {
            next = next.min(self.last_frame_end + FRAME_UNITS);
        }
        self.next_event = next;
    }

    /// Runs every device event due by now.
    pub fn handle_events(&mut self) {
        let (flags, frame_end) = self.ppu.on_events(self.clock);
        self.iflag |= flags;
        if self.timer.on_events(self.clock) {
            self.iflag |= INT_TIMER;
        }
        if self.clock >= self.serial.done_at {
            self.serial.done_at = u64::MAX;
            self.serial.sb = 0xff;
            self.serial.sc &= 0x7f;
            self.iflag |= INT_SERIAL;
        }
        if frame_end || (!self.ppu.lcd_on() && self.clock >= self.last_frame_end + FRAME_UNITS) {
            self.end_frame();
        }
        self.recompute_next_event();
    }

    fn end_frame(&mut self) {
        self.frame_ready = true;
        self.frames += 1;
        self.last_frame_end = if self.ppu.lcd_on() {
            self.ppu.next_vblank - FRAME_UNITS
        } else {
            self.clock
        };
        self.apu.sync(self.clock);
    }

    /// Takes a requested, enabled interrupt if IME allows: pushes the
    /// resume address and enters the vector. Returns whether it did.
    #[inline]
    pub fn take_interrupt(&mut self) -> bool {
        let pending = self.ie & self.iflag & 0x1f;
        if pending == 0 {
            return false;
        }
        if self.cpu.halted {
            self.cpu.halted = false;
            self.tick(1);
        }
        if !self.cpu.ime {
            return false;
        }
        let bit = pending.trailing_zeros();
        self.iflag &= !(1 << bit);
        self.cpu.ime = false;
        let pc = self.cpu.pc;
        self.push(pc);
        self.cpu.pc = 0x40 + 8 * bit as u16;
        self.tick(5);
        true
    }

    /// The whole machine as a save state.
    pub fn save_state(&self) -> Vec<u8> {
        let mut w = crate::state::Writer::default();
        crate::state::header(&mut w);
        let c = &self.cpu;
        for v in [c.a, c.f, c.b, c.c, c.d, c.e, c.h, c.l] {
            w.u8(v);
        }
        w.u16(c.sp);
        w.u16(c.pc);
        w.bool(c.ime);
        w.bool(c.halted);
        w.u64(self.clock);
        w.u32(self.speed_shift);
        w.bool(self.key1_prepare);
        self.cart.save(&mut w);
        w.bytes(&self.wram[..]);
        w.u8(self.wbk);
        w.bytes(&self.hram);
        w.u8(self.ie);
        w.u8(self.iflag);
        self.ppu.save(&mut w);
        self.apu.save(&mut w);
        self.timer.save(&mut w);
        w.u8(self.serial.sb);
        w.u8(self.serial.sc);
        w.u64(self.serial.done_at);
        w.u8(self.joyp_select);
        w.u8(self.rp);
        w.bytes(&self.io);
        w.bool(self.frame_ready);
        w.u64(self.last_frame_end);
        w.u64(self.frames);
        w.bytes
    }

    /// Restores a save state (taken from a machine on the same ROM).
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), crate::state::StateError> {
        let mut r = crate::state::open(bytes)?;
        let mut next = self.clone();
        let c = &mut next.cpu;
        for v in [
            &mut c.a, &mut c.f, &mut c.b, &mut c.c, &mut c.d, &mut c.e, &mut c.h, &mut c.l,
        ] {
            *v = r.u8()?;
        }
        c.sp = r.u16()?;
        c.pc = r.u16()?;
        c.ime = r.bool()?;
        c.halted = r.bool()?;
        next.clock = r.u64()?;
        next.speed_shift = r.u32()?;
        next.key1_prepare = r.bool()?;
        next.cart.load(&mut r)?;
        r.fill(&mut next.wram[..])?;
        next.wbk = r.u8()?;
        next.wram_offset = next.wbk as usize * 0x1000;
        r.fill(&mut next.hram)?;
        next.ie = r.u8()?;
        next.iflag = r.u8()?;
        next.ppu.load(&mut r)?;
        next.apu.load(&mut r)?;
        next.timer.load(&mut r)?;
        next.serial.sb = r.u8()?;
        next.serial.sc = r.u8()?;
        next.serial.done_at = r.u64()?;
        next.joyp_select = r.u8()?;
        next.rp = r.u8()?;
        r.fill(&mut next.io)?;
        next.frame_ready = r.bool()?;
        next.last_frame_end = r.u64()?;
        next.frames = r.u64()?;
        next.fault = None;
        next.recompute_next_event();
        next.interrupt_check();
        *self = next;
        Ok(())
    }

    /// Stops the run loop with a fault at its next boundary.
    #[cold]
    pub fn raise_fault(&mut self, bank: u16, pc: u16, reason: &'static str) {
        if self.fault.is_none() {
            self.fault = Some(Fault { bank, pc, reason });
        }
        self.next_event = 0;
    }

    /// While halted with nothing to wake for, skips the clock to the next
    /// event: the host is idle until then.
    #[inline]
    pub fn sleep(&mut self) {
        if self.clock < self.next_event {
            self.clock = self.next_event;
        }
    }
}
