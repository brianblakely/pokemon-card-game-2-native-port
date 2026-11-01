//! rgblink's symbol file: every label of the matched link, with its bank.

use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub bank: u16,
    pub addr: u16,
    pub name: String,
}

impl Symbol {
    /// A global label (`Name`) rather than a local one (`Parent.local`).
    pub fn is_global(&self) -> bool {
        !self.name.contains('.')
    }
}

#[derive(Default, Debug)]
pub struct Symbols {
    pub all: Vec<Symbol>,
    /// ROM labels by (bank, address), first-defined first.
    rom: BTreeMap<(u16, u16), Vec<usize>>,
    by_name: HashMap<String, usize>,
    /// RAM labels by address (all WRAM/SRAM banks), for operand comments.
    ram: BTreeMap<u16, Vec<usize>>,
}

impl Symbols {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut symbols = Symbols::default();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            let bad = || format!("symbol file line {}: {line:?}", number + 1);
            let (location, name) = line.split_once(' ').ok_or_else(bad)?;
            // Exported constants are listed as `VALUE NAME`, without a bank.
            let Some((bank, addr)) = location.split_once(':') else {
                continue;
            };
            let bank = u16::from_str_radix(bank, 16).map_err(|_| bad())?;
            let addr = u16::from_str_radix(addr, 16).map_err(|_| bad())?;
            let index = symbols.all.len();
            symbols.all.push(Symbol {
                bank,
                addr,
                name: name.to_string(),
            });
            symbols.by_name.entry(name.to_string()).or_insert(index);
            if addr < 0x8000 {
                symbols.rom.entry((bank, addr)).or_default().push(index);
            } else {
                symbols.ram.entry(addr).or_default().push(index);
            }
        }
        Ok(symbols)
    }

    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.by_name.get(name).map(|&i| &self.all[i])
    }

    /// The labels at a ROM location.
    pub fn at(&self, bank: u16, addr: u16) -> impl Iterator<Item = &Symbol> {
        self.rom
            .get(&(bank, addr))
            .into_iter()
            .flatten()
            .map(|&i| &self.all[i])
    }

    /// The best name for a ROM location: a global label if there is one.
    pub fn name_at(&self, bank: u16, addr: u16) -> Option<&str> {
        let mut best: Option<&Symbol> = None;
        for symbol in self.at(bank, addr) {
            if best.is_none() || (symbol.is_global() && !best.unwrap().is_global()) {
                best = Some(symbol);
            }
        }
        best.map(|s| s.name.as_str())
    }

    /// The global label at or before `addr` in `bank`'s ROM.
    pub fn owner(&self, bank: u16, addr: u16) -> Option<&Symbol> {
        let start = if bank == 0 { 0 } else { 0x4000 };
        self.rom
            .range((bank, start)..=(bank, addr))
            .rev()
            .flat_map(|(_, list)| list.iter().map(|&i| &self.all[i]))
            .find(|s| s.is_global())
    }

    /// A RAM label for an operand comment, if the address has exactly one
    /// name across its banks (or one in bank 0 / HRAM).
    pub fn ram_name(&self, addr: u16) -> Option<&str> {
        let list = self.ram.get(&addr)?;
        let globals: Vec<&Symbol> = list
            .iter()
            .map(|&i| &self.all[i])
            .filter(|s| s.is_global())
            .collect();
        let banks: std::collections::BTreeSet<u16> = globals.iter().map(|s| s.bank).collect();
        if banks.len() == 1 {
            globals.first().map(|s| s.name.as_str())
        } else {
            None
        }
    }
}
