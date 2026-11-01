# Development

All commands run from the repository root inside the pinned shell
(`nix develop path:./nix/runtime`) unless noted. Put your ROM at
`rom/poketcg2.gbc` (git-ignored) for anything that runs the game.

## Build

```sh
make -C src/decomp                                 # the ROM, checked against rom.sha1
cd src/runtime
cargo build --release -p tcg2-desktop              # the app: target/release/pokemon-tcg2
cargo build --release -p tcg2-headless -p tcg2-reference
cargo run --release -p tcg2-recomp -- --decomp ../decomp --work ../../build/decomp --out ../../build/kernel
```

The last command writes the generated kernel to `build/kernel/` for reading,
along with `report.txt`: per-bank counts and discovery warnings. To build with
sources made elsewhere, set `TCG2_KERNEL_DIR` to that directory. CI does this
for Windows and macOS, which need no RGBDS.

The development shell's SDL is Nix-built and cannot load a non-NixOS host's GPU
drivers. To run with them, build as the Linux release does (SDL compiled in,
only glibc linked; the binary lands in
`target/x86_64-unknown-linux-gnu/release/`):

```sh
nix develop path:./nix/runtime#linux -c bash -c 'cd src/runtime && cargo zigbuild --release -p tcg2-desktop --features static-sdl --target x86_64-unknown-linux-gnu.2.31'
```

## Tests

```sh
cd src/runtime
cargo test --release --workspace                   # unit tests: CPU flags, timer, sound, decoder, shader
./target/release/tcg2-lockstep --rom ../../rom/poketcg2.gbc --frames 100000 --random 7
./target/release/tcg2-lockstep --rom ../../rom/poketcg2.gbc --frames 400 --blocks 10
```

- `tcg2-lockstep` runs the translated kernel and the reference interpreter
  (`tcg2-reference`, written independently of the translator) on identical
  machines. It compares the CPU after every dispatch and all memory at every
  frame's end.
  - `--press FRAME:BUTTONS[:FRAMES]` and `--random SEED` script the input.
  - `--blocks N` instead runs every translated block start N times from
    randomised registers, after booting `--frames` frames, and compares each
    result.
  - `TCG2_TRACE=BB:AAAA:TRIAL` traces one trial instruction by instruction.
- Every tool runs the game in American English unless given `--language ja`
  (the ROM's own Japanese). Compare pictures with a reference emulator, which
  runs the unchanged ROM, in Japanese.
- `tcg2-headless --rom … --frames N [--press …] [--random SEED]` runs the game
  without a window.
  - `--screenshot PATH` and `--shots N:DIR` write PNGs; `--wav PATH` writes the
    sound.
  - `--sram` / `--save-sram` load and write battery RAM.
  - `--switch-language FRAME:TAG` switches language at a frame, as F9 does
    in the app.
  - It reports host time per game second.
- `tcg2-libretro --core CORE.so --rom … --frames N [--press …] [--shots N:DIR] [--wav PATH]`
  runs the ROM in a reference emulator (the pinned nixpkgs has
  `libretro.sameboy` and `libretro.gambatte`). It has the same input scripting,
  with colour correction off, for picture, sound and cost comparisons:

  ```sh
  SB=$(nix build --no-link --print-out-paths github:NixOS/nixpkgs/dc5d91f840324650bac8c379428c7037a416959a#libretro.sameboy)
  ./target/release/tcg2-libretro --core $SB/lib/retroarch/cores/sameboy_libretro.so --rom ../../rom/poketcg2.gbc --frames 2400 --shots 100:../../build/ref
  ```

  SameBoy runs its boot animation first: its frame N+226 is the kernel's
  frame N in the intro.

`--save-state FRAME:PATH` and `--load-state PATH` (headless) write and
resume save states. A resumed run ends on the same clock and memory hash as
an uninterrupted one. In the app, F5 saves `saves/quick.state` and F7 loads
it.

**Duels.** Random play rarely reaches a duel. The game's own startup debug
menu (`CallStartupDebugMenu`, unused in the retail flow) has *Duel*
(*たいせん*): a duel against a random opponent with the player's saved deck. Use `--call`
to open the menu from the start menu's frame wait, after making a save
with random play:

```sh
H=./target/release/tcg2-headless
$H --rom ../../rom/poketcg2.gbc --frames 40000 --random 7 --save-sram ../../build/save-7.sav --quiet
$H --rom ../../rom/poketcg2.gbc --sram ../../build/save-7.sav --frames 30000 \
   --press 700:start --press 1000:start --call 1150:CallStartupDebugMenu \
   --press 1250:down:4 --press 1270:down:4 --press 1290:down:4 --press 1310:down:4 --press 1330:down:4 \
   --press 1360:a --random 11@1450 --shots 600:../../build/shots/duel
```

`tcg2-lockstep` takes the same `--sram` and `--call`. The whole duel
matches the interpreter. When the duel returns, the start menu's
frame-function stack has been unbalanced by running a startup menu outside
startup, so the game soon calls a stale frame function. The console would do
the same, and the interpreter agrees.

**Windows.** The cross-built `tcg2-headless.exe` under Wine (`nixpkgs#wine64`,
with `WINEDLLOVERRIDES="mscoree,mshtml="`) ends on the same clock and
memory hash as the Linux build. `pokemon-tcg2.exe` with `SDL3.dll` beside it
runs under `xvfb-run wine`.

The desktop app runs headless under Xvfb with SDL's software renderer:

```sh
SDL_VIDEO_DRIVER=x11 SDL_RENDER_DRIVER=software TCG2_HOME=$PWD/build/apptest \
  xvfb-run -a ./src/runtime/target/release/pokemon-tcg2 --exit-after-frames 900 --screenshot build/apptest/end.png --stats
```

`--stats` reports frames, presented pictures, CPU time and main-thread wake-ups.
Under the software renderer the CPU figure includes scaling on the CPU. To
measure with the system's GPU drivers, use the release build
(`tools/release/shell.sh linux python3 tools/release/package.py linux --version v0.1`),
whose binaries link only glibc: the development shell's Nix-built SDL cannot
load a non-NixOS host's drivers. On an NVIDIA desktop, Xvfb (which has no GLX
here) needs `__GLX_VENDOR_LIBRARY_NAME=mesa
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json`.

