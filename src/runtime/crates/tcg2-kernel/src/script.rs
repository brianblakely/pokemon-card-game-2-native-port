//! Input for automated runs (tests, agents, measurement): button presses at
//! given frames, and seeded random play.

use tcg2_core::buttons;

pub fn parse_buttons(text: &str) -> Result<u8, String> {
    let mut b = 0;
    for name in text.split('+') {
        b |= match name {
            "a" => buttons::A,
            "b" => buttons::B,
            "start" => buttons::START,
            "select" => buttons::SELECT,
            "up" => buttons::UP,
            "down" => buttons::DOWN,
            "left" => buttons::LEFT,
            "right" => buttons::RIGHT,
            "none" => 0,
            _ => return Err(format!("unknown button {name:?}")),
        };
    }
    Ok(b)
}

/// `FRAME:BUTTONS[:FRAMES]`: `+`-joined buttons held from a frame for a
/// number of frames (default 6).
#[derive(Clone, Debug)]
pub struct Press {
    pub frame: u64,
    pub buttons: u8,
    pub frames: u64,
}

impl std::str::FromStr for Press {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() < 2 {
            return Err(format!("{text:?}: FRAME:BUTTONS[:FRAMES]"));
        }
        Ok(Press {
            frame: parts[0]
                .parse()
                .map_err(|_| format!("bad frame in {text:?}"))?,
            buttons: parse_buttons(parts[1])?,
            frames: match parts.get(2) {
                Some(f) => f.parse().map_err(|_| format!("bad length in {text:?}"))?,
                None => 6,
            },
        })
    }
}

/// Seeded random play: mostly A (through dialogue and menus) and walking,
/// sometimes B, rarely Start or Select; each held for a few frames.
#[derive(Clone, Debug)]
pub struct RandomPlay {
    state: u64,
    held: u8,
    until: u64,
}

impl std::str::FromStr for RandomPlay {
    type Err = String;
    /// `SEED` or `SEED@FRAME` (random play from that frame on).
    fn from_str(text: &str) -> Result<Self, String> {
        let (seed, from) = text.split_once('@').unwrap_or((text, "0"));
        let mut play = RandomPlay::new(seed.parse().map_err(|_| format!("bad seed {seed:?}"))?);
        play.until = from
            .parse()
            .map_err(|_| format!("bad start frame {from:?}"))?;
        Ok(play)
    }
}

impl RandomPlay {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            held: 0,
            until: 0,
        }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    pub fn buttons(&mut self, frame: u64) -> u8 {
        if frame >= self.until {
            let r = self.next();
            let (held, length) = match r % 100 {
                0..=44 => (buttons::A, 2 + (r >> 32) % 6),
                45..=51 => (buttons::B, 2 + (r >> 32) % 6),
                52..=59 => (buttons::UP, 4 + (r >> 32) % 40),
                60..=67 => (buttons::DOWN, 4 + (r >> 32) % 40),
                68..=75 => (buttons::LEFT, 4 + (r >> 32) % 40),
                76..=83 => (buttons::RIGHT, 4 + (r >> 32) % 40),
                84..=85 => (buttons::START, 2 + (r >> 32) % 4),
                86 => (buttons::SELECT, 2 + (r >> 32) % 4),
                _ => (0, 2 + (r >> 32) % 12),
            };
            self.held = held;
            self.until = frame + length;
        }
        self.held
    }
}

/// Presses and random play combined.
#[derive(Clone, Debug, Default)]
pub struct Script {
    pub presses: Vec<Press>,
    pub random: Option<RandomPlay>,
}

impl Script {
    pub fn buttons(&mut self, frame: u64) -> u8 {
        let mut held = self
            .presses
            .iter()
            .filter(|p| frame >= p.frame && frame < p.frame + p.frames)
            .fold(0, |b, p| b | p.buttons);
        if let Some(random) = &mut self.random {
            held |= random.buttons(frame);
        }
        held
    }
}
