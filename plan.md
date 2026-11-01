# Plan

The native port follows the phases of
[Xenogears: Ex Machina](https://github.com/brianblakely/xenogears-ex-machina).
The game's decompilation is the only implementation of its logic. A runtime
built around it carries the game to new platforms without changing it.

| Phase | Status | Scope |
| --- | --- | --- |
| 1. Decompilation | **Complete** (pret) | `src/decomp/` rebuilds the ROM byte for byte (`rom.sha1`), with every label of code and data named |
| 2. Translation and kernel | **Complete** | `tcg2-recomp` translates the decomp to Rust at build time: 131,000 instructions, 6,400 functions, every code/data boundary taken from an annotated build. The dispatcher runs the game with no interpreter |
| 3. Machine services | **Complete** | Memory map, MBC5 over the player's ROM, lazily timed LCD with on-demand line drawing, sound synthesis, timer, joypad, link ports, save states |
| 4. Desktop app | **Complete** | SDL3 (window, 2D renderer, sound, input) on Linux, Windows and macOS: any window size (sharp, nearest or smooth scaling; aspect, integer or stretch fit; GBC LCD colours; frame blending), any display rate (presentation decoupled from the game, a frame-rate cap, display-locked speed), keyboard and gamepads, battery saves, save states, screenshots, pause, fast-forward |
| 5. Battery use | **Complete** | 2.9-3.6 ms of host CPU per game second, 3× less than gambatte and 16-32× less than SameBoy. The host sleeps between frames, presents only changed pictures and idles the sound device in silence |
| 6. Verification | **Complete** | Instruction-for-instruction lockstep with an independent interpreter, over random play, a full duel and every translated entry point. Pictures and music checked against SameBoy. Windows and Linux builds give identical results |
| 7. Releases | Ready | `.github/workflows/release.yml` (as in Xenogears: Ex Machina) reserves a version when native inputs change, builds Linux archives (x86-64, AArch64; SDL3 compiled in, glibc 2.31+), Windows and macOS (Apple Silicon, Intel) packages in the pinned Nix shells, tests them, and publishes notes, checksums, notices and runtime sources. `packaging/arch/PKGBUILD` builds for Arch |
| 8. Languages | **Complete** | American English, the default: every text the game prints, translated for the port and drawn with the port's own half-width font, and the lettered pictures of duels and the slot machine redrawn, all laid into the ROM image at load (`tcg2-locale`). F9 or `language` switches to the original Japanese and back while playing |

## Possible next steps

- **Motion interpolation** for high-refresh displays. The LCD would have to keep
  its layers apart (background with a margin, window, sprites) so a host could
  draw scroll and sprite positions between frames. Frame blending is the
  current smoothing option.
- **A control session** for agents and tools: typed commands over stdin, like
  `xem-headless --control`.
- **Link play** between two running copies: card trades and link duels over
  the serial port, plus the Card Pop infrared exchange.
- **More platforms on the same kernel:** Android, and a browser build
  (wasm32: the kernel is plain Rust).
- **More languages:** each is a folder of translation files, a font and
  redrawn pictures (`src/runtime/crates/tcg2-locale/locales/`).
- **The remaining pictures with Japanese lettering** in English: the title
  logo and the opening's booster packs, booster pack art, the special-rules
  banner, the Card Pop! and Rare Card Pop! screens and the deck diagnosis
  title.
- **Lower power still:** batch two or more frames per wake-up while the
  picture is static and no input arrives, at the cost of a little sound
  latency.
