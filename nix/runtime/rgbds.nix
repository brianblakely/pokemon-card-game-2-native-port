# RGBDS at the version src/decomp/.rgbds-version names (nixpkgs carries 1.0.3).
{ pkgs }:
pkgs.rgbds.overrideAttrs (_: {
  version = "1.0.4";
  src = pkgs.fetchFromGitHub {
    owner = "gbdev";
    repo = "rgbds";
    rev = "v1.0.4";
    hash = "sha256-NNw0ZsUEpbmTGq8GZqAszi3Im1Pj331xNSYa7Aq2wWM=";
  };
})
