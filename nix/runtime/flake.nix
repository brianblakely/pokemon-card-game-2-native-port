{
  description = "Pinned runtime tools: Rust, SDL3, RGBDS for the decomp the kernel is translated from, and the release shells";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  # 26.11 removed Intel macOS. Its release host uses the supported 26.05
  # package set, locked separately instead of bypassing the platform check.
  inputs.nixpkgs-intel-macos.url = "github:NixOS/nixpkgs/nixos-26.05";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

  outputs =
    {
      nixpkgs,
      nixpkgs-intel-macos,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          packageSet = if system == "x86_64-darwin" then nixpkgs-intel-macos else nixpkgs;
          pkgs = import packageSet {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          rust = pkgs.rust-bin.stable."1.97.1".minimal.override {
            extensions = [
              "clippy"
              "rustfmt"
              "rust-src"
            ];
            # Windows is cross-built here; the Apple targets are for
            # `cargo check` of the macOS code paths (releases build macOS on
            # macOS).
            targets = [
              "x86_64-pc-windows-gnu"
              "aarch64-apple-darwin"
              "x86_64-apple-darwin"
            ];
          };
          rgbds = import ./rgbds.nix { inherit pkgs; };
          # The decomp's own build (src/decomp): the kernel is translated from
          # its matched ROM and symbol file.
          decompTools = with pkgs; [
            rgbds
            gcc
            gnumake
            python3
          ];
          windowsSdl = pkgs.pkgsCross.mingwW64.sdl3.override {
            traySupport = false;
            ibusSupport = false;
            openglSupport = false;
            vulkanSupport = false;
          };
          # Libraries SDL3 opens at run time in the development shell.
          runtimeLibraries =
            with pkgs;
            [ sdl3 ]
            ++ lib.optionals stdenv.hostPlatform.isLinux [
              vulkan-loader
              libGL
              wayland
              libxkbcommon
              libx11
              libxcursor
              libxi
              libxrandr
            ];
          # Headers for SDL's Linux backends (X11, Wayland, OpenGL/Vulkan,
          # PipeWire/PulseAudio/ALSA, udev, D-Bus); every library is opened
          # at run time, none is linked or shipped.
          linuxHeaders = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            with pkgs;
            [
              alsa-lib
              libpulseaudio
              pipewire
              libx11
              libxcb
              libxext
              libxcursor
              libxi
              libxrandr
              libxfixes
              libxrender
              libxscrnsaver
              libxtst
              xorgproto
              libxkbcommon
              wayland
              wayland-protocols
              libdecor
              libGL
              vulkan-headers
              dbus
              systemdLibs
            ]
          );
          linuxTestEnv = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath runtimeLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
          '';
        in
        {
          default = pkgs.mkShell {
            packages =
              (with pkgs; [
                rust
                git
                pkg-config
                ruff
                nixfmt
                (python3.withPackages (python: [ python.pillow ]))
              ])
              ++ decompTools
              ++ runtimeLibraries
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                pkgs.perf
                pkgs.xvfb-run
                pkgs.weston
              ];
            shellHook = ''
              export PYTHONDONTWRITEBYTECODE=1
              export SOURCE_DATE_EPOCH=0
            ''
            + linuxTestEnv;
          };
          # Release hosts use the same lock and toolchain as development: the
          # prepare job (translation, notices) and the macOS packages.
          release = pkgs.mkShell {
            packages =
              (with pkgs; [
                rust
                pkg-config
                git
                zip
                unzip
                sdl3
                cargo-about
                nix
              ])
              ++ decompTools;
            shellHook = ''
              export PYTHONDONTWRITEBYTECODE=1
              export SOURCE_DATE_EPOCH=0
              export TCG2_SDL3_SRC=${pkgs.sdl3.src}
              export TCG2_NIXPKGS_SOURCE=${packageSet.outPath}
            '';
          };
          # Builds the Linux release: SDL3 compiled from source into the app
          # and linked against glibc 2.31 with zig, so the one binary runs on
          # any distribution since 2020 and uses the system's own graphics,
          # sound and input libraries (SDL opens them at run time). The
          # headers below are all SDL needs to compile those backends in.
          linux = pkgs.mkShell {
            packages =
              (with pkgs; [
                rust
                cargo-zigbuild
                zig
                cmake
                pkg-config
                wayland-scanner
                git
                python3
                xvfb-run
              ])
              ++ linuxHeaders;
            shellHook = ''
              export PYTHONDONTWRITEBYTECODE=1
              export SOURCE_DATE_EPOCH=0
              # zig's compiler ignores the Nix compiler wrapper's include
              # paths: hand them to SDL's CMake build directly.
              export CFLAGS="$NIX_CFLAGS_COMPILE"
              export TCG2_LINUX_GLIBC=2.31
            '';
          };
          # Cross-builds the Windows desktop host (x86_64-pc-windows-gnu).
          windows = pkgs.mkShell {
            packages =
              (with pkgs; [
                rust
                pkg-config
                git
                zip
                pkgsCross.mingwW64.stdenv.cc
                windowsSdl
                nix
              ])
              ++ decompTools;
            shellHook = ''
              export PYTHONDONTWRITEBYTECODE=1
              export SOURCE_DATE_EPOCH=0
              export CC_x86_64_pc_windows_gnu=${pkgs.pkgsCross.mingwW64.stdenv.cc}/bin/x86_64-w64-mingw32-cc
              export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=$CC_x86_64_pc_windows_gnu
              export TCG2_WINDOWS_SDL_DLL=${windowsSdl.out}/bin/SDL3.dll
              export TCG2_WINDOWS_LIBRARIES='${
                builtins.toJSON [
                  windowsSdl
                  windowsSdl.out
                  pkgs.pkgsCross.mingwW64.stdenv.cc.cc.lib
                  pkgs.pkgsCross.mingwW64.stdenv.cc.libc
                  pkgs.pkgsCross.mingwW64.windows.pthreads
                  pkgs.pkgsCross.mingwW64.windows.mcfgthreads
                ]
              }'
              export TCG2_SDL3_SRC=${pkgs.sdl3.src}
              export TCG2_NIXPKGS_SOURCE=${nixpkgs.outPath}
              export SDL3_LIB_DIR=${windowsSdl}/lib
              export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L native=${pkgs.pkgsCross.mingwW64.windows.pthreads}/lib -L native=${pkgs.pkgsCross.mingwW64.windows.mcfgthreads}/lib -C link-arg=-lmcfgthread"
            '';
          };
        }
      );
    };
}
