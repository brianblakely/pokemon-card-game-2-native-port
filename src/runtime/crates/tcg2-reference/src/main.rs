//! `tcg2-lockstep`: runs the translated kernel and the reference interpreter
//! side by side on identical machines with the same input, comparing the
//! CPU after every dispatch and the whole machine at every frame's end.
//!
//! ```text
//! tcg2-lockstep --rom rom/poketcg2.gbc --frames 3000 --press 700:start ...
//! ```

use tcg2_core::Machine;
use tcg2_kernel::script::{RandomPlay, Script};
use tcg2_kernel::{Kernel, Language};

fn cpu_line(m: &Machine) -> String {
    let c = &m.cpu;
    format!(
        "pc {:04x} af {:02x}{:02x} bc {:02x}{:02x} de {:02x}{:02x} hl {:02x}{:02x} sp {:04x} ime {} halt {} bank {:02x} clock {}",
        c.pc,
        c.a,
        c.f,
        c.b,
        c.c,
        c.d,
        c.e,
        c.h,
        c.l,
        c.sp,
        c.ime as u8,
        c.halted as u8,
        m.cart.rom_bank,
        m.clock
    )
}

fn same_cpu(a: &Machine, b: &Machine) -> bool {
    let (x, y) = (&a.cpu, &b.cpu);
    x.a == y.a
        && x.f == y.f
        && x.b == y.b
        && x.c == y.c
        && x.d == y.d
        && x.e == y.e
        && x.h == y.h
        && x.l == y.l
        && x.sp == y.sp
        && x.ime == y.ime
        && x.halted == y.halted
        && a.clock == b.clock
}

/// Every byte of state the program can observe, by region.
fn regions(m: &Machine) -> Vec<(&'static str, Vec<u8>)> {
    let p = &m.ppu;
    vec![
        ("wram", m.wram.to_vec()),
        ("hram", m.hram.to_vec()),
        ("vram", p.vram.to_vec()),
        ("oam", p.oam.to_vec()),
        ("sram", m.cart.sram.to_vec()),
        ("bg palettes", p.bg_palettes.to_vec()),
        ("obj palettes", p.obj_palettes.to_vec()),
        (
            "lcd",
            vec![
                p.lcdc, p.stat, p.scy, p.scx, p.lyc, p.wy, p.wx, p.bcps, p.ocps, p.vbk,
            ],
        ),
        ("sound", m.apu.regs.to_vec()),
        (
            "system",
            vec![
                m.ie,
                m.iflag,
                m.wbk,
                m.cart.rom_bank as u8,
                (m.cart.rom_bank >> 8) as u8,
                m.cart.sram_bank,
                m.cart.sram_enabled as u8,
                m.timer.tima,
                m.timer.tma,
                m.timer.tac,
                m.serial.sb,
                m.serial.sc,
                m.speed_shift as u8,
            ],
        ),
    ]
}

