# Packaging

Releases are built by `.github/workflows/release.yml` with the tools in
`tools/release/`; see [docs/releases.md](../docs/releases.md). Packages contain
the app, its read-me (`install.txt`), `THIRD_PARTY_NOTICES.txt` and an empty
`rom/` folder (`rom-readme.txt`), never game data. The Android APK carries the
app and `THIRD_PARTY_NOTICES.txt`; it asks for the ROM itself.

| File | Use |
| --- | --- |
| `about.toml`, `about.hbs` | cargo-about's accepted licenses and notices template |
| `install.txt`, `rom-readme.txt` | The player's read-me and the note in `rom/` |
| `arch/PKGBUILD` | Builds for Arch Linux from source against the system's SDL3 (`makepkg -si`) |

A local release build of one platform, as CI makes it:

```sh
nix develop path:./nix/runtime#release -c bash -c 'cd src/runtime && cargo run --release --locked -p tcg2-recomp -- --decomp ../decomp --work ../../build/decomp --out ../../build/kernel'
nix develop path:./nix/runtime#release -c python3 tools/release/notices.py
tools/release/shell.sh linux python3 tools/release/package.py linux --version v0.1   # or: windows (shell "windows"), macos (shell "release"), android (shell "android")
nix develop path:./nix/runtime -c python3 tools/release/smoke.py linux --version v0.1 --graphical [--rom rom/poketcg2.gbc]
tools/release/shell.sh android python3 tools/release/smoke.py android --version v0.1 [--rom rom/poketcg2.gbc]   # on an emulator
```

The Android APK's manifest and activity are in `src/runtime/android/`; its
signing key is described in [docs/releases.md](../docs/releases.md#the-android-signing-key).
