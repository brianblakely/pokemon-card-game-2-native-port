# Releases and portable installation

Pushes to `master` build a GitHub release only when native app inputs change
(`tools/release/version.py --check-native-since`):
- **Inputs:** the decomp, the runtime crates (not the test tooling), `Cargo.lock`, `nix/runtime/`, the player read-me and the packaging scripts.
- **Not inputs:** documentation, tests, examples and workflow-only edits.

The workflow (`.github/workflows/release.yml`) compares the whole push before
reserving a tag:
- **Versions:** they start at `v0.1`, and each release increments the patch
  (`v0.1.1`, `v0.1.2`, …). A retry of the same commit reuses its tag.
- **Publishing:** it waits for the complete artifact set and the automated
  checks. Published files are immutable: a retry must reproduce their
  checksums (`SHA256SUMS`).
- **Rebuilds:** an explicit manual run rebuilds. Branches under `release/` run
  the same builds and checks without reserving a tag or publishing.

The release's jobs:

| Job | What |
| --- | --- |
| prepare | Reserves the version. Translates `src/decomp` to the kernel sources (`build/kernel`) once, in the pinned release shell. Audits dependency licenses (`tools/release/notices.py`, cargo-about with `packaging/about.toml`) and archives the authored source |
| checks | Workspace format, clippy (`-D warnings`) and tests with the released kernel; ruff; the decomp still rebuilds the original ROM |
| desktop | Builds and bundles every platform from the same kernel sources in the pinned Nix shells (`tools/release/package.py`), then runs the packaged executables (`tools/release/smoke.py`) |
| windows-test | Runs the packaged Windows build on Windows |
| publish | Publishes the notes, the artifacts and their checksums (`tools/release/publish.py`) |

**Release notes** use the guide in `tools/release/notes.md`. For each change
worth announcing to players, add a new `docs/changes/<short-description>.md`
with one or more short Markdown bullets. Describe what the player can do or
what now works, and leave code names and build details in the PR. The
publisher:
- includes only entries added since the previous published release on the
  commit's ancestry;
- says when no player-facing changes were noted;
- folds in every commit since that release (merges aside), newest first.

| Platform | Artifact | Graphics |
| --- | --- | --- |
| Linux x86-64 / AArch64 | `.tar.gz`: `pokemon-tcg2`, `tcg2-headless`, `rom/` (SDL3 compiled in; glibc 2.31+) | OpenGL, OpenGL ES or Vulkan, through the system's drivers |
| Windows x86-64 | ZIP: `pokemon-tcg2.exe`, `tcg2-headless.exe`, required DLLs, `rom/` | Direct3D 11 or 12 |
| macOS Intel / Apple Silicon | ZIP: `Pokémon Card GB2.app`, `tcg2-headless`, `rom/` | Metal |

Every release also carries:
- `THIRD_PARTY_NOTICES.txt`;
- the Windows and macOS `*-runtime-sources.tar.gz`: the sources and Nix build
  recipes of the libraries they bundle. Linux packages bundle no libraries:
  SDL3 is compiled into the app, and everything else is the system's;
- `*-source.tar.gz`: the port's authored files. `src/decomp` is pret's
  disassembly, kept out because it holds the game's extracted assets.

No release contains a ROM or any game data.

## Desktop installation

Extract the download to a writable folder and put your ROM of *Pokémon Card
GB2 - GR Dan Sanjou! (J)* in `rom/` beside the executable. The ROM is 2 MiB with
SHA-1 `a7e12bcc5f514e3aad8de570fd511aab0a308822`. The app keeps `saves/`,
`screenshots/` and `settings.ini` beside itself as well.

- **Windows:** keep the included DLLs beside the executables.
- **macOS:** the app is ad-hoc signed, not notarized. The first time, open it
  with right-click → **Open**. The folder beside the `.app` is used when it
  holds `rom/`, as the ZIP arrives. A copy moved to `/Applications` uses
  `~/Library/Application Support/pokemon-card-game-2-native-port/pokemon-tcg2/`.
- **Linux:** run `pokemon-tcg2` from the extracted folder. It needs glibc 2.31
  or newer (any distribution since 2020). SDL3 is compiled in, built with zig
  against that glibc (`nix develop path:./nix/runtime#linux`), and opens the
  system's X11 or Wayland, OpenGL or Vulkan, and PipeWire, PulseAudio or ALSA
  libraries at run time. The workflow's test runs it with the runner's own
  libraries under Xvfb (SDL's software renderer).

To update, replace the app and its included files and keep `rom/`, `saves/` and
`settings.ini`. Battery saves carry over between versions. Save states may need
the version that made them.

Arch Linux users can instead build against the system's libraries with
`packaging/arch/PKGBUILD`.
