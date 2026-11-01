//! The sound unit as a synthesizer driven by the game's register writes.
//!
//! Nothing runs per CPU cycle, and nothing runs per channel per output
//! sample. Each channel walks its own timer from one level change to the
//! next and records only the changes, as steps at their exact time; the
//! output is the running sum of those steps, averaged over each sample's
//! interval (a box filter: a step inside a sample counts for the part of it
//! that follows). A write first renders everything before its clock with the
//! old state; the host takes the rest at the end of each frame.
//!
//! Cost: one step per audible waveform edge, plus an add and the output
//! filter per sample.

const CLOCK_HZ: u64 = 8_388_608;
/// Clock units per APU clock (the sound unit runs at 4194304 Hz at both CPU
/// speeds).
const APU_UNIT: u64 = 2;
/// The frame sequencer's 512 Hz step.
const SEQUENCER_UNITS: u64 = CLOCK_HZ / 512;

const DUTY: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];
const NOISE_DIVISORS: [u64; 8] = [8, 16, 32, 48, 64, 80, 96, 112];
/// Bits that read back as 1 in $FF10-$FF2F.
const READ_MASK: [u8; 0x20] = [
    0x80, 0x3f, 0x00, 0xff, 0xbf, 0xff, 0x3f, 0x00, 0xff, 0xbf, 0x7f, 0xff, 0x9f, 0xff, 0xbf, 0xff,
    0xff, 0x00, 0x00, 0xbf, 0x00, 0x00, 0x70, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
];

#[derive(Clone, Debug, Default)]
struct Envelope {
    volume: u8,
    up: bool,
    period: u8,
    timer: u8,
}

impl Envelope {
    fn load(&mut self, nrx2: u8) {
        self.volume = nrx2 >> 4;
        self.up = nrx2 & 0x08 != 0;
        self.period = nrx2 & 7;
        self.timer = self.period;
    }

