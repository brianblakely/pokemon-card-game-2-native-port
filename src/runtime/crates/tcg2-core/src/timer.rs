//! DIV, TIMA, TMA and TAC, computed from the clock: TIMA's overflow is a
//! scheduled event, never a counter stepped in a loop. The game's timer
//! interrupt (about 241 Hz) runs its link and sound engines.

/// TIMA's period in CPU clocks for each TAC clock select.
const PERIODS: [u64; 4] = [1024, 16, 64, 256];

#[derive(Clone, Debug)]
pub struct Timer {
    /// The clock (in units) at which the 16-bit DIV counter was zero.
    div_base: u64,
    /// log2 of units per CPU clock: 1 at normal speed, 0 at double speed.
    cpu_shift: u32,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
    /// The clock at which `tima` was last brought up to date.
    synced_at: u64,
    pub next_overflow: u64,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            div_base: 0,
            cpu_shift: 1,
            tima: 0,
            tma: 0,
            tac: 0xf8,
            synced_at: 0,
            next_overflow: u64::MAX,
        }
    }
}

impl Timer {
    #[inline]
    fn counter(&self, clock: u64) -> u64 {
        (clock.saturating_sub(self.div_base)) >> self.cpu_shift
    }

    fn enabled(&self) -> bool {
        self.tac & 0x04 != 0
    }

    fn period(&self) -> u64 {
        PERIODS[(self.tac & 3) as usize]
    }

    pub fn div(&self, clock: u64) -> u8 {
        (self.counter(clock) >> 8) as u8
    }

    /// Brings TIMA up to `clock`; returns whether it overflowed on the way.
    fn sync(&mut self, clock: u64) -> bool {
        let mut overflowed = false;
        if self.enabled() && clock > self.synced_at {
            let p = self.period();
            let mut ticks = self.counter(clock) / p - self.counter(self.synced_at) / p;
            while ticks > 0 {
                let to_overflow = 256 - self.tima as u64;
                if ticks >= to_overflow {
                    ticks -= to_overflow;
                    self.tima = self.tma;
                    overflowed = true;
                } else {
                    self.tima += ticks as u8;
                    ticks = 0;
                }
            }
        }
        self.synced_at = clock;
        overflowed
    }

    fn schedule(&mut self) {
        self.next_overflow = u64::MAX;
        if !self.enabled() {
            return;
        }
        let p = self.period();
        let ticks = 256 - self.tima as u64;
        let target = (self.counter(self.synced_at) / p + ticks) * p;
        self.next_overflow = self.div_base + (target << self.cpu_shift);
    }

    pub fn read_tima(&mut self, clock: u64) -> u8 {
        self.sync(clock);
        self.tima
    }

    pub fn write(&mut self, reg: u8, value: u8, clock: u64) {
        self.sync(clock);
        match reg {
            0x04 => {
                self.div_base = clock;
                self.synced_at = clock;
            }
            0x05 => self.tima = value,
            0x06 => self.tma = value,
            0x07 => self.tac = 0xf8 | (value & 7),
            _ => unreachable!(),
        }
        self.schedule();
    }

    /// The overflow due by `clock`: returns whether the timer interrupt is
    /// raised.
    pub fn on_events(&mut self, clock: u64) -> bool {
        if clock < self.next_overflow {
            return false;
        }
        let at = self.next_overflow;
        self.sync(at);
        // `sync` reloaded TMA at the overflow.
        self.schedule();
        let more = self.sync(clock);
        if more {
            self.schedule();
        }
        true
    }

    /// The CPU's speed changed (`stop`), which also resets DIV.
    pub fn set_speed(&mut self, double: bool, clock: u64) {
        self.sync(clock);
        self.cpu_shift = if double { 0 } else { 1 };
        self.div_base = clock;
        self.synced_at = clock;
        self.schedule();
    }
}

impl Timer {
    pub(crate) fn save(&self, w: &mut crate::state::Writer) {
        w.u64(self.div_base);
        w.u32(self.cpu_shift);
        w.u8(self.tima);
        w.u8(self.tma);
        w.u8(self.tac);
        w.u64(self.synced_at);
        w.u64(self.next_overflow);
    }

    pub(crate) fn load(
        &mut self,
        r: &mut crate::state::Reader,
    ) -> Result<(), crate::state::StateError> {
        self.div_base = r.u64()?;
        self.cpu_shift = r.u32()?;
        self.tima = r.u8()?;
        self.tma = r.u8()?;
        self.tac = r.u8()?;
        self.synced_at = r.u64()?;
        self.next_overflow = r.u64()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_period_matches_the_game_setup() {
        // SetupTimer at double speed: TMA = -136, 16384 Hz select (doubled).
        let mut timer = Timer::default();
        timer.set_speed(true, 0);
        timer.write(0x06, (-136i16) as u8, 0);
        timer.write(0x07, 0x07, 0);
        let mut overflows = Vec::new();
        let mut clock = 0;
        while overflows.len() < 3 {
            clock = timer.next_overflow;
            assert!(timer.on_events(clock));
            overflows.push(clock);
        }
        // The first overflow counts up from TIMA = 0; then every 136 ticks of
        // 256 CPU clocks (one unit each at double speed): 240.94 Hz.
        assert_eq!(overflows[2] - overflows[1], 136 * 256);
        assert_eq!(overflows[1] - overflows[0], 136 * 256);
        assert!(clock > 0);
    }
}
