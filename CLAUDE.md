# Claude Development Notes

## Project Overview
A customizable whitenoise CLI tool written in Rust, designed for NixOS/KDE/Wayland with PipeWire audio support.

## Development Setup

### Prerequisites
- NixOS with Nix package manager
- PipeWire audio system (detected: PipeWire 1.4.7)

### Build Environment
Use flakes (recommended) or legacy nix-shell:

```bash
# Using flakes
nix develop
cargo build

# Or legacy nix-shell
nix-shell
cargo build
```

The dev environment provides:
- Rust toolchain
- ALSA development libraries
- Proper PKG_CONFIG_PATH configuration

### Key Dependencies
- `cpal` (0.15) - Cross-platform audio library
- `clap` (4.0) - CLI argument parsing with derive features
- `rand` (0.8) - Random number generation with `small_rng` feature
- `anyhow` (1.0) - Error handling
- `ctrlc` (3.0) - Signal handling for graceful shutdown
- `hound` (3.5) - WAV audio decoding for embedded rain sample

## Architecture

### Core Components
1. **NoiseGenerator** - Thread-safe audio generation with EQ band processing
2. **RainSamplePlayer** - Embedded WAV playback with crossfade looping
3. **FrequencyBandGenerator** - Biquad-filtered noise for each EQ band
4. **Audio Stream** - Real-time audio output via cpal with mutex-protected generator
5. **CLI Interface** - Clap-based argument parsing with customization options
6. **Device Management** - Audio device discovery and selection

### Embedded Assets
- `assets/rain_loop.wav` - 15-second CC0 rain sample from BigSoundBank
- Embedded at compile time via `include_bytes!`
- Seamless looping with 2-second S-curve crossfade

### Thread Safety
- Uses `Arc<Mutex<NoiseGenerator>>` for cross-thread access in audio callback
- `Arc<AtomicBool>` for clean shutdown signaling
- SmallRng instead of ThreadRng for Send trait compatibility

## Build Commands

```bash
# Development build
nix-shell --run "cargo build"

# Release build  
nix-shell --run "cargo build --release"

# Run with specific device
nix-shell --run "./target/debug/whitenoise -d pipewire"
```

## Testing Commands

```bash
# List available audio devices
nix-shell --run "./target/debug/whitenoise --list-devices"

# Interactive mode with real-time frequency control
nix-shell --run "./target/debug/whitenoise"

# Test perceptual normalization toggle (press 'N' while running)
nix-shell --run "./target/debug/whitenoise"

# Non-interactive mode
nix-shell --run "./target/debug/whitenoise --non-interactive"
```

## Technical Notes

### Sound Styles
- **Vanilla**: Pure white noise split into 8 frequency bands with independent gain
- **Rain**: Embedded WAV sample with crossfade looping, processed through same EQ

### Audio DSP Implementation
- **Biquad filters**: Professional-grade lowpass, highpass, and bandpass filters
- **Frequency band separation**: 8 distinct bands with proper center frequencies
- **Perceptual normalization**: Fletcher-Munson compensation for equal loudness
- **Real-time parameter updates**: All settings change instantly during playback
- **Soft limiting**: Prevents harsh clipping when multiple bands are active
- **Crossfade looping**: S-curve (cosine) blend for seamless rain sample loops
- **Linear interpolation resampling**: Adapts embedded 44.1kHz sample to output rate

### Perceptual Normalization (Fletcher-Munson)
- **Technical mode** (default): Flat frequency response for professional control
- **Perceptual mode** ('N' key): Compensated for human hearing sensitivity
- **Compensation factors**:
  - Sub Bass (<100Hz): 2.8x boost
  - Bass (100-500Hz): 2.0x boost  
  - Mid (500-4000Hz): 1.0x reference
  - Presence (4-6kHz): 0.8x slight cut
  - Air (>10kHz): 2.2x boost

### Audio System Compatibility
- Works with PipeWire (primary)
- Falls back to ALSA
- Supports device selection by name matching

### Rust Edition & Compatibility
- Uses Rust 2024 edition
- Handles `gen` keyword conflict with `r#gen` escape
- SmallRng requires explicit feature flag for thread safety

### NixOS Integration
- `shell.nix` provides ALSA development headers
- PKG_CONFIG_PATH properly configured for alsa-sys compilation
- No system-level audio library installation required

## Troubleshooting

### Build Issues
- If alsa-sys fails: ensure you're in nix-shell
- Missing SmallRng: verify rand features include "small_rng"
- Keyword conflicts: use r# prefix for reserved words

### Runtime Issues
- No audio output: check device selection with `--list-devices`
- Permission errors: ensure user in audio group
- Latency issues: check PipeWire/PulseAudio settings