## Translations

A language lives in `src/runtime/crates/tcg2-locale/locales/<tag>/`
([runtime](runtime.md#languages-tcg2-locale)):

| File | What |
| --- | --- |
| `text1.txt` … `text17.txt` | One entry per text of `src/decomp/src/text/textN.asm`, by label |
| `data.txt` | Texts the game keeps in its code banks, by label (`Owner.local` for local labels) |
| `font.txt` | The half-width font, 4×8 pixels per character |
| `fullwidth.txt` | Full-width characters the game's code writes itself, redrawn |
| `graphics.txt` | Pictures with lettering, redrawn (`[Label] WxH`, then rows of `.`, `:`, `x`, `#`) |

An entry is a `[Label]` line followed by the text's lines, as they appear on
screen. A line between backticks keeps its spaces. `<RAMNAME>`, `<RAMTEXT>`,
`<RAMNUM>` and the symbols (`<Lv>`, `<FIRE>`, …) are the game's control
codes. `src/text.rs` has the details.

```sh
cd src/runtime
cargo test --release -p tcg2-locale    # every text translated, drawable, within its box; pictures their original's size
```

The test reads the Japanese from `src/decomp`, so it needs no ROM. To see a
change, run the game (`tcg2-headless … --screenshot`, or the app). The fixed
limits that matter most:
- **Dialog** scrolls two lines at a time, at most 36 characters per line.
- **A question** with a Yes/No menu has one line.
- **A card's rules** take at most 6 lines.
- **A Pokédex entry** takes at most 3 lines.
- **Labels and menu items** are no wider than the room the Japanese had, at
  two English letters per Japanese character.
- **Card names** stay within 20 characters (24 for Trainers and Energy): lists
  pad the rest of a row after the name.

The game draws whatever the engine encodes, so a new language needs only
its files and a `Language` variant. One that is not written in Latin
letters needs a font for the half-width engine, or full-width text.

## Profiling

```sh
perf record -g ./src/runtime/target/release/tcg2-headless --rom rom/poketcg2.gbc --frames 30000 --random 7 --quiet
perf report --no-children
```

In play, drawing changed lines (`Ppu::render_span`) and sound synthesis
(`Apu::sync`) take most of the kernel's time. The translated game code is
under 5% of it.

## Rules

- Never edit `src/decomp/` for the port, not even behind conditionals. Integration
  belongs in `src/runtime` and the build tooling. After anything touches
  `src/decomp/`, run `make -C src/decomp` (which checks the SHA-1) and the
  lockstep checks.
- Never commit ROMs, saves or generated kernels: `rom/`, `build/` and the
  targets are ignored.
- The translator must stay exact. After changing `tcg2-recomp` or `tcg2-core`,
  run `--blocks 10` and a long `--random` lockstep before relying on a change.
