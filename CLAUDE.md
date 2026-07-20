# Development Notes

## Project

`whitenoise` is a Rust 2024 desktop CLI for white noise and looped rain ambience. Linux is the currently exercised platform, but audio and terminal I/O use CPAL and Crossterm rather than Linux-specific application code.

The native Rust toolchain is the primary development environment. The Nix flake and `shell.nix` are optional compatibility paths.

## Prerequisites

- Rust 1.85 or newer
- Linux: `pkg-config` and ALSA development headers
- Optional `pulseaudio` feature: PulseAudio development headers

See `README.md` for distribution-specific package names.

## Commands

```bash
cargo build
cargo test
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Manual smoke checks:

```bash
cargo run -- --help
cargo run -- --list-hosts
cargo run -- --list-devices
cargo run -- --volume 10
cargo run -- --non-interactive --volume 10 --style rain
```

## Architecture

- `src/main.rs`: argument parsing, lifecycle, and startup safety
- `src/device.rs`: CPAL host/device discovery and deterministic name matching
- `src/settings.rs`: settings model, legacy migration, validation, and persistence
- `src/audio.rs`: rain decoding/looping, white/pink/brown sources, graphic EQ, smoothing, limiting, and typed CPAL callbacks
- `src/ui.rs`: interactive terminal rendering and controls
- `assets/rain_loop.wav`: embedded mono rain recording

## Real-time audio rules

- Generate once per audio frame, then populate every interleaved channel.
- Do not allocate, block, decode files, print, or take a blocking mutex in the audio callback.
- Read UI settings with `try_lock` once per callback buffer and retain the last snapshot on contention.
- Keep source and parameter changes ramped to prevent discontinuities.
- Smooth EQ changes in the gain (dB) domain and recompute biquad coefficients from the smoothed gain. Never interpolate raw biquad coefficients: the low bands have near-unit-circle poles and interpolated intermediates blow up (worst on sub bass, worse at higher sample rates).
- Neutral EQ must remain an exact identity transform.
- Any new DSP path needs finite/bounded-output tests at extreme settings.

## Behavior worth preserving

- Interactive mode starts muted unless `--volume` is supplied.
- Non-interactive mode must fail clearly rather than run indefinitely at zero volume.
- Legacy `sound_style = "Vanilla"` and `perceptual_normalization` settings remain readable.
- The listening contour is a heuristic preset, not a claimed equal-loudness calibration.
- Pink and brown filters are designed at startup for the actual sample rate; spectral-slope tests pin them to -3 and -6 dB/octave.
- The rain source advances once per output frame regardless of channel count.

## Audio backends

The default Linux build uses CPAL's ALSA host, which normally reaches PipeWire through its ALSA compatibility layer. A PulseAudio host can be compiled with `--features pulseaudio` and selected with `--host pulseaudio`.

Do not assume NixOS, KDE, Wayland, a particular PipeWire version, or a specific device name in code or documentation.

## Decision log

- 2026-07-19: EQ changes are smoothed in the gain (dB) domain and coefficients are rebuilt from the smoothed gain. Motivated by a real bug: per-sample linear interpolation of raw biquad coefficients drove the Sub Bass band (near-unit-circle poles) into transient blow-ups up to 44 dB over the signal at 48 kHz and to infinity at 96 kHz and above. Filters also flush non-finite state so a poisoned band recovers instead of going silent until restart.
- 2026-07-19: Pink noise is designed at startup for the actual device sample rate (matched-Z pole/zero ladder plus a bisection-solved correction zero) instead of using fixed 44.1 kHz coefficients, which are 3 to 5 dB off near 16 kHz at other rates. Verified to within about 0.25 dB of the ideal -3 dB/octave from 20 Hz to 20 kHz at rates from 22.05 to 384 kHz.
- 2026-07-20: Coverage is gated in CI at 60% lines via cargo-llvm-cov (measured 61.4% when the gate was added; device.rs and ui.rs had no tests yet). Ratchet to 70 once those modules gain tests; never lower the gate.
- 2026-07-20: Gate ratcheted to 70 (measured 72.8% after device name matching, UI key handling, and settings persistence gained tests). Documented coverage exemptions, all environment-bound rather than logic: main.rs lifecycle glue (stream startup, signal handling), ui.rs rendering and raw-terminal paths, and device.rs functions that talk to a live CPAL host (the name-matching contract itself is extracted and tested as match_device_name).
- 2026-07-20: Cargo dependency updates are deliberate and manual. Dependabot watches GitHub Actions only; CI enforces `--locked` everywhere so drift cannot slip in through a stale lockfile.
