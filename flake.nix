{
  description = "A customizable white noise generator CLI with rain sounds";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        whitenoise = pkgs.rustPlatform.buildRustPackage {
          pname = "whitenoise";
          version = "0.1.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = with pkgs; [
            alsa-lib
          ];

          meta = with pkgs.lib; {
            description = "Interactive white noise generator with frequency band control and rain sounds";
            homepage = "https://github.com/clearcmos/whitenoise-cli";
            license = licenses.mit;
            maintainers = [];
            platforms = platforms.linux;
          };
        };
      in
      {
        packages = {
          default = whitenoise;
          whitenoise = whitenoise;
        };

        apps.default = {
          type = "app";
          program = "${whitenoise}/bin/whitenoise";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            rust-analyzer
            pkg-config
            alsa-lib
            alsa-lib.dev
          ];

          PKG_CONFIG_PATH = "${pkgs.alsa-lib.dev}/lib/pkgconfig";

          shellHook = ''
            echo "Whitenoise development environment"
            echo "Run 'cargo build' to build, 'cargo run' to test"
          '';
        };
      }
    ) // {
      # Overlay for easy integration into NixOS configs
      overlays.default = final: prev: {
        whitenoise = self.packages.${prev.system}.whitenoise;
      };
    };
}