    fn step(&mut self) {
        if self.period == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period;
            if self.up && self.volume < 15 {
                self.volume += 1;
            } else if !self.up && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Channel {
    on: bool,
    dac: bool,
    length: u16,
    length_enabled: bool,
    frequency: u16,
    /// The clock of this channel's next timer step.
    next_step: u64,
    /// Duty step (squares), sample position (wave).
    position: u8,
    envelope: Envelope,
    // Square 1's sweep.
    sweep_shadow: u16,
    sweep_timer: u8,
    sweep_enabled: bool,
    // Noise.
    lfsr: u16,
}

#[derive(Clone)]
pub struct Apu {
    pub regs: [u8; 0x30],
    power: bool,
    ch: [Channel; 4],
    sequencer_step: u8,
    next_sequencer: u64,
    /// Everything before this clock has been rendered.
    time: u64,
    sample_rate: u64,
    /// The clock that sample 0 starts at, and the samples output since.
    origin: u64,
    samples: u64,
    /// Each channel's digital level now (0-15; 0 while off).
    level: [u8; 4],
    /// Each channel's output now (left, right), its DAC and the mixer applied.
    contribution: [[f32; 2]; 4],
    /// Steps to the output, by sample from `samples` on.
    deltas: Vec<[f32; 2]>,
    /// The output's current value (the sum of all steps so far).
    sum: [f32; 2],
    capacitor: [f32; 2],
    charge: f32,
    /// Interleaved stereo samples, drained by the host.
    pub output: Vec<f32>,
    /// The output is exactly silent (no DAC on): hosts may idle their device.
    pub silent: bool,
}

impl Apu {
    pub fn new(sample_rate: u32) -> Self {
        let mut apu = Self {
            regs: [0; 0x30],
            power: true,
            ch: Default::default(),
            sequencer_step: 0,
            next_sequencer: SEQUENCER_UNITS,
            time: 0,
            sample_rate: sample_rate as u64,
            origin: 0,
            samples: 0,
            level: [0; 4],
            contribution: [[0.0; 2]; 4],
            deltas: Vec::with_capacity(2048),
            sum: [0.0; 2],
            capacitor: [0.0; 2],
            charge: 0.999958f32.powf(4_194_304.0 / sample_rate as f32),
            output: Vec::with_capacity(4096),
            silent: true,
        };
        apu.regs[0x16] = 0xf1;
        apu
    }

    /// The clock at which output sample `index` ends.
    fn sample_end(&self, index: u64) -> u64 {
        self.origin + (index + 1) * CLOCK_HZ / self.sample_rate
    }

    /// Records a step of `delta` in the output at clock `t` (not before the
    /// sample being built).
    #[inline]
    fn step_output(&mut self, t: u64, delta: [f32; 2]) {
        let position = (t - self.origin) as u128 * self.sample_rate as u128;
        let sample = (position / CLOCK_HZ as u128) as u64;
        let fraction = (position % CLOCK_HZ as u128) as f32 / CLOCK_HZ as f32;
        let i = sample.saturating_sub(self.samples) as usize;
        if self.deltas.len() < i + 2 {
            self.deltas.resize(i + 2, [0.0; 2]);
        }
        for (side, d) in delta.iter().enumerate() {
            self.deltas[i][side] += d * (1.0 - fraction);
            self.deltas[i + 1][side] += d * fraction;
        }
    }

    /// The mixer's gain for each channel (panning, master volume, scale).
    fn gains(&self, ch: usize) -> [f32; 2] {
        let nr50 = self.regs[0x14];
        let nr51 = self.regs[0x15];
        let left = if nr51 & (0x10 << ch) != 0 {
            ((nr50 >> 4) & 7) as f32 + 1.0
        } else {
            0.0
        };
        let right = if nr51 & (0x01 << ch) != 0 {
            (nr50 & 7) as f32 + 1.0
        } else {
            0.0
        };
        [left / 32.0, right / 32.0]
    }

    fn wave_level(&self, position: u8) -> u8 {
        let byte = self.regs[0x20 + (position as usize >> 1)];
        let sample = if position & 1 == 0 {
            byte >> 4
        } else {
            byte & 0x0f
        };
        match (self.regs[0x0c] >> 5) & 3 {
            0 => 0,
            1 => sample,
            2 => sample >> 1,
            _ => sample >> 2,
        }
    }

    /// The channel's level now, from its state.
    fn current_level(&self, ch: usize) -> u8 {
        let c = &self.ch[ch];
        if !c.on {
            return 0;
        }
        match ch {
            0 | 1 => {
                let duty = DUTY[(self.regs[if ch == 0 { 0x01 } else { 0x06 }] >> 6) as usize];
                if duty >> (7 - c.position) & 1 != 0 {
                    c.envelope.volume
                } else {
                    0
                }
            }
            2 => self.wave_level(c.position),
            _ => {
                if c.lfsr & 1 == 0 {
                    c.envelope.volume
                } else {
                    0
                }
            }
        }
    }

    fn output_of(&self, ch: usize, level: u8) -> [f32; 2] {
        if !self.ch[ch].dac {
            return [0.0; 2];
        }
        let analog = level as f32 / 7.5 - 1.0;
        let g = self.gains(ch);
        [analog * g[0], analog * g[1]]
    }

    /// Re-derives every channel's output at `t` (after a write or a frame
    /// sequencer step) and records the changes.
    fn refresh(&mut self, t: u64) {
        for ch in 0..4 {
            self.level[ch] = self.current_level(ch);
            let new = self.output_of(ch, self.level[ch]);
            let old = self.contribution[ch];
            if new != old {
                self.contribution[ch] = new;
                self.step_output(t, [new[0] - old[0], new[1] - old[1]]);
            }
        }
        self.silent = !self.ch.iter().any(|c| c.dac);
    }

    fn period(&self, ch: usize) -> u64 {
        match ch {
            0 | 1 => (2048 - self.ch[ch].frequency as u64) * 4 * APU_UNIT,
            2 => (2048 - self.ch[2].frequency as u64) * 2 * APU_UNIT,
            _ => {
                let nr43 = self.regs[0x12];
                (NOISE_DIVISORS[(nr43 & 7) as usize] << (nr43 >> 4)) * APU_UNIT
            }
        }
    }

    /// Walks one channel's timer through [`time`, `end`), recording each
    /// change of its level.
    fn run_channel(&mut self, ch: usize, end: u64) {
        if !self.ch[ch].on || self.ch[ch].next_step >= end {
            return;
        }
        let period = self.period(ch);
        // Output per unit of digital level, through the DAC and the mixer.
        let g = self.gains(ch);
        let unit = if self.ch[ch].dac {
            [g[0] / 7.5, g[1] / 7.5]
        } else {
            [0.0; 2]
        };
        let duty = DUTY[(self.regs[if ch == 0 { 0x01 } else { 0x06 }] >> 6) as usize];
        let narrow = self.regs[0x12] & 0x08 != 0;
        let mut level = self.level[ch];
        while self.ch[ch].next_step < end {
            let t = self.ch[ch].next_step;
            self.ch[ch].next_step += period;
            let next = match ch {
                0 | 1 => {
                    let c = &mut self.ch[ch];
                    c.position = (c.position + 1) & 7;
                    if duty >> (7 - c.position) & 1 != 0 {
                        c.envelope.volume
                    } else {
                        0
                    }
                }
                2 => {
                    let p = (self.ch[2].position + 1) & 31;
                    self.ch[2].position = p;
                    self.wave_level(p)
                }
                _ => {
                    let c = &mut self.ch[3];
                    let bit = (c.lfsr ^ (c.lfsr >> 1)) & 1;
                    c.lfsr = (c.lfsr >> 1) | (bit << 14);
                    if narrow {
                        c.lfsr = (c.lfsr & !0x40) | (bit << 6);
                    }
                    if c.lfsr & 1 == 0 {
                        c.envelope.volume
                    } else {
                        0
                    }
                }
            };
            if next != level {
                let d = next as f32 - level as f32;
                self.step_output(t, [d * unit[0], d * unit[1]]);
                level = next;
            }
        }
        self.level[ch] = level;
        self.contribution[ch] = self.output_of(ch, level);
    }

    /// Outputs every sample that ends by `self.time`.
    fn emit(&mut self) {
        let mut done = 0;
        while self.sample_end(self.samples) <= self.time {
            if let Some(delta) = self.deltas.get(done) {
                self.sum[0] += delta[0];
                self.sum[1] += delta[1];
            }
            for side in 0..2 {
                let v = self.sum[side];
                // The console's output capacitor (a high-pass filter).
                let filtered = v - self.capacitor[side];
                self.capacitor[side] = v - filtered * self.charge;
                self.output.push(filtered);
            }
            self.samples += 1;
            done += 1;
        }
        let done = done.min(self.deltas.len());
        self.deltas.drain(..done);
    }

    fn sequencer(&mut self) {
        let step = self.sequencer_step;
        self.sequencer_step = (step + 1) & 7;
        if step & 1 == 0 {
            for c in &mut self.ch {
                if c.length_enabled && c.length > 0 {
                    c.length -= 1;
                    if c.length == 0 {
                        c.on = false;
                    }
                }
            }
        }
        if step == 2 || step == 6 {
            self.sweep();
        }
        if step == 7 {
            for ch in [0, 1, 3] {
                self.ch[ch].envelope.step();
            }
        }
    }

    fn sweep_target(&mut self) -> u16 {
        let nr10 = self.regs[0x00];
        let c = &self.ch[0];
        let delta = c.sweep_shadow >> (nr10 & 7);
        let target = if nr10 & 0x08 != 0 {
            c.sweep_shadow.wrapping_sub(delta)
        } else {
            c.sweep_shadow + delta
        };
        if target > 2047 {
            self.ch[0].on = false;
        }
        target
    }

    fn sweep(&mut self) {
        let nr10 = self.regs[0x00];
        let period = (nr10 >> 4) & 7;
        let c = &mut self.ch[0];
        c.sweep_timer = c.sweep_timer.saturating_sub(1);
        if c.sweep_timer != 0 {
            return;
        }
        c.sweep_timer = if period == 0 { 8 } else { period };
        if !c.sweep_enabled || period == 0 {
            return;
        }
        let target = self.sweep_target();
        if target <= 2047 && nr10 & 7 != 0 {
            self.ch[0].frequency = target;
            self.ch[0].sweep_shadow = target;
            self.sweep_target();
        }
    }

    /// Renders the output up to `clock`.
    pub fn sync(&mut self, clock: u64) {
        while self.time < clock {
            let end = clock.min(self.next_sequencer);
            for ch in 0..4 {
                self.run_channel(ch, end);
            }
            self.time = end;
            if self.next_sequencer <= end {
                self.sequencer();
                self.next_sequencer += SEQUENCER_UNITS;
                self.refresh(end);
            }
            self.emit();
        }
        // The running sum plus the steps still pending is the output now:
        // re-derive it so rounding cannot accumulate.
        for side in 0..2 {
            let now: f32 = self.contribution.iter().map(|c| c[side]).sum();
            let pending: f32 = self.deltas.iter().map(|d| d[side]).sum();
            self.sum[side] = now - pending;
        }
    }

    pub fn read(&mut self, reg: u8, clock: u64) -> u8 {
        let i = (reg - 0x10) as usize;
        if i >= 0x20 {
            return self.regs[i];
        }
        if i == 0x16 {
            self.sync(clock);
            let mut v = 0x70 | if self.power { 0x80 } else { 0 };
            for ch in 0..4 {
                if self.ch[ch].on {
                    v |= 1 << ch;
                }
            }
            return v;
        }
        self.regs[i] | READ_MASK[i]
    }

    pub fn write(&mut self, reg: u8, value: u8, clock: u64) {
        self.sync(clock);
        self.apply(reg, value, clock);
        self.refresh(clock);
    }

    fn apply(&mut self, reg: u8, value: u8, clock: u64) {
        let i = (reg - 0x10) as usize;
        if i >= 0x20 {
            self.regs[i] = value;
            return;
        }
        if i == 0x16 {
            let on = value & 0x80 != 0;
            if self.power && !on {
                self.regs[..0x16].fill(0);
                for c in &mut self.ch {
                    *c = Channel::default();
                }
            } else if !self.power && on {
                self.sequencer_step = 0;
            }
            self.power = on;
            self.regs[0x16] = value & 0x80;
            return;
        }
        if !self.power {
            return;
        }
        self.regs[i] = value;
        let ch = i / 5;
        match i {
            0x01 | 0x06 | 0x0b | 0x10 => {
                self.ch[ch].length = if ch == 2 {
                    256 - value as u16
                } else {
                    64 - (value & 63) as u16
                };
            }
            0x02 | 0x07 | 0x11 => {
                self.ch[ch].dac = value & 0xf8 != 0;
                if !self.ch[ch].dac {
                    self.ch[ch].on = false;
                }
            }
            0x0a => {
                self.ch[2].dac = value & 0x80 != 0;
                if !self.ch[2].dac {
                    self.ch[2].on = false;
                }
            }
            0x03 | 0x08 | 0x0d => {
                self.ch[ch].frequency = (self.ch[ch].frequency & 0x700) | value as u16;
            }
            0x04 | 0x09 | 0x0e | 0x13 => {
                if ch != 3 {
                    self.ch[ch].frequency =
                        (self.ch[ch].frequency & 0xff) | ((value as u16 & 7) << 8);
                }
                self.ch[ch].length_enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.trigger(ch, clock);
                }
            }
            _ => {}
        }
    }

    fn trigger(&mut self, ch: usize, clock: u64) {
        let nrx2 = [self.regs[0x02], self.regs[0x07], 0, self.regs[0x11]][ch];
        let period = self.period(ch);
        let c = &mut self.ch[ch];
        c.on = c.dac;
        if c.length == 0 {
            c.length = if ch == 2 { 256 } else { 64 };
        }
        c.next_step = clock + period;
        if ch != 2 {
            c.envelope.load(nrx2);
        }
        match ch {
            2 => c.position = 0,
            3 => c.lfsr = 0x7fff,
            _ => {}
        }
        if ch == 0 {
            let nr10 = self.regs[0x00];
            let period = (nr10 >> 4) & 7;
            let c = &mut self.ch[0];
            c.sweep_shadow = c.frequency;
            c.sweep_timer = if period == 0 { 8 } else { period };
            c.sweep_enabled = period != 0 || nr10 & 7 != 0;
            if nr10 & 7 != 0 {
                self.sweep_target();
            }
        }
    }

    /// Takes the samples rendered so far.
    pub fn drain(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.output);
    }

