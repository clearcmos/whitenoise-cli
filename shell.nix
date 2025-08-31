{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    rustc
    cargo
    pkg-config
    alsa-lib
    alsa-lib.dev
  ];
  
  PKG_CONFIG_PATH = "${pkgs.alsa-lib.dev}/lib/pkgconfig";
  
  shellHook = ''
    echo "Rust development environment with ALSA support"
    echo "PKG_CONFIG_PATH set to: $PKG_CONFIG_PATH"
  '';
}