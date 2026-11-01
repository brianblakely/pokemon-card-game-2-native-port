//! A straightforward SM83 interpreter, written independently of the
//! translator (its own decoding, cycle counts and flag arithmetic), for
//! checking the kernel. It never runs in the app: `tcg2-lockstep` runs it
//! beside the translated code on an identical machine and compares the two
//! after every dispatch.

use tcg2_core::Machine;

const Z: u8 = 0x80;
const N: u8 = 0x40;
const H: u8 = 0x20;
const C: u8 = 0x10;

fn fetch(m: &mut Machine) -> u8 {
    let v = m.read(m.cpu.pc);
    m.cpu.pc = m.cpu.pc.wrapping_add(1);
    v
}

fn fetch16(m: &mut Machine) -> u16 {
    let lo = fetch(m) as u16;
    let hi = fetch(m) as u16;
    lo | (hi << 8)
}

fn hl(m: &Machine) -> u16 {
    (m.cpu.h as u16) << 8 | m.cpu.l as u16
}

fn set_hl(m: &mut Machine, v: u16) {
    m.cpu.h = (v >> 8) as u8;
    m.cpu.l = v as u8;
}

fn get_r(m: &mut Machine, i: u8) -> u8 {
    match i {
        0 => m.cpu.b,
        1 => m.cpu.c,
        2 => m.cpu.d,
        3 => m.cpu.e,
        4 => m.cpu.h,
        5 => m.cpu.l,
        6 => {
            let a = hl(m);
            m.read(a)
        }
        _ => m.cpu.a,
    }
}

fn set_r(m: &mut Machine, i: u8, v: u8) {
    match i {
        0 => m.cpu.b = v,
        1 => m.cpu.c = v,
        2 => m.cpu.d = v,
        3 => m.cpu.e = v,
        4 => m.cpu.h = v,
        5 => m.cpu.l = v,
        6 => {
            let a = hl(m);
            m.write(a, v)
        }
        _ => m.cpu.a = v,
    }
}

/// BC, DE, HL, SP.
fn get_rr(m: &Machine, i: u8) -> u16 {
    match i {
        0 => (m.cpu.b as u16) << 8 | m.cpu.c as u16,
        1 => (m.cpu.d as u16) << 8 | m.cpu.e as u16,
        2 => hl(m),
        _ => m.cpu.sp,
    }
}

fn set_rr(m: &mut Machine, i: u8, v: u16) {
    match i {
        0 => {
            m.cpu.b = (v >> 8) as u8;
            m.cpu.c = v as u8
        }
        1 => {
            m.cpu.d = (v >> 8) as u8;
            m.cpu.e = v as u8
        }
        2 => set_hl(m, v),
        _ => m.cpu.sp = v,
    }
}

fn flag(set: bool, f: u8) -> u8 {
    if set { f } else { 0 }
}

fn cond(m: &Machine, i: u8) -> bool {
    match i & 3 {
        0 => m.cpu.f & Z == 0,
        1 => m.cpu.f & Z != 0,
        2 => m.cpu.f & C == 0,
        _ => m.cpu.f & C != 0,
    }
}

fn push(m: &mut Machine, v: u16) {
    m.cpu.sp = m.cpu.sp.wrapping_sub(1);
    m.write(m.cpu.sp, (v >> 8) as u8);
    m.cpu.sp = m.cpu.sp.wrapping_sub(1);
    m.write(m.cpu.sp, v as u8);
}

fn pop(m: &mut Machine) -> u16 {
    let lo = m.read(m.cpu.sp) as u16;
    m.cpu.sp = m.cpu.sp.wrapping_add(1);
    let hi = m.read(m.cpu.sp) as u16;
    m.cpu.sp = m.cpu.sp.wrapping_add(1);
    lo | (hi << 8)
}

