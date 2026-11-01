//! `tcg2-headless`: the kernel without a window, for tests, agents and
//! measurement.
//!
//! ```text
//! tcg2-headless --rom rom/poketcg2.gbc --frames 900 \
//!     --press 400:start --press 520:a:8 --screenshot out.png --shots 60:shots/
//! ```
//!
//! `--press FRAME:BUTTONS[:FRAMES]` holds `+`-joined buttons (a, b, start,
//! select, up, down, left, right) from that frame for a number of frames
//! (default 6); `--random SEED` adds seeded random play. `--shots N:DIR` writes a PNG every N frames. `--wav PATH`
//! writes the sound. `--sram PATH` loads (and `--save-sram PATH` writes)
//! battery RAM. `--save-state FRAME:PATH` writes a save state after that
//! frame, `--load-state PATH` starts from one. `--call FRAME:ROUTINE` calls one of the game's bank-0
//! routines from its frame wait at that frame (`CallStartupDebugMenu` opens
//! the game's own debug menu, whose sixth item starts a duel against a
//! random opponent); `ROUTINE,a=02,bc=0000` sets registers (hex) first, so
//! `EmptyScreenAndLoadScene,a=02` shows scene 2. `--language TAG` picks the game's language (`en-US`,
//! the default, or `ja`, the ROM's own text); `--switch-language FRAME:TAG`
//! switches to another at that frame, as the app's F9 does. The run reports
//! the host CPU time it took per second of game time.

use std::path::{Path, PathBuf};
use std::time::Instant;
use tcg2_core::{FRAME_RATE, SCREEN_HEIGHT, SCREEN_WIDTH};
use tcg2_kernel::script::Script;
use tcg2_kernel::{Kernel, Language};

struct Options {
    rom: PathBuf,
    frames: u64,
    script: Script,
    calls: Vec<(u64, String)>,
    save_state: Option<(u64, PathBuf)>,
    load_state: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    shots: Option<(u64, PathBuf)>,
    wav: Option<PathBuf>,
    sram: Option<PathBuf>,
    save_sram: Option<PathBuf>,
    language: Language,
    switches: Vec<(u64, Language)>,
    quiet: bool,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        rom: PathBuf::from("rom/poketcg2.gbc"),
        frames: 600,
        script: Script::default(),
        calls: Vec::new(),
        save_state: None,
        load_state: None,
        screenshot: None,
        shots: None,
        wav: None,
        sram: None,
        save_sram: None,
        language: Language::default(),
        switches: Vec::new(),
        quiet: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--rom" => o.rom = value()?.into(),
            "--frames" => o.frames = value()?.parse().map_err(|_| "bad --frames")?,
            "--press" => o.script.presses.push(value()?.parse()?),
            "--save-state" => {
                let v = value()?;
                let (frame, path) = v.split_once(':').ok_or("--save-state FRAME:PATH")?;
                o.save_state = Some((
                    frame.parse().map_err(|_| "bad --save-state frame")?,
                    path.into(),
                ));
            }
            "--load-state" => o.load_state = Some(value()?.into()),
            "--call" => {
                let v = value()?;
                let (frame, name) = v.split_once(':').ok_or("--call FRAME:ROUTINE")?;
                o.calls.push((
                    frame.parse().map_err(|_| "bad --call frame")?,
                    name.to_string(),
                ));
            }
            "--random" => o.script.random = Some(value()?.parse()?),
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--shots" => {
                let v = value()?;
                let (every, dir) = v.split_once(':').ok_or("--shots N:DIR")?;
                o.shots = Some((
                    every.parse().map_err(|_| "bad --shots interval")?,
                    dir.into(),
                ));
            }
            "--wav" => o.wav = Some(value()?.into()),
            "--sram" => o.sram = Some(value()?.into()),
            "--save-sram" => o.save_sram = Some(value()?.into()),
            "--language" => {
                let tag = value()?;
                o.language = Language::parse(&tag).ok_or(format!("unknown language {tag:?}"))?;
            }
            "--switch-language" => {
                let v = value()?;
                let (frame, tag) = v.split_once(':').ok_or("--switch-language FRAME:TAG")?;
                o.switches.push((
                    frame.parse().map_err(|_| "bad --switch-language frame")?,
                    Language::parse(tag).ok_or(format!("unknown language {tag:?}"))?,
                ));
            }
            "--quiet" => o.quiet = true,
            "--version" => {
                println!(
                    "Pokémon Card GB2 {} (headless)",
                    option_env!("TCG2_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {arg:?}")),
        }
    }
    Ok(o)
}

