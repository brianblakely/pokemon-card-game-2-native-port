# Runtime

How the port turns `src/decomp` into a native program, and how that program
runs. The decomp is the only implementation of the game. It is never edited for
the port: everything the port needs lives in `src/runtime` and the build
tooling. A change to `src/decomp` is made only for the original game, and its
build must still match `rom.sha1`.

## Foundations

| Decision | Choice |
| --- | --- |
| Rust workspace | `src/runtime/` (`Cargo.toml`, crates in `src/runtime/crates/*`, committed `Cargo.lock`) |
| Toolchain | `nix develop path:./nix/runtime`: Rust 1.97.1 (rust-overlay), RGBDS 1.0.4, SDL3 3.4.14; `#windows` adds the MinGW cross toolchain and SDL3; `#linux` builds the release with zig against glibc 2.31 |
| Decomp toolchain | `nix develop path:./nix`: RGBDS 1.0.4, GCC, make, Python |
| Versions | sdl3-sys =0.6.8 (SDL 3.4.14, the sibling project's pin). The only graphics library is SDL's own 2D renderer |
| Game data | Read from the player's ROM at run time. SHA-1 `a7e12bcc5f514e3aad8de570fd511aab0a308822` is checked before boot. Builds contain code, plus the port's own translation and font |

## Translation (`tcg2-recomp`)

The kernel crate's build script runs the translator. `tcg2-recomp --out DIR` does
the same by hand.

1. **Annotated build.** The decomp is mirrored into `OUT_DIR/decomp`, keeping
   modification times so make sees what is current. Every source file is
   scanned:
   - Every statement is classified as code (an SM83 mnemonic, or a macro whose
     body emits an instruction first, such as `farcall` or `start_script`),
     data (`db`, `dw`, `ds`, `INCBIN`, or data macros) or neither.
   - Each label is classified by the first statement after it.
   - At every unlabelled change between code and data, a local label is injected
     (`.tcg2_code_N` or `.tcg2_data_N`).
   - Macros are classified from their own bodies, both `MACRO name` and
     `MACRO? name` forms. Block comments are ignored.

   Labels emit no bytes, so the annotated ROM must still match `rom.sha1`, and
   the build checks it. Its symbol file gives the address of every label and
   every injected boundary.
2. **Discovery.** Recursive descent decodes every instruction reachable from
   these seeds: the reset and interrupt vectors, `$0100`, every code label, and
   every injected code boundary. It follows direct jumps and calls, and the far
   calls' inline targets (`rst $28`: bank and address; `rst $18`: an address in
   bank 1). Execution after a call or `rst` continues past their inline
   operands. A call that is directly followed by data does not fall through,
   because its callee consumes that data: `call StartScript` is followed by
   script bytecode. Code resumes after the script's `quit_script` at an injected
   `.tcg2_code_N`, reached only through the script engine's return. Code that
   falls through into data is translated anyway, since the console runs it, and
   reported. All 83 such reports are one original bug: `HandleCheckMenuInput_Bank06`
   calls `PlaySFXConfirmOrCancel` while bank 06 is mapped, so the console runs
   bank 06's attack-animation data at `$5328` as instructions.
3. **Code generation.** Each bank becomes a module, and each global label
   becomes a function: `fn label_BBAAAA(m: &mut Machine, pc: u16) -> u16`.
   The function is a `loop { match pc { … } }` over its blocks, with one arm per
   block start. Each instruction becomes the statements that do what it does,
   with the original instruction, symbolised, as a comment:
   - **Flags:** set by `tcg2_core::cpu`'s helpers, bit for bit, including H and N.
   - **Cycles:** added to the clock. They are batched and flushed before any
     access a device observes: I/O, VRAM, OAM and every computed address.
   - **Static accesses:** reads and writes of work RAM, high RAM and ROM at
     constant addresses index the arrays directly.
   - **Transfers within the function:** `local!` sets `pc` and continues the
     loop, unless the clock has reached the next event; then it returns to the
     dispatcher.
   - **Everything else** returns the next address: calls (after pushing the
     return address), returns (popping it), `jp hl`, `rst`, other functions and
     other banks.
   - **ROM bank switches:** a store to the MBC's bank register in bank 1+ code
     ends its block, so the next instruction comes from the new bank.

   The generated code is about 615,000 lines and compiles in about 35 s.