    pub(crate) fn save(&self, w: &mut crate::state::Writer) {
        w.bytes(&self.regs);
        w.bool(self.power);
        for c in &self.ch {
            w.bool(c.on);
            w.bool(c.dac);
            w.u16(c.length);
            w.bool(c.length_enabled);
            w.u16(c.frequency);
            w.u64(c.next_step);
            w.u8(c.position);
            w.u8(c.envelope.volume);
            w.bool(c.envelope.up);
            w.u8(c.envelope.period);
            w.u8(c.envelope.timer);
            w.u16(c.sweep_shadow);
            w.u8(c.sweep_timer);
            w.bool(c.sweep_enabled);
            w.u16(c.lfsr);
        }
        w.u8(self.sequencer_step);
        w.u64(self.next_sequencer);
        w.u64(self.time);
        w.bytes(&self.level);
        for c in &self.contribution {
            w.f32(c[0]);
            w.f32(c[1]);
        }
        w.bool(self.silent);
    }

    /// Loads a state saved at `time`; output restarts from there (what was
    /// pending in the old output is dropped).
    pub(crate) fn load(
        &mut self,
        r: &mut crate::state::Reader,
    ) -> Result<(), crate::state::StateError> {
        r.fill(&mut self.regs)?;
        self.power = r.bool()?;
        for c in &mut self.ch {
            c.on = r.bool()?;
            c.dac = r.bool()?;
            c.length = r.u16()?;
            c.length_enabled = r.bool()?;
            c.frequency = r.u16()?;
            c.next_step = r.u64()?;
            c.position = r.u8()?;
            c.envelope.volume = r.u8()?;
            c.envelope.up = r.bool()?;
            c.envelope.period = r.u8()?;
            c.envelope.timer = r.u8()?;
            c.sweep_shadow = r.u16()?;
            c.sweep_timer = r.u8()?;
            c.sweep_enabled = r.bool()?;
            c.lfsr = r.u16()?;
        }
        self.sequencer_step = r.u8()?;
        self.next_sequencer = r.u64()?;
        self.time = r.u64()?;
        r.fill(&mut self.level)?;
        for c in &mut self.contribution {
            c[0] = r.f32()?;
            c[1] = r.f32()?;
        }
        self.silent = r.bool()?;
        self.origin = self.time;
        self.samples = 0;
        self.deltas.clear();
        self.output.clear();
        for side in 0..2 {
            self.sum[side] = self.contribution.iter().map(|c| c[side]).sum();
            self.capacitor[side] = self.sum[side];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_renders_its_share_of_samples() {
        let mut apu = Apu::new(48_000);
        apu.sync(CLOCK_HZ);
        assert_eq!(apu.output.len(), 2 * 48_000);
    }

    #[test]
    fn a_triggered_square_is_heard_at_its_pitch() {
        let mut apu = Apu::new(48_000);
        apu.write(0x24, 0x77, 0);
        apu.write(0x25, 0xff, 0);
        apu.write(0x26, 0x80, 0);
        apu.write(0x16, 0x80, 0); // NR21: 50% duty
        apu.write(0x17, 0xf0, 0); // NR22: volume 15, no envelope
        apu.write(0x18, 0x00, 0);
        apu.write(0x19, 0x87, 0); // trigger: 131072 / (2048 - 1792) = 512 Hz
        apu.sync(CLOCK_HZ);
        let left: Vec<f32> = apu.output.iter().step_by(2).copied().collect();
        let peak = left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.1, "peak {peak}");
        // A 512 Hz square crosses zero about 1024 times a second.
        let crossings = left
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        assert!(
            (1000..1050).contains(&crossings),
            "{crossings} zero crossings"
        );
        assert!(!apu.silent);
    }
}