/// Runs every translated block start from randomized registers on both the
/// kernel and the interpreter, one dispatch each, and compares the result:
/// the whole program's translation, reachable in play or not.
fn check_blocks(base: &Machine, code: &tcg2_kernel::Code, trials: u64) -> usize {
    let mut rng = RandomPlay::new(0x5eed);
    let mut random = |frame: &mut u64| {
        *frame += 1000;
        // RandomPlay's generator, read through its buttons: cheap and seeded.
        let mut v = 0u64;
        for i in 0..8 {
            v = v << 8 | rng.buttons(*frame + i * 100) as u64;
        }
        v
    };
    let mut frame = 0u64;
    let mut failures = 0;
    let mut self_switched = 0u64;
    let entries: Vec<(usize, u16)> = tcg2_kernel::entries().collect();
    let routines = tcg2_kernel::ram_routines();
    for &(bank, addr) in &entries {
        for trial in 0..trials {
            let mut a = base.clone();
            let r = random(&mut frame) ^ ((bank as u64) << 48) ^ ((addr as u64) << 32) ^ trial;
            let regs = r.to_le_bytes();
            a.cpu.a = regs[0];
            a.cpu.f = regs[1] & 0xf0;
            a.cpu.b = regs[2];
            a.cpu.c = regs[3];
            a.cpu.d = regs[4];
            a.cpu.e = regs[5];
            a.cpu.h = regs[6];
            a.cpu.l = regs[7];
            a.cpu.sp = 0xcf00 - (regs[2] as u16 & 0x7e);
            a.cpu.pc = addr;
            a.cpu.halted = false;
            if bank != 0 {
                a.cart.write_register(0x2000, bank as u8);
            }
            // Bound each run: the dispatch returns at its next block
            // boundary past this budget.
            a.next_event = a.clock + 20_000;
            let mut b = a.clone();
            let switches_before = a.cart.bank_switches;
            let next = code.dispatch(&mut a, addr);
            if addr >= 0x4000 && a.cart.bank_switches != switches_before {
                // Random registers made the code switch its own bank (a
                // store through HL into the MBC): the console would fetch
                // from the new bank mid-block, which game code never does.
                self_switched += 1;
                continue;
            }
            a.cpu.pc = next;
            let trace =
                std::env::var("TCG2_TRACE").ok() == Some(format!("{bank:02x}:{addr:04x}:{trial}"));
            if trace {
                eprintln!("start: {}", cpu_line(&b));
            }
            let mut steps = 0u64;
            while b.clock < a.clock && b.fault.is_none() && steps < 1_000_000 {
                tcg2_reference::step(&mut b);
                if trace {
                    eprintln!("  step: {}", cpu_line(&b));
                }
                steps += 1;
            }
            let pc_ok = next == b.cpu.pc
                || routines.iter().any(|&(ram, rom, len)| {
                    (rom..rom + len).contains(&next) && b.cpu.pc == ram + (next - rom)
                });
            let faulted = a.fault.is_some() || b.fault.is_some();
            let same = same_cpu(&a, &b) && pc_ok;
            let memory_same = regions(&a) == regions(&b);
            if faulted {
                if a.fault.is_some() != b.fault.is_some() {
                    failures += 1;
                    eprintln!(
                        "block {bank:02x}:{addr:04x} [{}]: only {} faulted",
                        tcg2_kernel::describe(bank as u16, addr),
                        if a.fault.is_some() {
                            "the kernel"
                        } else {
                            "the interpreter"
                        }
                    );
                }
            } else if !same || !memory_same {
                failures += 1;
                eprintln!(
                    "block {bank:02x}:{addr:04x} [{}] trial {trial}:",
                    tcg2_kernel::describe(bank as u16, addr)
                );
                eprintln!("  kernel      {}", cpu_line(&a));
                eprintln!("  interpreter {}", cpu_line(&b));
                if !memory_same {
                    for ((name, x), (_, y)) in regions(&a).iter().zip(regions(&b).iter()) {
                        if let Some(i) = x.iter().zip(y.iter()).position(|(p, q)| p != q) {
                            eprintln!(
                                "  {name}[{i:#06x}]: kernel {:#04x} interpreter {:#04x}",
                                x[i], y[i]
                            );
                        }
                    }
                }
                if failures > 40 {
                    return failures;
                }
            }
        }
    }
    println!(
        "{} block starts x {trials} trials checked ({self_switched} set aside: they switched their own ROM bank), {failures} differences",
        entries.len()
    );
    failures
}

