# Packaging

Releases are built by `.github/workflows/release.yml` with the tools in
`tools/release/`; see [docs/releases.md](../docs/releases.md). Packages contain
the app, its read-me (`install.txt`), `THIRD_PARTY_NOTICES.txt` and an empty
`rom/` folder (`rom-readme.txt`), never game data.

| File | Use |
| --- | --- |
| `about.toml`, `about.hbs` | cargo-about's accepted licenses and notices template |
| `install.txt`, `rom-readme.txt` | The player's read-me and the note in `rom/` |
| `arch/PKGBUILD` | Builds for Arch Linux from source against the system's SDL3 (`makepkg -si`) |

A local release build of one platform, as CI makes it:

```sh
nix develop path:./nix/runtime#release -c bash -c 'cd src/runtime && cargo run --release --locked -p tcg2-recomp -- --decomp ../decomp --work ../../build/decomp --out ../../build/kernel'
nix develop path:./nix/runtime#release -c python3 tools/release/notices.py
tools/release/shell.sh linux python3 tools/release/package.py linux --version v0.1   # or: windows (shell "windows"), macos (shell "release")
nix develop path:./nix/runtime -c python3 tools/release/smoke.py linux --version v0.1 --graphical [--rom rom/poketcg2.gbc]
```