fn write_png(path: &Path, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        SCREEN_WIDTH as u32,
        SCREEN_HEIGHT as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// FNV-1a over the RAM the game observes: work RAM, VRAM, HRAM, battery RAM.
fn memory_hash(m: &tcg2_core::Machine) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for region in [
        &m.wram[..],
        &m.ppu.vram[..],
        &m.hram[..],
        &m.cart.sram[..],
        &m.ppu.oam[..],
    ] {
        for &b in region {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn cpu_time() -> f64 {
    // Process CPU time (user + system), where the platform reports it.
    #[cfg(target_os = "linux")]
    {
        if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
            let fields: Vec<&str> = stat
                .rsplit_once(')')
                .map(|(_, r)| r)
                .unwrap_or("")
                .split_whitespace()
                .collect();
            if fields.len() > 13 {
                let ticks: f64 = fields[11].parse::<f64>().unwrap_or(0.0)
                    + fields[12].parse::<f64>().unwrap_or(0.0);
                return ticks / 100.0;
            }
        }
    }
    0.0
}

fn main() {
    let mut o = options().unwrap_or_else(|e| {
        eprintln!("tcg2-headless: {e}");
        std::process::exit(2)
    });
    let rom = std::fs::read(&o.rom).unwrap_or_else(|e| {
        eprintln!("tcg2-headless: {}: {e}", o.rom.display());
        std::process::exit(1)
    });
    let rate = 48_000;
    let mut kernel = Kernel::new(rom, rate, o.language).unwrap_or_else(|e| {
        eprintln!("tcg2-headless: {e}");
        std::process::exit(1)
    });
    if let Some(path) = &o.sram
        && let Ok(bytes) = std::fs::read(path)
    {
        kernel.load_sram(&bytes);
    }
    if let Some(path) = &o.load_state {
        let bytes = std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("tcg2-headless: {}: {e}", path.display());
            std::process::exit(1)
        });
        kernel.machine.load_state(&bytes).unwrap_or_else(|e| {
            eprintln!("tcg2-headless: {}: {e}", path.display());
            std::process::exit(1)
        });
    }
    let mut audio = Vec::new();
    let mut changed_frames = 0u64;
    let started = Instant::now();
    let cpu_started = cpu_time();
    let mut result = Ok(());
    for frame in 0..o.frames {
        let held = o.script.buttons(frame);
        for (at, name) in &o.calls {
            if *at == frame {
                tcg2_kernel::call_routine(&mut kernel.machine, name)
                    .unwrap_or_else(|e| eprintln!("--call {name}: {e}"));
            }
        }
        for &(at, language) in &o.switches {
            if at == frame {
                kernel
                    .set_language(language)
                    .unwrap_or_else(|e| eprintln!("--switch-language: {e}"));
            }
        }
        kernel.set_buttons(held);
        if let Err(fault) = kernel.run_frame() {
            result = Err(fault);
            break;
        }
        if kernel.take_picture_changed() {
            changed_frames += 1;
        }
        if let Some((at, path)) = &o.save_state
            && *at == frame + 1
        {
            std::fs::write(path, kernel.machine.save_state()).unwrap_or_else(|e| eprintln!("{e}"));
        }
        if o.wav.is_some() {
            kernel.drain_audio(&mut audio);
        } else {
            let mut discard = Vec::new();
            kernel.drain_audio(&mut discard);
        }
        if let Some((every, dir)) = &o.shots
            && (frame + 1) % every == 0
        {
            let path = dir.join(format!("{:06}.png", frame + 1));
            write_png(&path, kernel.framebuffer()).unwrap_or_else(|e| eprintln!("{e}"));
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let cpu = cpu_time() - cpu_started;
    let frames = kernel.machine.frames;
    if let Some(path) = &o.screenshot {
        write_png(path, kernel.framebuffer()).unwrap_or_else(|e| eprintln!("{e}"));
    }
    if let Some(path) = &o.wav {
        write_wav(path, &audio, rate).unwrap_or_else(|e| eprintln!("{e}"));
    }
    if let Some(path) = &o.save_sram {
        std::fs::write(path, kernel.sram()).unwrap_or_else(|e| eprintln!("{e}"));
    }
    if !o.quiet {
        let game_seconds = frames as f64 / FRAME_RATE;
        println!(
            "{frames} frames ({game_seconds:.1} s of game time) in {wall:.3} s wall, {cpu:.2} s CPU: \
             {:.2} ms of host time per game second; the picture changed in {changed_frames} frames",
            1000.0 * wall / game_seconds.max(1e-9)
        );
        let m = &kernel.machine;
        println!(
            "pc {:04x} bank {:02x} sp {:04x} a {:02x} lcdc {:02x} halted {} ime {} clock {} memory {:016x}",
            m.cpu.pc,
            m.cart.rom_bank,
            m.cpu.sp,
            m.cpu.a,
            m.ppu.lcdc,
            m.cpu.halted,
            m.cpu.ime,
            m.clock,
            memory_hash(m)
        );
    }
    if let Err(fault) = result {
        eprintln!("tcg2-headless: {fault}");
        std::process::exit(3);
    }
}
