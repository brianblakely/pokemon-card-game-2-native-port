{
  description = "Pinned tools for the Pokémon TCG 2 disassembly in src/decomp (RGBDS, its C tools and scripts)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
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
          pkgs = import nixpkgs { inherit system; };
          rgbds = import ./runtime/rgbds.nix { inherit pkgs; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rgbds
              gcc
              gnumake
              python3
              git
            ];
            shellHook = ''
              export PYTHONDONTWRITEBYTECODE=1
              export SOURCE_DATE_EPOCH=0
            '';
          };
        }
      );
    };
}
