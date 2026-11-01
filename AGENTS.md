- Read README.md, docs/runtime.md and docs/development.md, then only the relevant
  sources. Start from a real target (a failing check, a fault, a profile) and
  inspect the first difference.
- src/decomp/ is the pret disassembly and stays a pure, byte-identical build of
  the original ROM (`make -C src/decomp` checks rom.sha1). It is the game's only
  implementation. Never edit it for the native port (not even behind `IF`). Put
  integration in src/runtime/ and the build tooling.
- The kernel is translated from src/decomp at build time (tcg2-recomp). Fix
  translation problems in the translator or in tcg2-core's machine, never in
  generated code. Keep translation exact: after touching tcg2-recomp or
  tcg2-core, run `tcg2-lockstep --blocks 10` and a long `--random` lockstep.
- The app reads game data only from the player's ROM in rom/ (git-ignored). Never
  commit ROMs, saves, generated kernels or extracted assets, and never embed game
  data in builds.
- Battery use is the port's purpose. Do not add per-cycle stepping, polling
  loops, busy waits or unconditional per-frame GPU work. Measure with
  tcg2-headless (host time per game second) and the app's `--stats`.
- Use the pinned Nix shells (`nix develop path:./nix/runtime`, `path:./nix` for
  the decomp alone). Record the exact commands you ran and their results, and say
  which checks need the private ROM.
- Prefer the simplest implementation. Do not keep obsolete compatibility paths.