/// 8-bit ALU on A by the opcode's operation field.
fn alu(m: &mut Machine, op: u8, v: u8) {
    let a = m.cpu.a;
    let carry = (m.cpu.f >> 4) & 1;
    match op & 7 {
        0 | 1 => {
            let c = if op & 7 == 1 { carry } else { 0 };
            let r = a as u16 + v as u16 + c as u16;
            let r8 = r as u8;
            m.cpu.f = flag(r8 == 0, Z) | flag((a ^ v ^ r8) & 0x10 != 0, H) | flag(r > 0xff, C);
            m.cpu.a = r8;
        }
        2 | 3 | 7 => {
            let c = if op & 7 == 3 { carry } else { 0 };
            let r = a as i16 - v as i16 - c as i16;
            let r8 = r as u8;
            m.cpu.f = flag(r8 == 0, Z) | N | flag((a ^ v ^ r8) & 0x10 != 0, H) | flag(r < 0, C);
            if op & 7 != 7 {
                m.cpu.a = r8;
            }
        }
        4 => {
            m.cpu.a = a & v;
            m.cpu.f = flag(m.cpu.a == 0, Z) | H;
        }
        5 => {
            m.cpu.a = a ^ v;
            m.cpu.f = flag(m.cpu.a == 0, Z);
        }
        _ => {
            m.cpu.a = a | v;
            m.cpu.f = flag(m.cpu.a == 0, Z);
        }
    }
}

fn sp_plus(m: &mut Machine, e: u8) -> u16 {
    let sp = m.cpu.sp;
    let d = e as i8 as i16 as u16;
    let r = sp.wrapping_add(d);
    let x = sp ^ d ^ r;
    m.cpu.f = flag(x & 0x10 != 0, H) | flag(x & 0x100 != 0, C);
    r
}

