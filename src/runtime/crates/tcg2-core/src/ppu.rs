//! The LCD controller as a lazy service.
//!
//! Nothing steps the PPU dot by dot. Its position (LY, STAT's mode) is a
//! function of the clock since the LCD was switched on, computed when the game
//! reads a register, and its interrupts are scheduled as clock events. Lines
//! are drawn on demand: just before a write changes anything visible, every
//! line the beam has passed is drawn with the state it saw, so mid-frame
//! effects (the duel's LY-timed scroll wave) come out as on the console.
//!
//! A line is only drawn when something visible changed since the picture in
//! the framebuffer was made; a screen the game leaves alone costs no drawing
//! at all, and the host learns from `frame_changed` that it need not present.

pub const SCREEN_WIDTH: usize = 160;
pub const SCREEN_HEIGHT: usize = 144;
/// Clock units (1/8388608 s) per line: 456 dots of 2 units.
pub const LINE_UNITS: u64 = 912;
pub const LINES_PER_FRAME: u64 = 154;
pub const FRAME_UNITS: u64 = LINE_UNITS * LINES_PER_FRAME;
const MODE2_END: u64 = 160; // 80 dots of OAM search
const MODE3_END: u64 = MODE2_END + 344; // 172 dots of drawing
const VBLANK_START: u64 = 144 * LINE_UNITS;

pub const STAT_MODE0_IRQ: u8 = 0x08;
pub const STAT_MODE1_IRQ: u8 = 0x10;
pub const STAT_MODE2_IRQ: u8 = 0x20;
pub const STAT_LYC_IRQ: u8 = 0x40;

#[derive(Clone)]
pub struct Ppu {
    pub vram: Box<[u8; 0x4000]>,
    pub vbk: u8,
    pub oam: [u8; 0xa0],
    pub lcdc: u8,
    /// STAT's interrupt selects (bits 3-6); the rest is computed.
    pub stat: u8,
    pub scy: u8,
    pub scx: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    pub bcps: u8,
    pub ocps: u8,
    pub bg_palettes: [u8; 64],
    pub obj_palettes: [u8; 64],
    /// RGBA8888 of each palette entry (8 palettes x 4 colours) as stored in
    /// the framebuffer, kept with the palette RAM.
    bg_rgba: [u32; 32],
    obj_rgba: [u32; 32],

    /// The clock at which line 0 of the current LCD-on period began.
    lcd_on_at: u64,
    /// Frames begun since the LCD was switched on (the one being drawn).
    frame: u64,
    /// Lines of `frame` already accounted for (drawn, or unchanged).
    next_line: u8,
    window_line: u8,
    /// The visible state differs from what made the framebuffer's lines at
    /// and after `next_line`.
    dirty: bool,
    /// A change landed mid-frame: the lines above it keep the old picture, so
    /// the next frame is drawn in full again.
    redraw_next_frame: bool,
    /// Some line of the current frame was drawn (the picture changed).
    changed_this_frame: bool,
    /// The last frame completed differed from the one before.
    pub frame_changed: bool,

    pub next_vblank: u64,
    pub next_stat: u64,

    /// RGBA8888 pixels (one word each, bytes in R, G, B, A order),
    /// row-major, 160x144.
    pixels: Box<[u32]>,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

#[inline(always)]
fn rgba(lo: u8, hi: u8) -> u32 {
    let c = u16::from_le_bytes([lo, hi]);
    let x = |v: u16| {
        let v = (v & 31) as u8;
        (v << 3) | (v >> 2)
    };
    u32::from_ne_bytes([x(c), x(c >> 5), x(c >> 10), 0xff])
}

/// `SPREAD[b]` holds bit 7 of `b` in byte 0, bit 6 in byte 1 ... bit 0 in
/// byte 7: a tile row's eight pixels in screen order are
/// `SPREAD[lo] | SPREAD[hi] << 1`. `SPREAD_FLIPPED` for mirrored tiles.
const SPREAD: [u64; 256] = spread(false);
const SPREAD_FLIPPED: [u64; 256] = spread(true);

const fn spread(flipped: bool) -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut b = 0;
    while b < 256 {
        let mut i = 0;
        while i < 8 {
            let bit = if flipped { i } else { 7 - i };
            if b >> bit & 1 != 0 {
                table[b] |= 1 << (8 * i);
            }
            i += 1;
        }
        b += 1;
    }
    table
}