4. **Tables.** Every block start of every bank is listed with its function. The
   output also includes the RAM routines (`hDMAFunction` is a copy of `DMA`),
   the global labels for diagnostics, and the texts the game keeps outside
   its text table (`TEXT_DATA`: every label outside `src/text` whose first
   statement is a text macro, with its size up to the next label) for
   [languages](#languages-tcg2-locale).

## The dispatcher (`tcg2-kernel`)

`Kernel::run_frame` loops until the frame ends:

1. **Events.** If the clock has reached the next event, it runs device events
   (LCD, timer, serial). If the frame ended, it returns to the host. Otherwise
   it takes a requested, enabled interrupt by pushing the address it was going
   to run and entering the vector.
2. **Halt.** If the CPU is halted, it skips the clock to the next event. This
   is where a frame's idle time goes: zero host time.
3. **Dispatch.** Otherwise it looks up the function for `pc`. Banked addresses
   use the current MBC bank's table. It runs the function and continues at the
   address it returns.

Code in RAM is handled two ways:

- `jp nn` trampolines (the game's VBlank and LCD interrupt hooks in WRAM) jump
  straight to their target.
- The OAM DMA routine in HRAM runs its ROM translation while HRAM still holds
  the same bytes.

An address with no translation, or bytes that are no instruction, stops the
game with a fault that names the nearest label.

Interrupts are taken between blocks, a few cycles later than the console takes
them. `ei` and `reti` make the dispatcher look again at its next boundary.

## The machine (`tcg2-core`)

One clock counts units of 1/8,388,608 s: a CPU cycle at double speed, half of
one at normal speed. `stop` with KEY1 armed switches speed and resets DIV.

**Memory.** Accesses to work RAM bank 0 (the stack and most variables) happen
in place. Every other access is one call:

- ROM bank 0 and the mapped bank;
- VRAM (two banks), cartridge RAM (4 banks, battery-backed), WRAM banks 1-7;
- OAM, I/O and HRAM.

The `rom/` image is shared (`Arc`), so a machine clones cheaply. Tests rely on
this.

**LCD (`ppu.rs`).**
- LY and STAT's mode are computed from the clock since the LCD was switched on:
  154 lines of 912 units, with mode 2 for 80 dots and mode 3 for 172.
- Vertical blank and the STAT sources (mode 0, mode 2, LY=LYC) are scheduled
  events.
- Lines are drawn on demand. Before any write that changes something visible
  (VRAM, OAM, palettes, LCDC, scroll, window), every line the beam has begun is
  drawn with the state it saw. A line is drawn with the state at the start of
  its mode 3. This reproduces the duel's LY-timed wavy scroll (`DistortScreen`)
  exactly.
- A frame is only drawn when something visible changed since the framebuffer
  was made. A change mid-frame redraws the next frame in full too, because the
  lines above it kept the old state. Unchanged lines still advance the window's
  line counter.
- Rendering is CGB mode only (the cartridge is CGB-only): BG and window tile
  attributes, ten sprites per line in OAM order, BG-over-OBJ priority, and LCDC
  bit 0 master priority.
- Tile rows decode eight pixels at once through a 256-entry bit-spreading table.
  The LCD switched off shows white.

**Sound (`apu.rs`).**
- The four channels have duty, envelope, length and sweep, wave RAM with volume
  shift, 7/15-bit noise, the 512 Hz frame sequencer, NR50/NR51 mixing, and the
  output capacitor's high-pass filter.
- Each register write first renders everything before its clock.
- Each channel walks its own timer from one level change to the next. Only the
  changes are recorded, as steps at their exact time. A step inside a sample
  counts for the part of that sample after it (a box filter), and the output is
  the running sum.
- Cost: one step per audible waveform edge, plus an add and the filter per
  48 kHz sample.
- `silent` (every DAC off) lets the host pause its device.

**Timer.** DIV and TIMA are computed from the clock. TIMA's overflow is an
event. The game's timer interrupt (about 241 Hz) runs its link and sound
engines.

**Link ports.**
- Serial: an internally clocked transfer completes after 8 bit times and
  receives `$FF`, as with no cable. An externally clocked one never completes.
- Infrared (`RP`) never sees light.

## The desktop host (`tcg2-desktop`)

The host runs one loop:

1. **Sleep.** `SDL_WaitEventTimeout` sleeps until the next frame is due or an
   event arrives. While paused, or while minimised with `pause_hidden`, it
   blocks with no timeout at all.
2. **Run.** It reads input from SDL's keyboard and gamepad state, then runs each
   frame that is due. At most 5 frames run per wake-up, then the clock resyncs.
3. **Sound.** Each frame's sound is queued to an SDL3 audio stream. A
   resampling ratio within 0.5% holds the queue near 3 frames against drift
   between the system clock and the sound card's. The device period is 1024
   frames.
4. **Present.** If the picture changed, or the window needs it, the picture is
   presented with SDL's 2D renderer (Direct3D 11/12 on Windows, Metal on
   macOS, OpenGL or Vulkan on Linux): one 160×144 streaming texture, scaled
   into the window with vsync. `max_fps` caps how often this happens.

The picture is scaled by SDL with *sharp* (`SDL_SCALEMODE_PIXELART`: square
pixels at fractional scales), *nearest* or *smooth* (bilinear) sampling, and
`tcg2-render` fits it by aspect, integer multiples or stretch. Options, applied
by `tcg2-render` on the CPU to each new picture (23,040 pixels, a few
microseconds) and not at all while it is still:

- GBC LCD colour correction (linear-light mix, after Near's matrix; a table of
  all 32,768 console colours, built on first use);
- frame blending (half of the previous picture).

Pacing is `speed = exact` (59.7275 Hz, the console's rate) by default.
`speed = display` runs the game at a 60 Hz display's own rate when that rate is
within 1.5%, so each frame is shown exactly once.

Files sit beside the executable (beside the `.app` on macOS), or in SDL's
per-user folder when that isn't writable: `rom/`, `saves/poketcg2.sav`,
`screenshots/` and `settings.ini`. `TCG2_HOME` overrides the location.
Battery RAM is written atomically, 0.75 s after the game last changed it, and on
exit.

## Languages (`tcg2-locale`)

The game prints every text through one table, `TextOffsets`. Text number
N's entry, 4 bytes at N × 4 from the start of the table's bank ($37), holds
the text's 24-bit offset from that same point, and `GetTextOffsetFromTextID`
switches to the bank the offset falls in. The ROM's 6,443 texts are
Japanese, a full-width tile per character.

A language is a set of translations keyed by the decomp's text labels:
`locales/en-US/text1.txt` to `text17.txt` mirror `src/text/text*.asm` (the
format is described in `src/text.rs`). When the kernel boots, or the player
switches language, `tcg2_locale::localize` builds the ROM image:

1. **4 MiB.** Banks $00-$7F are the player's ROM. Banks $80-$FF are free, and
   the game's own bank switch reaches them: it writes only the MBC5's low
   bank register. Every image holds every language's text and differs only
   in steps 3 to 7.
2. **Text.** The engine draws Latin letters two ways, and each translation
   names its own (`src/text.rs`):
   - **Full-width**, a letter to a tile, as the Japanese is: letters and most
     signs as `TX_FULLWIDTH4` and a code in the ROM's font of Latin letters
     (`gfx/fonts/full_width/4.png`, the codes the name keyboard types, so a
     typed name matches), digits and a few signs as single codes of the
     digit font, `$70` a blank. No prefix: every print starts full-width.
   - **Half-width**, `TX_HALFWIDTH` and Latin letters two to a tile.

   A full-width text needs twice the room, so a text is full-width only
   where the decomp shows that it fits: the crate's tests
   (`src/tests/layout.rs`) read from the decomp how each text is printed
   (`PrintScrollableText` pages of two 18-tile lines, also from the lists
   `PrintScrollableTextFromList` prints; the wide box at the bottom, two
   lines of 18, one under a yes/no menu; the narrow box, lines of 10; a menu
   box's items, each up to the border or the next item's cursor, and a menu
   only if all its items fit; the duel's coin toss, card lists and the
   reason a check hands back, which the duel prints in the wide box; tables
   of data: a card's text on its page, seven lines of 18, its Pokédex
   entry, three, letters, the glossary, the practice duel's steps and the
   special rules; and the places the scan can't follow, measured by hand
   from the code that prints each text: `src/tests/measured.txt`, with a
   group for texts that share a size) and check that every text that fits
   is full-width, flowed into lines of its box (a card's type in brackets
   keeps a line of its own), and that the rest are not. A text printed
   nowhere the tests know (a name the code puts into another text) stays
   half-width; one can be made full-width by hand within the Japanese
   text's own lines. So does a text that puts another into itself
   (`<RAMTEXT>`): what it puts in is half-width, and a line reads in one
   size, unless the texts it puts in are known and all full-width
   (`PUT_INTO`). Debug menus and routines, which no player sees, do not count.
   `TCG2_BLESS=1` rewrites the files to that layout.

   `<RAMNAME>`, `<RAMTEXT>`, `<RAMNUM>` and symbols such as `<Lv>` become
   their control codes. Texts follow one another from bank $80, never across
   a bank's end, and identical texts share their bytes.
3. **The table.** The chosen language's entries point at its texts. A text it
   leaves out (a symbol, a number) prints as the ROM has it.
4. **Fonts.** Full-width English uses the ROM's full-width font.
   Half-width English is drawn with the port's own 4×8 font (`font.txt`) in
   place of `HalfWidthFont`. The game pads names and list rows with $70, its
   full-width blank, also after half-width text, where $70 is 'p'
   (`CopyCardNameAndLevel`). So the font draws $70 blank, and the encoder
   writes 'p' as $3C, the '<' that no text needs.
5. **Text outside the table.** A few texts sit in code banks: a deck name's
   suffix, default names, a title. `data.txt` replaces them in place
   (`TEXT_DATA` gives where), each within its original bytes.
6. **Full-width characters.** `fullwidth.txt` redraws a few of the
   full-width font's characters. The game's code itself writes "枚" (a
   counter word English lacks) after card counts: it is blank. ("の", which
   the code puts between the player's name and an unnamed deck's number,
   stays: the name entry keyboard types it too.) The font has no "é", drawn
   over its unused second copy of "e", and its period and comma float a row
   above the letters' baseline, where a period reads as a dash: they are
   redrawn on it.
7. **Pictures.** `graphics.txt` redraws pictures with lettering (the duel's
   banners, the play area's location labels, the result stamps, the slot
   machine's signs, the booster packs' names, the Card Pop! screens) in the
   original's four colors, over the original's tiles. A picture the game
   assembles from scattered tiles (sprites, a sign's tilemap) is drawn as it
   appears and mapped back to its tiles. Only the lettering is drawn: pixels
   it does not change stay the ROM's.

No code is changed: the kernel was translated from the same program and runs
on the new image as it is. `Kernel::set_language` swaps the cartridge's
image. The mapped bank stays mapped, and a text being printed keeps reading
the bytes it started on. Text already on screen, and names the game has
stored in battery RAM, stay as they are.

## Save states

`Machine::save_state` and `load_state` (`tcg2-core/src/state.rs`) serialise every
field of the machine: registers, clock, memory, and each device's internal
state. The format is a magic and version followed by fixed little-endian
fields, about 175 KB. A state is complete because the dispatcher keeps none
of the game's state outside the machine: it resumes at `cpu.pc` and keeps no
native stack. Loading resets the sound output's sample origin to the state's
time and refreshes the picture. Battery RAM in a state counts as changed, so
the host writes it to the save file.

## Known differences from the console

- Interrupts are taken at block boundaries, a few cycles late. A palette
  fade in the intro lands a few dozen cycles differently against the beam from
  SameBoy, so one frame differs.
- Frame boundaries restart when the LCD is switched on, so frame numbers drift
  against other emulators around LCD-off loads. The content stays the same.
- HDMA is done at once (the game never uses it). OAM DMA is instant (the
  game's wait loop still takes its time).
- Code that writes the MBC's bank register through a computed address, in bank
  1+ code, keeps running its block from the old bank until the block ends. The
  game never does this.
