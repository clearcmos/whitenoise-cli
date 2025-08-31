# Whitenoise CLI

A customizable white noise generator for the command line, built with Rust for optimal performance and low latency audio output.

## Features

- 🎵 **Professional DSP filtering** with 8 distinct frequency bands
- 🎛️ **Real-time interactive control** - adjust frequencies while playing
- 🧠 **Perceptual normalization** - Fletcher-Munson compensation (toggle with 'N')
- 🎧 **Audio device selection** - choose your preferred output device
- ⚡ **Low latency** - built with Rust for real-time audio performance  
- 🖥️ **Cross-platform** - works on Linux, macOS, and Windows
- 🎯 **NixOS optimized** - includes proper Nix development environment
- 💻 **CLI-friendly** - perfect for terminal workflows and automation
- 💾 **Settings persistence** - remembers your frequency preferences

## Installation

### Prerequisites

- **NixOS/Linux**: Nix package manager (included by default on NixOS)
- **Audio System**: PipeWire, PulseAudio, or ALSA
- **Rust**: Provided automatically via Nix shell

### Build from Source

```bash
# Clone the repository
git clone <repository-url>
cd whitenoise-cli

# Enter development environment
nix-shell

# Build the application
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
# N   - Toggle perceptual normalization (Fletcher-Munson)
# Q   - Quit
```

**Interactive Interface:**
```
🎵 Interactive White Noise Generator
Mode: TECHNICAL (Flat response) - Press N to toggle
Controls: ↑/↓ select, ←/→ adjust, Q to quit

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

### Advanced Configuration

```bash
# Custom frequency range and sample rate
./target/release/whitenoise \
  --volume 0.2 \
  --min-freq 50 \
  --max-freq 15000 \
  --sample-rate 44100

# Ultra-quiet background noise
./target/release/whitenoise -v 0.05 -m 100 -M 8000
```

## Command Line Options

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--list-devices` | `-l` | - | List available audio devices |
| `--device` | `-d` | - | Select specific audio device |
| `--non-interactive` | - | - | Run without interactive UI |
| `--help` | `-h` | - | Show help information |

## Interactive Controls

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate between volume and frequency bands |
| `←/→` | Decrease/increase selected parameter |
| `N` | Toggle perceptual normalization (Fletcher-Munson) |
| `Q/Esc` | Quit application |

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

### 🧘 Focus & Concentration
```bash
# Gentle background noise for work
./target/release/whitenoise -v 0.15
```

### 😴 Sleep Aid
```bash
# Soft, low-frequency white noise
./target/release/whitenoise -v 0.2 -m 50 -M 2000
```

### 🎧 Audio Testing
```bash
# Full spectrum test on specific device
./target/release/whitenoise -v 0.5 -d "USB Headphones"
```

### 🔇 Tinnitus Relief
```bash
# Mid-frequency range, moderate volume
./target/release/whitenoise -v 0.3 -m 500 -M 8000
```

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
1. Try different sample rate: `./target/release/whitenoise -s 44100`
2. Check system audio settings
3. Ensure no other applications are using exclusive audio access

## Technical Details

- **Language**: Rust 2024 Edition
- **Audio Backend**: CPAL (Cross-Platform Audio Library)
- **Supported Systems**: Linux (ALSA/PipeWire), macOS (CoreAudio), Windows (WASAPI)
- **Memory Usage**: Minimal (~1-2MB RAM)
- **CPU Usage**: Very low (<1% on modern systems)

## Contributing

This project uses Nix for reproducible builds. See `CLAUDE.md` for detailed development instructions.

## License

[Add your preferred license here]

---

**Enjoy your customizable white noise experience! 🎵**