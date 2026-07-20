# Whitenoise CLI

`whitenoise` is a small Rust terminal application for continuous white, pink, and brown noise and rain ambience. It provides a master volume, an eight-band graphic EQ, live source switching, settings persistence, and explicit audio host/device selection.

The current release is `0.2.0`. It requires Rust 1.85 or newer.

## What works

- Neutral, wideband white noise from a fast per-stream PRNG
- Pink and brown noise from filters designed at startup for the actual device sample rate; pink stays within about 0.25 dB of the ideal -3 dB/octave slope from 20 Hz to 20 kHz
- A real 15-second mono rain recording with resampling and a two-second equal-power loop crossfade
- Automatic rain level normalization and peak conditioning so the ambience is audible without clipping isolated drops
- Eight serial peaking-EQ filters from 20 Hz to 20 kHz; the center position is a true 0 dB bypass
- Smoothed volume, EQ, and 200 ms source transitions to avoid clicks
- Correct interleaved output: one source frame is generated and then copied to every device channel
- Integer and floating-point PCM output formats supported by CPAL
- Non-blocking settings snapshots in the real-time audio callback
- Interactive terminal UI and script-friendly non-interactive mode
- Legacy `Vanilla`/`perceptual_normalization` settings migration

## Build

The normal build uses the native Rust toolchain. Nix is optional.

### Linux prerequisites

CPAL requires ALSA development headers on Linux, including systems whose audio server is PipeWire.

Arch Linux:

```bash
sudo pacman -S rust alsa-lib pkgconf
```

Debian or Ubuntu:

```bash
sudo apt install cargo rustc pkg-config libasound2-dev
```

Fedora:

```bash
sudo dnf install cargo rust alsa-lib-devel pkgconf-pkg-config
```

Then build normally:

```bash
cargo build --release
```

Install for the current user with:

```bash
cargo install --path .
```

The existing flake remains available for Nix users:

```bash
nix develop
cargo build --release
```

### Optional PulseAudio host

The default Linux build uses CPAL's ALSA host. This works with PipeWire through its ALSA compatibility device on a normal desktop. To compile CPAL's PulseAudio host instead (also usable through `pipewire-pulse`), install the PulseAudio development package and build with:

```bash
cargo build --release --features pulseaudio
./target/release/whitenoise --host pulseaudio
```

Use `--list-hosts` to see which hosts were compiled into a particular binary.

## Usage

Interactive mode starts muted for headphone safety unless an initial volume is supplied:

```bash
whitenoise
whitenoise --volume 20 --style rain
```

Controls:

| Key | Action |
| --- | --- |
| Up / Down | Select volume or an EQ band |
| Left / Right | Adjust the selected control |
| S | Cycle white, pink, brown, and rain |
| N | Toggle the gentle listening contour |
| R | Reset every EQ band to 0 dB |
| Q / Esc | Quit |

Non-interactive mode uses saved settings and accepts explicit overrides:

```bash
whitenoise --non-interactive --volume 20 --style white
whitenoise --non-interactive --volume 15 --style brown
whitenoise --non-interactive --volume 15 --style rain
```

If neither `--volume` nor a non-zero saved volume is available, non-interactive mode exits with an explanation instead of silently playing nothing.

Device and host discovery:

```bash
whitenoise --list-hosts
whitenoise --list-devices
whitenoise --device "USB Headphones"
whitenoise --device pipewire
```

Device matching prefers a case-insensitive exact name, then accepts a unique substring. Ambiguous matches are reported rather than selecting an arbitrary device.

Full options:

```text
Usage: whitenoise [OPTIONS]

Options:
      --list-hosts
  -l, --list-devices
      --host <HOST>
  -d, --device <DEVICE>
      --non-interactive
  -v, --volume <PERCENT>
  -s, --style <STYLE>       [possible values: white, pink, brown, rain]
  -h, --help
  -V, --version
```

## EQ and listening contour

Each band ranges from -12 dB to +12 dB. A slider at 50% is 0 dB, so the default EQ does not color either source.

| Band | Range |
| --- | --- |
| Sub Bass | 20-60 Hz |
| Bass | 60-250 Hz |
| Low Mid | 250-500 Hz |
| Mid | 500-2,000 Hz |
| High Mid | 2,000-4,000 Hz |
| Presence | 4,000-6,000 Hz |
| Brilliance | 6,000-12,000 Hz |
| Air | 12,000-20,000 Hz |

The optional listening contour is a conservative convenience curve. It is not described as Fletcher-Munson compensation because a valid equal-loudness correction depends on listening level, transducer response, and the listener.

## Settings

Settings are stored in the platform configuration directory:

- Linux: usually `~/.config/whitenoise/settings.toml`
- macOS: under the user's Application Support directory
- Windows: under the user's roaming application-data directory

Malformed settings are reported and safe defaults are used. Numeric settings are clamped before they reach the audio engine.

## Audio design

White noise begins as a single uniform random signal with constant expected spectral density.

Pink noise shapes that signal with a ladder of matched-Z pole/zero pairs spaced two octaves apart, plus one correction zero solved numerically at startup for the actual sample rate, keeping the response within about 0.25 dB of the ideal -3 dB/octave slope from 20 Hz to 20 kHz at common rates. Brown noise uses a leaky integrator with its leak at 8 Hz, below the audible band, and an exact closed-form output gain. Both are level-matched to the white source by RMS rather than by any claimed perceptual weighting.

Sources are combined with an equal-power crossfade and pass through a serial graphic EQ whose gains are smoothed in the dB domain. At neutral settings every biquad is exactly the identity transform, avoiding the gaps, overlaps, and phase-heavy recombination of the previous parallel band-pass implementation.

The rain WAV is decoded once at startup, downmixed if necessary, linearly resampled to the device rate, and looped with an equal-power crossfade. Its original recording has a high crest factor, so a measured normalization gain and static peak compression bring up the rain bed while retaining drop transients.

Output is currently mono-compatible: the same generated frame is copied to all output channels. This preserves the timing of the mono rain recording and avoids advancing any source once per channel.

## Development

```bash
cargo fmt --all --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Unit tests cover settings migration and sanitization, neutral-EQ transparency, EQ stability while sliders move, pink and brown spectral slopes and levels, output frame/channel handling, rain asset decoding and resampling, limiter bounds, style-switching crossfades, and long extreme-setting runs.

## Rain asset

`assets/rain_loop.wav` is recorded in repository history as a CC0 rain-on-puddles sample from BigSoundBank. See `assets/README.md` for the retained provenance and checksum.

## License

The program source is licensed under the MIT License. See [LICENSE](LICENSE).
