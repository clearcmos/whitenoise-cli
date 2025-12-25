# Whitenoise CLI

A customizable white noise generator for the command line, built with Rust for optimal performance and low latency audio output.

## Features

- **Sound styles** - Vanilla white noise or realistic Rain (embedded CC0 sample)
- **Professional DSP filtering** with 8 distinct frequency bands
- **Real-time interactive control** - adjust frequencies while playing
- **Perceptual normalization** - Fletcher-Munson compensation (toggle with 'N')
- **Audio device selection** - choose your preferred output device
- **Low latency** - built with Rust for real-time audio performance
- **Cross-platform** - works on Linux, macOS, and Windows
- **NixOS optimized** - includes proper Nix development environment
- **CLI-friendly** - perfect for terminal workflows and automation
- **Settings persistence** - remembers your frequency preferences

## Installation

### NixOS / Nix Flakes (Recommended)

Add to your NixOS configuration:

```nix
{
  inputs.whitenoise.url = "github:clearcmos/whitenoise-cli";

  # Add to your system packages
  environment.systemPackages = [
    inputs.whitenoise.packages.x86_64-linux.default
  ];
}
```

Or run directly without installing:

```bash
nix run github:clearcmos/whitenoise-cli
```

### Build from Source

```bash
# Clone the repository
git clone https://github.com/clearcmos/whitenoise-cli
cd whitenoise-cli

# Using flakes
nix develop
cargo build --release

# Or using legacy nix-shell
nix-shell
cargo build --release

# The binary will be available at: target/release/whitenoise
```

## Usage

### Interactive Mode (Recommended)

```bash
# Start interactive mode with real-time frequency control
./target/release/whitenoise

# Controls:
# ↑/↓ - Select frequency band or volume
# ←/→ - Adjust selected parameter
# S   - Switch sound style (Vanilla/Rain)
# N   - Toggle perceptual normalization (Fletcher-Munson)
# Q   - Quit
```

**Interactive Interface:**
```
Interactive White Noise Generator
Sound Style: Vanilla (Adjustable) - Press S to switch
Mode: TECHNICAL (Flat response) - Press N to toggle
Controls: ↑/↓ select, ←/→ adjust, S style, N mode, Q to quit

► Volume      [██████████░░░░░░░░░░░░░░░░░░░░] 30.0%
  Sub Bass    [███████████████░░░░░░░░░░░░░░] 50.0%
  Bass        [███████████████░░░░░░░░░░░░░░] 50.0%
  Low Mid     [███████████████░░░░░░░░░░░░░░] 50.0%
  Mid         [███████████████░░░░░░░░░░░░░░] 50.0%
  Hi Mid      [███████████████░░░░░░░░░░░░░░] 50.0%
  Presence    [███████████████░░░░░░░░░░░░░░] 50.0%
  Brilliance  [███████████████░░░░░░░░░░░░░░] 50.0%
  Air         [███████████████░░░░░░░░░░░░░░] 50.0%
```

### Non-Interactive Mode

```bash
# Run without UI (uses saved settings)
./target/release/whitenoise --non-interactive
```

### Device Selection

```bash
# List available audio devices
./target/release/whitenoise --list-devices

# Select specific device (e.g., use PipeWire directly)
./target/release/whitenoise --device pipewire

# Use Bluetooth headphones
./target/release/whitenoise --device bluetooth
```

## Command Line Options

| Option | Short | Description |
|--------|-------|-------------|
| `--list-devices` | `-l` | List available audio devices |
| `--device` | `-d` | Select specific audio device |
| `--non-interactive` | | Run without interactive UI (uses saved settings) |
| `--help` | `-h` | Show help information |

## Interactive Controls

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate between volume and frequency bands |
| `←/→` | Decrease/increase selected parameter |
| `S` | Switch sound style (Vanilla/Rain) |
| `N` | Toggle perceptual normalization (Fletcher-Munson) |
| `Q/Esc` | Quit application |

## Sound Styles

| Style | Description |
|-------|-------------|
| **Vanilla** | Pure white noise with full frequency band control |
| **Rain** | Realistic rain recording with seamless looping and EQ control |

The Rain style uses an embedded CC0-licensed recording from BigSoundBank, with crossfade looping for seamless playback. Frequency bands still apply to shape the rain sound.

## Frequency Bands

| Band | Range | Purpose |
|------|-------|---------|
| **Sub Bass** | 20-60 Hz | Deep rumble, felt more than heard |
| **Bass** | 60-250 Hz | Low-end punch, warmth |
| **Low Mid** | 250-500 Hz | Body, thickness |
| **Mid** | 500-2000 Hz | Clarity, speech intelligibility |
| **Hi Mid** | 2000-4000 Hz | Presence, definition |
| **Presence** | 4000-6000 Hz | Consonants, detail |
| **Brilliance** | 6000-12000 Hz | Sparkle, air |
| **Air** | 12000-20000 Hz | Ultra-high shimmer |

## Perceptual Normalization (Fletcher-Munson)

Press **'N'** to toggle between two modes:

### Technical Mode (Default) 
- **Professional control**: Each slider controls actual frequency amplitude
- **Flat response**: What audio engineers prefer for technical work
- **Unequal perceived loudness**: Bands will sound different in volume due to human hearing

### Perceptual Mode 
- **Equal loudness**: Bands sound more equally loud to human ears
- **Fletcher-Munson compensated**: Automatic boosting/cutting based on hearing sensitivity
- **Intuitive control**: More natural-feeling adjustments

**Compensation Applied in Perceptual Mode:**
- Sub Bass: +2.8x boost (we barely hear this naturally)
- Bass: +2.0x boost (still low sensitivity)
- Mid: Reference level (peak human hearing)
- Presence: -0.8x slight cut (we're very sensitive here)
- Air: +2.2x boost (we don't hear well at these frequencies)

## Use Cases

### Focus & Concentration
Start the app and adjust volume/EQ interactively for gentle background noise.

### Sleep Aid
Use Rain mode (press S) with reduced high frequencies for a soothing sound.

### Audio Testing
```bash
# Test on specific device
./target/release/whitenoise -d "USB Headphones"
```

### Tinnitus Relief
Adjust mid-frequency bands interactively to find the most comfortable masking sound.

## Stopping the Application

Press `Ctrl+C` to gracefully stop the white noise generator.

## Troubleshooting

### No Audio Output
1. Check available devices: `./target/release/whitenoise --list-devices`
2. Test with different device: `./target/release/whitenoise -d pipewire`
3. Verify volume isn't too low: `./target/release/whitenoise -v 0.5`

### Build Issues on NixOS
1. Ensure you're in the nix-shell: `nix-shell`
2. Clean build: `cargo clean && cargo build --release`

### Audio Latency/Crackling
1. Check system audio settings (PipeWire/PulseAudio configuration)
2. Ensure no other applications are using exclusive audio access
3. Try a different audio device: `./target/release/whitenoise -d pipewire`

## Technical Details

- **Language**: Rust 2024 Edition
- **Audio Backend**: CPAL (Cross-Platform Audio Library)
- **Supported Systems**: Linux (ALSA/PipeWire), macOS (CoreAudio), Windows (WASAPI)
- **Binary Size**: ~2.5MB (includes embedded rain sample)
- **Memory Usage**: ~3-4MB RAM
- **CPU Usage**: Very low (<1% on modern systems)

## Contributing

This project uses Nix for reproducible builds. See `CLAUDE.md` for detailed development instructions.

## License

MIT License - See LICENSE file for details

---

Enjoy your customizable white noise experience!