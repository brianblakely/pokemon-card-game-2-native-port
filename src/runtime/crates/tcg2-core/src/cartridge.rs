//! The cartridge: the player's own ROM image behind an MBC5 (ROM banks,
//! 32 KiB of battery-backed RAM), the only source of the game's proprietary
//! data. The kernel compiled into the binary is code alone; every graphic,
//! text, card and song it shows is read from this image at run time.

use sha1::{Digest, Sha1};
use std::fmt;
use std::sync::Arc;

pub const ROM_SIZE: usize = 0x20_0000;
pub const SRAM_SIZE: usize = 0x8000;
/// SHA-1 of "Pokémon Card GB2 - GR Dan Sanjou! (J)", the ROM src/decomp
/// rebuilds byte for byte (src/decomp/rom.sha1).
pub const ROM_SHA1: &str = "a7e12bcc5f514e3aad8de570fd511aab0a308822";

#[derive(Debug)]
pub enum RomError {
    Size(usize),
    Digest(String),
}

impl fmt::Display for RomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RomError::Size(size) => write!(
                f,
                "the ROM is {size} bytes; Pokémon Card GB2 is {ROM_SIZE} (2 MiB)"
            ),
            RomError::Digest(digest) => write!(
                f,
                "the ROM's SHA-1 is {digest}, not {ROM_SHA1} (Pokémon Card GB2 - GR Dan Sanjou! (J)); \
                 use an unmodified dump of your own cartridge"
            ),
        }
    }
}

impl std::error::Error for RomError {}

pub fn sha1_hex(bytes: &[u8]) -> String {
    let digest = Sha1::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks that `bytes` is the ROM the kernel was translated from.
pub fn verify_rom(bytes: &[u8]) -> Result<(), RomError> {
    if bytes.len() != ROM_SIZE {
        return Err(RomError::Size(bytes.len()));
    }
    let digest = sha1_hex(bytes);
    if digest != ROM_SHA1 {
        return Err(RomError::Digest(digest));
    }
    Ok(())
}

#[derive(Clone)]
pub struct Cartridge {
    pub rom: Arc<[u8]>,
    /// Byte offset of the bank mapped at $4000-$7FFF.
    pub rom_offset: usize,
    pub rom_bank: u16,
    pub sram: Box<[u8]>,
    pub sram_bank: u8,
    pub sram_enabled: bool,
    /// Set by every write that changes battery RAM: the host saves it.
    pub sram_dirty: bool,
    /// ROM bank changes so far (for tests).
    pub bank_switches: u64,
}

impl Cartridge {
    pub fn new(rom: Arc<[u8]>) -> Self {
        assert!(
            rom.len() >= 0x8000 && rom.len().is_power_of_two(),
            "ROM size"
        );
        Self {
            rom,
            rom_offset: 0x4000,
            rom_bank: 1,
            sram: vec![0xff; SRAM_SIZE].into_boxed_slice(),
            sram_bank: 0,
            sram_enabled: false,
            sram_dirty: false,
            bank_switches: 0,
        }
    }

    /// Writes to $0000-$7FFF: the MBC5's registers.
    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1fff => self.sram_enabled = value & 0x0f == 0x0a,
            0x2000..=0x2fff => self.set_rom_bank((self.rom_bank & 0x100) | value as u16),
            0x3000..=0x3fff => {
                self.set_rom_bank((self.rom_bank & 0xff) | ((value as u16 & 1) << 8))
            }
            0x4000..=0x5fff => self.sram_bank = value & 0x0f,
            _ => {}
        }
    }

    fn set_rom_bank(&mut self, bank: u16) {
        if bank != self.rom_bank {
            self.bank_switches += 1;
        }
        self.rom_bank = bank;
        let banks = self.rom.len() / 0x4000;
        self.rom_offset = (bank as usize % banks) * 0x4000;
    }

    #[inline(always)]
    fn sram_index(&self, addr: u16) -> usize {
        ((self.sram_bank as usize & 3) << 13) | (addr as usize & 0x1fff)
    }

    #[inline]
    pub fn read_sram(&self, addr: u16) -> u8 {
        if self.sram_enabled {
            self.sram[self.sram_index(addr)]
        } else {
            0xff
        }
    }

    #[inline]
    pub fn write_sram(&mut self, addr: u16, value: u8) {
        if self.sram_enabled {
            let i = self.sram_index(addr);
            if self.sram[i] != value {
                self.sram[i] = value;
                self.sram_dirty = true;
            }
        }
    }

    pub(crate) fn save(&self, w: &mut crate::state::Writer) {
        w.u16(self.rom_bank);
        w.bytes(&self.sram);
        w.u8(self.sram_bank);
        w.bool(self.sram_enabled);
    }

    pub(crate) fn load(
        &mut self,
        r: &mut crate::state::Reader,
    ) -> Result<(), crate::state::StateError> {
        let bank = r.u16()?;
        self.set_rom_bank(bank);
        r.fill(&mut self.sram)?;
        self.sram_bank = r.u8()?;
        self.sram_enabled = r.bool()?;
        // The loaded RAM is what the host's battery save should now hold.
        self.sram_dirty = true;
        Ok(())
    }

    /// Loads a battery save (a raw 32 KiB image, as emulators write them).
    pub fn load_sram(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.sram.len());
        self.sram[..n].copy_from_slice(&bytes[..n]);
        self.sram_dirty = false;
    }
}