impl Ppu {
    pub fn new() -> Self {
        let mut ppu = Self {
            vram: Box::new([0; 0x4000]),
            vbk: 0,
            oam: [0; 0xa0],
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            lyc: 0,
            bgp: 0xfc,
            obp0: 0xff,
            obp1: 0xff,
            wy: 0,
            wx: 0,
            bcps: 0,
            ocps: 0,
            bg_palettes: [0xff; 64],
            obj_palettes: [0xff; 64],
            bg_rgba: [u32::MAX; 32],
            obj_rgba: [u32::MAX; 32],
            lcd_on_at: 0,
            frame: 0,
            next_line: 0,
            window_line: 0,
            dirty: true,
            redraw_next_frame: false,
            changed_this_frame: false,
            frame_changed: true,
            next_vblank: VBLANK_START,
            next_stat: u64::MAX,
            pixels: vec![u32::MAX; SCREEN_WIDTH * SCREEN_HEIGHT].into_boxed_slice(),
        };
        ppu.refresh_rgba();
        ppu
    }

    fn refresh_rgba(&mut self) {
        for i in 0..32 {
            self.bg_rgba[i] = rgba(self.bg_palettes[i * 2], self.bg_palettes[i * 2 + 1]);
            self.obj_rgba[i] = rgba(self.obj_palettes[i * 2], self.obj_palettes[i * 2 + 1]);
        }
    }