/// Executes one instruction at PC, adding its cycles before its memory
/// accesses (where the kernel's translated code reads devices).
pub fn step(m: &mut Machine) {
    let op = fetch(m);
    let x = op >> 6;
    let y = (op >> 3) & 7;
    let z = op & 7;
    match x {
        1 => {
            if op == 0x76 {
                m.tick(1);
                m.halt();
            } else {
                m.tick(if y == 6 || z == 6 { 2 } else { 1 });
                let v = get_r(m, z);
                set_r(m, y, v);
            }
        }
        2 => {
            m.tick(if z == 6 { 2 } else { 1 });
            let v = get_r(m, z);
            alu(m, y, v);
        }
        0 => match z {
            0 => match y {
                0 => m.tick(1),
                1 => {
                    let a = fetch16(m);
                    m.tick(5);
                    let sp = m.cpu.sp;
                    m.write(a, sp as u8);
                    m.write(a.wrapping_add(1), (sp >> 8) as u8);
                }
                2 => {
                    fetch(m);
                    m.tick(1);
                    m.stop();
                }
                3 => {
                    let e = fetch(m) as i8;
                    m.tick(3);
                    m.cpu.pc = m.cpu.pc.wrapping_add(e as i16 as u16);
                }
                _ => {
                    let e = fetch(m) as i8;
                    if cond(m, y - 4) {
                        m.tick(3);
                        m.cpu.pc = m.cpu.pc.wrapping_add(e as i16 as u16);
                    } else {
                        m.tick(2);
                    }
                }
            },
            1 => {
                if y & 1 == 0 {
                    let v = fetch16(m);
                    m.tick(3);
                    set_rr(m, y >> 1, v);
                } else {
                    m.tick(2);
                    let h = hl(m);
                    let v = get_rr(m, y >> 1);
                    let r = h as u32 + v as u32;
                    let x = h ^ v ^ (r as u16);
                    m.cpu.f = (m.cpu.f & Z) | flag(x & 0x1000 != 0, H) | flag(r > 0xffff, C);
                    set_hl(m, r as u16);
                }
            }
            2 => {
                m.tick(2);
                let addr = match y >> 1 {
                    0 => get_rr(m, 0),
                    1 => get_rr(m, 1),
                    _ => hl(m),
                };
                if y & 1 == 0 {
                    let a = m.cpu.a;
                    m.write(addr, a);
                } else {
                    m.cpu.a = m.read(addr);
                }
                match y >> 1 {
                    2 => set_hl(m, addr.wrapping_add(1)),
                    3 => set_hl(m, addr.wrapping_sub(1)),
                    _ => {}
                }
            }
            3 => {
                m.tick(2);
                let v = get_rr(m, y >> 1);
                let v = if y & 1 == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                set_rr(m, y >> 1, v);
            }
            4 | 5 => {
                m.tick(if y == 6 { 3 } else { 1 });
                let v = get_r(m, y);
                let r = if z == 4 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                let n = if z == 5 { N } else { 0 };
                m.cpu.f = (m.cpu.f & C) | flag(r == 0, Z) | n | flag((v ^ 1 ^ r) & 0x10 != 0, H);
                set_r(m, y, r);
            }
            6 => {
                let v = fetch(m);
                m.tick(if y == 6 { 3 } else { 2 });
                set_r(m, y, v);
            }
            _ => {
                m.tick(1);
                let a = m.cpu.a;
                let c = m.cpu.f & C != 0;
                match y {
                    0 => {
                        m.cpu.a = a.rotate_left(1);
                        m.cpu.f = flag(a & 0x80 != 0, C);
                    }
                    1 => {
                        m.cpu.a = a.rotate_right(1);
                        m.cpu.f = flag(a & 1 != 0, C);
                    }
                    2 => {
                        m.cpu.a = (a << 1) | c as u8;
                        m.cpu.f = flag(a & 0x80 != 0, C);
                    }
                    3 => {
                        m.cpu.a = (a >> 1) | ((c as u8) << 7);
                        m.cpu.f = flag(a & 1 != 0, C);
                    }
                    4 => {
                        let mut v = a as i32;
                        let n = m.cpu.f & N != 0;
                        let h = m.cpu.f & H != 0;
                        let mut carry = c;
                        if !n {
                            if carry || v > 0x99 {
                                v += 0x60;
                                carry = true;
                            }
                            if h || (v & 0x0f) > 9 {
                                v += 6;
                            }
                        } else {
                            if carry {
                                v -= 0x60;
                            }
                            if h {
                                v -= 6;
                            }
                        }
                        m.cpu.a = v as u8;
                        m.cpu.f = flag(m.cpu.a == 0, Z) | (m.cpu.f & N) | flag(carry, C);
                    }
                    5 => {
                        m.cpu.a = !a;
                        m.cpu.f |= N | H;
                    }
                    6 => m.cpu.f = (m.cpu.f & Z) | C,
                    _ => m.cpu.f = (m.cpu.f & Z) | if c { 0 } else { C },
                }
            }
        },
        _ => match op {
            0xc0 | 0xc8 | 0xd0 | 0xd8 => {
                if cond(m, y) {
                    m.tick(5);
                    m.cpu.pc = pop(m);
                } else {
                    m.tick(2);
                }
            }
            0xc1 | 0xd1 | 0xe1 | 0xf1 => {
                m.tick(3);
                let v = pop(m);
                match y >> 1 {
                    3 => {
                        m.cpu.a = (v >> 8) as u8;
                        m.cpu.f = v as u8 & 0xf0;
                    }
                    i => set_rr(m, i, v),
                }
            }
            0xc9 => {
                m.tick(4);
                m.cpu.pc = pop(m);
            }
            0xd9 => {
                m.tick(4);
                m.ei();
                m.cpu.pc = pop(m);
            }
            0xe9 => {
                m.tick(1);
                m.cpu.pc = hl(m);
            }
            0xf9 => {
                m.tick(2);
                m.cpu.sp = hl(m);
            }
            0xc2 | 0xca | 0xd2 | 0xda => {
                let a = fetch16(m);
                if cond(m, y) {
                    m.tick(4);
                    m.cpu.pc = a;
                } else {
                    m.tick(3);
                }
            }
            0xc3 => {
                let a = fetch16(m);
                m.tick(4);
                m.cpu.pc = a;
            }
            0xe0 => {
                let n = fetch(m);
                m.tick(3);
                let a = m.cpu.a;
                m.write(0xff00 | n as u16, a);
            }
            0xf0 => {
                let n = fetch(m);
                m.tick(3);
                m.cpu.a = m.read(0xff00 | n as u16);
            }
            0xe2 => {
                m.tick(2);
                let (c, a) = (m.cpu.c, m.cpu.a);
                m.write(0xff00 | c as u16, a);
            }
            0xf2 => {
                m.tick(2);
                let c = m.cpu.c;
                m.cpu.a = m.read(0xff00 | c as u16);
            }
            0xea => {
                let a = fetch16(m);
                m.tick(4);
                let v = m.cpu.a;
                m.write(a, v);
            }
            0xfa => {
                let a = fetch16(m);
                m.tick(4);
                m.cpu.a = m.read(a);
            }
            0xe8 => {
                let e = fetch(m);
                m.tick(4);
                m.cpu.sp = sp_plus(m, e);
            }
            0xf8 => {
                let e = fetch(m);
                m.tick(3);
                let v = sp_plus(m, e);
                set_hl(m, v);
            }
            0xf3 => {
                m.tick(1);
                m.di();
            }
            0xfb => {
                m.tick(1);
                m.ei();
            }
            0xc4 | 0xcc | 0xd4 | 0xdc => {
                let a = fetch16(m);
                if cond(m, y) {
                    m.tick(6);
                    let ret = m.cpu.pc;
                    push(m, ret);
                    m.cpu.pc = a;
                } else {
                    m.tick(3);
                }
            }
            0xcd => {
                let a = fetch16(m);
                m.tick(6);
                let ret = m.cpu.pc;
                push(m, ret);
                m.cpu.pc = a;
            }
            0xc5 | 0xd5 | 0xe5 | 0xf5 => {
                m.tick(4);
                let v = match y >> 1 {
                    3 => (m.cpu.a as u16) << 8 | m.cpu.f as u16,
                    i => get_rr(m, i),
                };
                push(m, v);
            }
            0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => {
                let v = fetch(m);
                m.tick(2);
                alu(m, y, v);
            }
            0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => {
                m.tick(4);
                let ret = m.cpu.pc;
                push(m, ret);
                m.cpu.pc = (y * 8) as u16;
            }
            0xcb => {
                let cb = fetch(m);
                let (cx, cy, cz) = (cb >> 6, (cb >> 3) & 7, cb & 7);
                m.tick(match (cx, cz) {
                    (1, 6) => 3,
                    (_, 6) => 4,
                    _ => 2,
                });
                let v = get_r(m, cz);
                let c = (m.cpu.f >> 4) & 1;
                match cx {
                    0 => {
                        let (r, out) = match cy {
                            0 => (v.rotate_left(1), v >> 7),
                            1 => (v.rotate_right(1), v & 1),
                            2 => ((v << 1) | c, v >> 7),
                            3 => ((v >> 1) | (c << 7), v & 1),
                            4 => (v << 1, v >> 7),
                            5 => ((v >> 1) | (v & 0x80), v & 1),
                            6 => (v.rotate_left(4), 0),
                            _ => (v >> 1, v & 1),
                        };
                        m.cpu.f = flag(r == 0, Z) | flag(out != 0, C);
                        set_r(m, cz, r);
                    }
                    1 => m.cpu.f = (m.cpu.f & C) | H | flag(v & (1 << cy) == 0, Z),
                    2 => set_r(m, cz, v & !(1 << cy)),
                    _ => set_r(m, cz, v | (1 << cy)),
                }
            }
            _ => {
                // Not an instruction: the console locks up.
                m.tick(1);
                m.cpu.pc = m.cpu.pc.wrapping_sub(1);
                m.raise_fault(
                    0xffff,
                    m.cpu.pc,
                    "the interpreter met a byte that is no instruction",
                );
            }
        },
    }
}
