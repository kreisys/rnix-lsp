{ pkgs ? import <nixpkgs> {}
}: {
  rnix-lsp = pkgs.rustPlatform.buildRustPackage rec {
    pname = "rnix-lsp";
    version = "0.1.0";
    src = ./.;
    cargoSha256 = "021zcdz3dynrm78hips0jyf0xfkrxa0f28swalgx7vqz6jj8yi8a";
  };
}