    #[inline(always)]
    pub fn lcd_on(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    /// (frame since LCD on, line, clock units into the line).
    #[inline]
    fn position(&self, clock: u64) -> (u64, u64, u64) {
        let elapsed = clock.saturating_sub(self.lcd_on_at);
        let frame = elapsed / FRAME_UNITS;
        let within = elapsed % FRAME_UNITS;
        (frame, within / LINE_UNITS, within % LINE_UNITS)
    }

    /// The frame whose lines the beam is on at `clock` and how many of its
    /// lines it has begun drawing. Vertical blank belongs to the next frame,
    /// with none of its lines reached.
    #[inline]
    fn beam(&self, clock: u64) -> (u64, u8) {
        let (frame, line, dot) = self.position(clock);
        if line >= 144 {
            (frame + 1, 0)
        } else if dot >= MODE2_END {
            (frame, line as u8 + 1)
        } else {
            (frame, line as u8)
        }
    }

    pub fn ly(&self, clock: u64) -> u8 {
        if !self.lcd_on() {
            return 0;
        }
        self.position(clock).1 as u8
    }

    pub fn mode(&self, clock: u64) -> u8 {
        if !self.lcd_on() {
            return 0;
        }
        let (_, line, dot) = self.position(clock);
        if line >= 144 {
            1
        } else if dot < MODE2_END {
            2
        } else if dot < MODE3_END {
            3
        } else {
            0
        }
    }

    pub fn read_stat(&self, clock: u64) -> u8 {
        let coincidence = if self.lcd_on() && self.ly(clock) == self.lyc {
            0x04
        } else {
            0
        };
        0x80 | (self.stat & 0x78) | coincidence | self.mode(clock)
    }

    /// Accounts for every line the beam has reached by `clock`, drawing those
    /// whose picture changed.
    pub fn sync(&mut self, clock: u64) {
        if !self.lcd_on() {
            return;
        }
        let (frame, reached) = self.beam(clock);
        while self.frame < frame {
            self.finish_frame();
            self.frame += 1;
        }
        self.draw_lines(reached);
    }

    fn window_visible(&self, ly: u8) -> bool {
        self.lcdc & 0x20 != 0 && self.wy <= ly && self.wx <= 166
    }

    fn draw_lines(&mut self, until: u8) {
        while self.next_line < until {
            let line = self.next_line;
            if self.dirty {
                self.draw_line(line);
                self.changed_this_frame = true;
            } else if self.window_visible(line) {
                // Unchanged, but the window's own line counter moves on.
                self.window_line = self.window_line.wrapping_add(1);
            }
            self.next_line += 1;
        }
    }

    fn finish_frame(&mut self) {
        self.draw_lines(144);
        self.frame_changed |= self.changed_this_frame;
        self.changed_this_frame = false;
        self.dirty = self.redraw_next_frame;
        self.redraw_next_frame = false;
        self.next_line = 0;
        self.window_line = 0;
    }

    /// Called before a visible change: lines drawn so far keep the old state.
    #[inline]
    fn before_change(&mut self, clock: u64) {
        self.sync(clock);
        if self.next_line > 0 && self.next_line < 144 {
            self.redraw_next_frame = true;
        }
        self.dirty = true;
    }

    #[inline]
    pub fn write_vram(&mut self, addr: u16, value: u8, clock: u64) {
        let i = ((self.vbk as usize & 1) << 13) | (addr as usize & 0x1fff);
        if self.vram[i] != value {
            self.before_change(clock);
            self.vram[i] = value;
        }
    }

    #[inline]
    pub fn read_vram(&self, addr: u16) -> u8 {
        self.vram[((self.vbk as usize & 1) << 13) | (addr as usize & 0x1fff)]
    }

    #[inline]
    pub fn write_oam(&mut self, index: usize, value: u8, clock: u64) {
        if self.oam[index] != value {
            self.before_change(clock);
            self.oam[index] = value;
        }
    }

    pub fn write_oam_block(&mut self, data: &[u8; 0xa0], clock: u64) {
        if self.oam != *data {
            self.before_change(clock);
            self.oam = *data;
        }
    }

    /// Register writes ($FF40-$FF4B, $FF68-$FF6B; not DMA).
    pub fn write_register(&mut self, reg: u8, value: u8, clock: u64) {
        macro_rules! visible {
            ($field:ident) => {
                if self.$field != value {
                    self.before_change(clock);
                    self.$field = value;
                }
            };
        }
        match reg {
            0x40 => {
                if self.lcdc == value {
                    return;
                }
                let was_on = self.lcd_on();
                let on = value & 0x80 != 0;
                if was_on && !on {
                    self.before_change(clock);
                    self.lcdc = value;
                    // The console shows a blank (white) screen.
                    self.pixels.fill(u32::MAX);
                    self.frame_changed = true;
                    self.dirty = true;
                    self.redraw_next_frame = false;
                    self.next_line = 0;
                    self.next_vblank = u64::MAX;
                } else if !was_on && on {
                    self.lcdc = value;
                    self.lcd_on_at = clock;
                    self.frame = 0;
                    self.next_line = 0;
                    self.window_line = 0;
                    self.dirty = true;
                    self.redraw_next_frame = false;
                    self.next_vblank = clock + VBLANK_START;
                } else {
                    self.before_change(clock);
                    self.lcdc = value;
                }
            }
            0x41 => self.stat = value & 0x78,
            0x42 => visible!(scy),
            0x43 => visible!(scx),
            0x45 => self.lyc = value,
            0x47 => visible!(bgp),
            0x48 => visible!(obp0),
            0x49 => visible!(obp1),
            0x4a => visible!(wy),
            0x4b => visible!(wx),
            0x68 => self.bcps = value & 0xbf,
            0x69 => {
                let i = (self.bcps & 0x3f) as usize;
                if self.bg_palettes[i] != value {
                    self.before_change(clock);
                    self.bg_palettes[i] = value;
                    let entry = i / 2;
                    self.bg_rgba[entry] =
                        rgba(self.bg_palettes[entry * 2], self.bg_palettes[entry * 2 + 1]);
                }
                if self.bcps & 0x80 != 0 {
                    self.bcps = 0x80 | ((self.bcps + 1) & 0x3f);
                }
            }
            0x6a => self.ocps = value & 0xbf,
            0x6b => {
                let i = (self.ocps & 0x3f) as usize;
                if self.obj_palettes[i] != value {
                    self.before_change(clock);
                    self.obj_palettes[i] = value;
                    let entry = i / 2;
                    self.obj_rgba[entry] = rgba(
                        self.obj_palettes[entry * 2],
                        self.obj_palettes[entry * 2 + 1],
                    );
                }
                if self.ocps & 0x80 != 0 {
                    self.ocps = 0x80 | ((self.ocps + 1) & 0x3f);
                }
            }
            _ => {}
        }
        self.schedule_stat(clock);
    }

    pub fn read_register(&self, reg: u8, clock: u64) -> u8 {
        match reg {
            0x40 => self.lcdc,
            0x41 => self.read_stat(clock),
            0x42 => self.scy,
            0x43 => self.scx,
            0x44 => self.ly(clock),
            0x45 => self.lyc,
            0x47 => self.bgp,
            0x48 => self.obp0,
            0x49 => self.obp1,
            0x4a => self.wy,
            0x4b => self.wx,
            0x68 => self.bcps | 0x40,
            0x69 => self.bg_palettes[(self.bcps & 0x3f) as usize],
            0x6a => self.ocps | 0x40,
            0x6b => self.obj_palettes[(self.ocps & 0x3f) as usize],
            _ => 0xff,
        }
    }

    pub fn next_event(&self) -> u64 {
        self.next_vblank.min(self.next_stat)
    }

    /// Schedules the next STAT interrupt (mode 0, mode 2 or LY=LYC) strictly
    /// after `after`; mode 1's rides on the vertical blank event.
    fn schedule_stat(&mut self, after: u64) {
        self.next_stat = u64::MAX;
        let mode0 = self.stat & STAT_MODE0_IRQ != 0;
        let mode2 = self.stat & STAT_MODE2_IRQ != 0;
        let lyc = self.stat & STAT_LYC_IRQ != 0 && (self.lyc as u64) < LINES_PER_FRAME;
        if !self.lcd_on() || !(mode0 || mode2 || lyc) {
            return;
        }
        let elapsed = after.saturating_sub(self.lcd_on_at);
        let frame_start = self.lcd_on_at + elapsed / FRAME_UNITS * FRAME_UNITS;
        let mut best = u64::MAX;
        for base in [frame_start, frame_start + FRAME_UNITS] {
            if lyc {
                let t = base + self.lyc as u64 * LINE_UNITS;
                if t > after {
                    best = best.min(t);
                }
            }
            if mode0 || mode2 {
                let first = if after >= base {
                    (after - base) / LINE_UNITS
                } else {
                    0
                };
                for line in first..144 {
                    let start = base + line * LINE_UNITS;
                    if mode2 && start > after {
                        best = best.min(start);
                    }
                    if mode0 && start + MODE3_END > after {
                        best = best.min(start + MODE3_END);
                    }
                    if best != u64::MAX {
                        break;
                    }
                }
            }
            if best != u64::MAX {
                break;
            }
        }
        self.next_stat = best;
    }

    /// Processes the LCD events due by `clock`; returns the interrupt flags
    /// raised and whether a frame ended (the start of vertical blank).
    pub fn on_events(&mut self, clock: u64) -> (u8, bool) {
        let mut flags = 0;
        let mut frame_end = false;
        if clock >= self.next_stat {
            flags |= 0x02;
            let at = self.next_stat;
            self.schedule_stat(at);
        }
        if clock >= self.next_vblank {
            let at = self.next_vblank;
            // At the blank the beam has left the frame: finish it.
            self.sync(at);
            self.next_vblank = at + FRAME_UNITS;
            flags |= 0x01;
            if self.stat & STAT_MODE1_IRQ != 0 {
                flags |= 0x02;
            }
            frame_end = true;
        }
        (flags, frame_end)
    }

    pub(crate) fn save(&self, w: &mut crate::state::Writer) {
        w.bytes(&self.vram[..]);
        w.u8(self.vbk);
        w.bytes(&self.oam);
        for v in [
            self.lcdc, self.stat, self.scy, self.scx, self.lyc, self.bgp, self.obp0, self.obp1,
            self.wy, self.wx, self.bcps, self.ocps,
        ] {
            w.u8(v);
        }
        w.bytes(&self.bg_palettes);
        w.bytes(&self.obj_palettes);
        w.u64(self.lcd_on_at);
        w.u64(self.frame);
        w.u8(self.next_line);
        w.u8(self.window_line);
        w.bool(self.dirty);
        w.bool(self.redraw_next_frame);
        w.bool(self.changed_this_frame);
        w.u64(self.next_vblank);
        w.u64(self.next_stat);
        w.bytes(self.framebuffer());
    }

    pub(crate) fn load(
        &mut self,
        r: &mut crate::state::Reader,
    ) -> Result<(), crate::state::StateError> {
        r.fill(&mut self.vram[..])?;
        self.vbk = r.u8()?;
        r.fill(&mut self.oam)?;
        for v in [
            &mut self.lcdc,
            &mut self.stat,
            &mut self.scy,
            &mut self.scx,
            &mut self.lyc,
            &mut self.bgp,
            &mut self.obp0,
            &mut self.obp1,
            &mut self.wy,
            &mut self.wx,
            &mut self.bcps,
            &mut self.ocps,
        ] {
            *v = r.u8()?;
        }
        r.fill(&mut self.bg_palettes)?;
        r.fill(&mut self.obj_palettes)?;
        self.refresh_rgba();
        self.lcd_on_at = r.u64()?;
        self.frame = r.u64()?;
        self.next_line = r.u8()?;
        self.window_line = r.u8()?;
        self.dirty = r.bool()?;
        self.redraw_next_frame = r.bool()?;
        self.changed_this_frame = r.bool()?;
        self.next_vblank = r.u64()?;
        self.next_stat = r.u64()?;
        let pixels = r.vec()?;
        if pixels.len() != self.pixels.len() * 4 {
            return Err(crate::state::StateError("picture size".into()));
        }
        for (word, bytes) in self.pixels.iter_mut().zip(pixels.chunks_exact(4)) {
            *word = u32::from_ne_bytes(bytes.try_into().unwrap());
        }
        self.frame_changed = true;
        Ok(())
    }

    /// The picture: 160x144 RGBA8.
    pub fn framebuffer(&self) -> &[u8] {
        // SAFETY: any u32 is four valid bytes; the slice covers exactly the
        // pixels' memory and borrows them.
        unsafe {
            std::slice::from_raw_parts(self.pixels.as_ptr().cast::<u8>(), self.pixels.len() * 4)
        }
    }

    /// The framebuffer changed since the last call.
    pub fn take_frame_changed(&mut self) -> bool {
        std::mem::take(&mut self.frame_changed)
    }

    // The renderer (CGB mode).

    #[inline(always)]
    fn tile_row(&self, bank: usize, tile_addr: usize, row: usize) -> (u8, u8) {
        let i = (bank << 13) | (tile_addr + row * 2);
        (self.vram[i], self.vram[i + 1])
    }

    fn tile_data_addr(&self, tile: u8) -> usize {
        if self.lcdc & 0x10 != 0 {
            tile as usize * 16
        } else {
            (0x1000i32 + (tile as i8 as i32) * 16) as usize
        }
    }

    fn draw_line(&mut self, ly: u8) {
        // Per pixel: the background's colour index (bits 0-1) and its
        // over-sprites priority (bit 2), for the sprites to test.
        let mut bg = [0u8; SCREEN_WIDTH];
        let row = ly as usize * SCREEN_WIDTH;
        let window_x = if self.window_visible(ly) {
            self.wx as i32 - 7
        } else {
            SCREEN_WIDTH as i32
        };

        // Background, up to where the window starts.
        let bg_map = if self.lcdc & 0x08 != 0 {
            0x1c00
        } else {
            0x1800
        };
        let y = self.scy.wrapping_add(ly) as usize;
        let bg_end = window_x.clamp(0, SCREEN_WIDTH as i32) as usize;
        self.render_span(row, bg_map, y, self.scx as usize, 0, bg_end, &mut bg);
        // Window.
        if window_x < SCREEN_WIDTH as i32 {
            let win_map = if self.lcdc & 0x40 != 0 {
                0x1c00
            } else {
                0x1800
            };
            let start = window_x.max(0) as usize;
            // The window's x of its first visible pixel (WX < 7 cuts its left).
            let first = (start as i32 - window_x) as usize;
            let wy = self.window_line as usize;
            self.render_span(
                row,
                win_map,
                wy,
                first.wrapping_sub(start) & 0xff,
                start,
                SCREEN_WIDTH,
                &mut bg,
            );
            self.window_line = self.window_line.wrapping_add(1);
        }

        if self.lcdc & 0x02 != 0 {
            self.draw_sprites(ly, &bg);
        }
    }

    /// Draws screen pixels [`from`, `to`) of `row` from a 32x32 tile map
    /// scrolled by `scroll` (wrapping at 256), on map line `y`: one tile row
    /// fetched and decoded per eight pixels.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn render_span(
        &mut self,
        row: usize,
        map_base: usize,
        y: usize,
        scroll: usize,
        from: usize,
        to: usize,
        bg: &mut [u8; SCREEN_WIDTH],
    ) {
        let mut x = from;
        while x < to {
            let px = (x + scroll) & 0xff;
            let map = map_base + (y / 8) * 32 + px / 8;
            let tile = self.vram[map];
            let attr = self.vram[0x2000 + map];
            let bank = ((attr >> 3) & 1) as usize;
            let line = if attr & 0x40 != 0 { 7 - (y & 7) } else { y & 7 };
            let (lo, hi) = self.tile_row(bank, self.tile_data_addr(tile), line);
            let spread = if attr & 0x20 != 0 {
                &SPREAD_FLIPPED
            } else {
                &SPREAD
            };
            let colours = spread[lo as usize] | spread[hi as usize] << 1;
            let palette = (attr & 7) as usize * 4;
            let priority = (attr >> 5) & 4;
            let first = px & 7;
            let count = (8 - first).min(to - x);
            for i in 0..count {
                let c = (colours >> (8 * (first + i))) as u8 & 3;
                self.pixels[row + x + i] = self.bg_rgba[palette + c as usize];
                bg[x + i] = c | priority;
            }
            x += count;
        }
    }

