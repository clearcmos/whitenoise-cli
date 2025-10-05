# Next Steps for Whitenoise CLI Installation

## Current Issue
`cargo install --path .` fails because it runs outside the nix-shell environment and can't find ALSA libraries.

## Solutions

### Option 1: Manual Binary Copy (Quick Fix)
```bash
# Build in nix-shell
nix-shell --run "cargo build --release"

# Copy binary to PATH
cp target/release/whitenoise ~/.local/bin/
# or system-wide: sudo cp target/release/whitenoise /usr/local/bin/
```

**Note**: Binary still depends on nix-shell libraries, may not work outside shell.

### Option 2: Proper Nix Package (Recommended)
Create `default.nix`:

```nix
{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
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
}
```

Then install with:
```bash
nix-env -f . -i
```

This creates a properly packaged binary that works anywhere on the system.

### Option 3: System ALSA Libraries
Add to NixOS configuration.nix:
```nix
environment.systemPackages = with pkgs; [
  alsa-lib
  alsa-lib.dev
];
```

Then `cargo install --path .` would work, but this pollutes the system.

## Recommendation
Use Option 2 for a clean, NixOS-native installation that works system-wide.