fn main() {
    let mut rom_path = String::from("rom/poketcg2.gbc");
    let mut blocks: Option<u64> = None;
    let mut frames = 1200u64;
    let mut script = Script::default();
    let mut calls: Vec<(u64, String)> = Vec::new();
    let mut sram: Option<String> = None;
    let mut language = Language::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--rom" => rom_path = value,
            "--frames" => frames = value.parse().expect("frame count"),
            "--sram" => sram = Some(value),
            "--language" => language = Language::parse(&value).expect("--language en-US|ja"),
            "--call" => {
                let (frame, name) = value.split_once(':').expect("--call FRAME:ROUTINE");
                calls.push((frame.parse().expect("frame"), name.to_string()));
            }
            "--blocks" => blocks = Some(value.parse().expect("trials per block")),
            "--random" => {
                script.random = Some(value.parse().unwrap_or_else(|e: String| panic!("{e}")))
            }
            "--press" => script
                .presses
                .push(value.parse().unwrap_or_else(|e: String| panic!("{e}"))),
            _ => panic!("unknown argument {arg}"),
        }
    }
    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| panic!("{rom_path}: {e}"));
    let mut kernel = Kernel::new(rom, 48_000, language).expect("ROM");
    // The same image: the player's ROM in the same language.
    let mut b = Machine::new(kernel.machine.cart.rom.clone(), 48_000);
    if let Some(path) = &sram {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        kernel.load_sram(&bytes);
        b.cart.load_sram(&bytes);
    }
    if let Some(trials) = blocks {
        // A realistic machine to start from: the game booted for a while.
        for _ in 0..frames {
            kernel.run_frame().expect("boot");
        }
        let failures = check_blocks(&kernel.machine, &kernel.code, trials);
        std::process::exit(if failures == 0 { 0 } else { 1 });
    }
    let routines = tcg2_kernel::ram_routines();
    // The kernel runs a RAM routine as its ROM translation: the same code at
    // another address.
    let same_pc = |kernel_pc: u16, interp_pc: u16| {
        kernel_pc == interp_pc
            || routines.iter().any(|&(ram, rom, len)| {
                (rom..rom + len).contains(&kernel_pc) && interp_pc == ram + (kernel_pc - rom)
            })
    };
    let (mut dispatches, mut steps) = (0u64, 0u64);
    let mut covered = std::collections::HashSet::new();
    let mut history: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let diverge = |what: &str, kernel: &Machine, b: &Machine| -> ! {
        eprintln!("DIVERGED ({what}) at frame {}", kernel.frames);
        eprintln!(
            "  kernel      {} [{}]",
            cpu_line(kernel),
            tcg2_kernel::describe(kernel.cart.rom_bank, kernel.cpu.pc)
        );
        eprintln!(
            "  interpreter {} [{}]",
            cpu_line(b),
            tcg2_kernel::describe(b.cart.rom_bank, b.cpu.pc)
        );
        std::process::exit(1)
    };
    for frame in 0..frames {
        let held = script.buttons(frame);
        for (at, name) in &calls {
            if *at == frame {
                tcg2_kernel::call_routine(&mut kernel.machine, name).expect("--call");
                tcg2_kernel::call_routine(&mut b, name).expect("--call");
            }
        }
        kernel.machine.buttons = held;
        b.buttons = held;
        let mut previous = String::new();
        loop {
            let a = &mut kernel.machine;
            if a.clock >= a.next_event {
                if let Some(fault) = a.fault.take() {
                    eprintln!("kernel fault: {}", tcg2_kernel::KernelFault(fault));
                    eprintln!("the dispatches before it:");
                    for line in &history {
                        eprintln!("  {line}");
                    }
                    std::process::exit(1);
                }
                if let Some(fault) = b.fault.take() {
                    eprintln!(
                        "interpreter fault at {:02x}:{:04x}: {}",
                        fault.bank, fault.pc, fault.reason
                    );
                    std::process::exit(1);
                }
                a.handle_events();
                b.handle_events();
                if a.next_event != b.next_event || a.frame_ready != b.frame_ready {
                    diverge("device events", a, &b);
                }
                if a.frame_ready {
                    a.frame_ready = false;
                    b.frame_ready = false;
                    a.interrupt_check();
                    b.interrupt_check();
                    break;
                }
                let took = a.take_interrupt();
                if took != b.take_interrupt() || !same_cpu(a, &b) {
                    diverge("interrupt", a, &b);
                }
            }
            if a.cpu.halted {
                if !b.cpu.halted {
                    diverge("halt", a, &b);
                }
                a.sleep();
                b.sleep();
                continue;
            }
            let pc = a.cpu.pc;
            covered.insert((if pc < 0x4000 { 0 } else { a.cart.rom_bank }, pc));
            let before = cpu_line(a);
            if history.len() == 40 {
                history.pop_front();
            }
            history.push_back(format!(
                "{before} [{}]",
                tcg2_kernel::describe(a.cart.rom_bank, pc)
            ));
            let next = kernel.code.dispatch(a, pc);
            a.cpu.pc = next;
            dispatches += 1;
            while b.clock < a.clock && b.fault.is_none() {
                tcg2_reference::step(&mut b);
                steps += 1;
            }
            if b.fault.is_none() && (!same_pc(next, b.cpu.pc) || !same_cpu(a, &b)) {
                eprintln!("  previous dispatch began: {previous}");
                eprintln!(
                    "  this dispatch began:     {before} [{}]",
                    tcg2_kernel::describe(a.cart.rom_bank, pc)
                );
                diverge("registers after a dispatch", a, &b);
            }
            previous = before;
        }
        let (ra, rb) = (regions(&kernel.machine), regions(&b));
        for ((name, x), (_, y)) in ra.iter().zip(rb.iter()) {
            if let Some(i) = x.iter().zip(y.iter()).position(|(p, q)| p != q) {
                eprintln!(
                    "DIVERGED (memory) at the end of frame {frame}: {name}[{i:#06x}] kernel {:#04x} interpreter {:#04x}",
                    x[i], y[i]
                );
                std::process::exit(1);
            }
        }
    }
    println!(
        "{frames} frames in lockstep: {dispatches} dispatches ({} distinct entries), {steps} instructions, the machines identical at every frame",
        covered.len()
    );
    let m = &kernel.machine;
    println!(
        "ended at {} [{}]",
        cpu_line(m),
        tcg2_kernel::describe(m.cart.rom_bank, m.cpu.pc)
    );
}