    fn draw_sprites(&mut self, ly: u8, bg: &[u8; SCREEN_WIDTH]) {
        let height = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let mut selected = [0usize; 10];
        let mut count = 0;
        for i in 0..40 {
            let y = self.oam[i * 4] as i32 - 16;
            if (ly as i32) >= y && (ly as i32) < y + height {
                selected[count] = i;
                count += 1;
                if count == 10 {
                    break;
                }
            }
        }
        let master_priority = self.lcdc & 0x01 != 0;
        let row = ly as usize * SCREEN_WIDTH;
        // CGB: at each pixel the first sprite in OAM with a visible pixel is
        // the one shown, and only then does the background's priority decide
        // between that pixel and the background.
        let mut taken = [false; SCREEN_WIDTH];
        for &i in &selected[..count] {
            let y = self.oam[i * 4] as i32 - 16;
            let x0 = self.oam[i * 4 + 1] as i32 - 8;
            if x0 <= -8 || x0 >= SCREEN_WIDTH as i32 {
                continue;
            }
            let mut tile = self.oam[i * 4 + 2];
            let attr = self.oam[i * 4 + 3];
            if height == 16 {
                tile &= 0xfe;
            }
            let mut line = ly as i32 - y;
            if attr & 0x40 != 0 {
                line = height - 1 - line;
            }
            let bank = ((attr >> 3) & 1) as usize;
            let (lo, hi) = self.tile_row(bank, tile as usize * 16, line as usize);
            let spread = if attr & 0x20 != 0 {
                &SPREAD_FLIPPED
            } else {
                &SPREAD
            };
            let colours = spread[lo as usize] | spread[hi as usize] << 1;
            let behind = attr & 0x80 != 0;
            let palette = (attr & 7) as usize * 4;
            for px in 0..8 {
                let x = x0 + px;
                if !(0..SCREEN_WIDTH as i32).contains(&x) {
                    continue;
                }
                let xi = x as usize;
                let c = (colours >> (8 * px)) as u8 & 3;
                if c == 0 || taken[xi] {
                    continue;
                }
                taken[xi] = true;
                let under = bg[xi];
                if master_priority && under & 3 != 0 && (under & 4 != 0 || behind) {
                    continue;
                }
                self.pixels[row + xi] = self.obj_rgba[palette + c as usize];
            }
        }
    }
}
