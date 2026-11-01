//! The Game Boy Color hardware the Pokémon TCG 2 kernel runs on, modelled as
//! host services rather than emulated: no CPU is interpreted here and no
//! device is stepped cycle by cycle. The kernel (the original program,
//! translated to Rust from src/decomp) drives a [`Machine`]: it adds each
//! instruction's cycles to one clock, and the devices compute their state
//! from that clock when the game reads or writes them.

pub mod apu;
pub mod cartridge;
pub mod cpu;
pub mod machine;
pub mod ppu;
pub mod state;
pub mod timer;

pub use cartridge::{ROM_SHA1, ROM_SIZE, RomError, verify_rom};
pub use cpu::Cpu;
pub use machine::{CLOCK_HZ, Fault, Machine, buttons};
pub use ppu::{FRAME_UNITS, SCREEN_HEIGHT, SCREEN_WIDTH};

/// The console's frame rate: 8388608 / 140448 Hz.
pub const FRAME_RATE: f64 = CLOCK_HZ as f64 / FRAME_UNITS as f64;
