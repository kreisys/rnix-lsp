{
  sources ? import ./nix/sources.nix,
  pkgs ? import <nixpkgs> {},
  naersk ? pkgs.callPackage sources.naersk {},
}: naersk.buildPackage ./.
