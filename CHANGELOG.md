# Changelog

## 0.3.0 - 2026-07-20

### Added

- Pink and brown noise styles. Pink uses a matched-Z pole/zero ladder with a startup-solved correction zero, accurate to about 0.25 dB of the ideal -3 dB/octave slope from 20 Hz to 20 kHz at any common sample rate; brown uses a leaky integrator with an exact closed-form gain. Both are RMS-matched to the white source. The S key and `--style` cycle white, pink, brown, and rain.

### Fixed

- Adjusting low EQ bands (especially Sub Bass) could make the filter blow up into loud distortion, or go permanently silent at high sample rates. EQ changes are now smoothed in the gain domain with coefficients rebuilt from the smoothed gain, instead of interpolating raw biquad coefficients. Filters also flush non-finite state so a band can never stay poisoned.

### Verification

- Harden CI: SHA-pinned actions, least-privilege permissions, concurrency cancellation, per-job timeouts, an aggregate gate job, and Dependabot for workflow actions.
- Gate coverage in CI at 70 percent lines via cargo-llvm-cov.
- Add tests for device name matching, interactive key handling, settings file persistence, EQ stability while sliders move, colored-noise spectral slopes and levels, and style-switching crossfades.

## 0.2.0 - 2026-07-11

### Fixed

- Generate one sample per audio frame instead of once per interleaved channel, restoring correct rain speed and channel timing.
- Build streams for the device's actual integer or floating-point PCM format rather than assuming `f32`.
- Make non-interactive mode reject an accidental zero-volume infinite run.
- Stop probing every ALSA bridge during device listing, which could block when an audio server was unavailable.
- Decode the embedded WAV without silently discarding corrupt samples.
- Restore terminal state through an RAII guard on normal exits and errors.

### Audio

- Replace the parallel fixed-Q band-pass bank with a neutral serial eight-band peaking EQ.
- Generate genuine wideband white noise at neutral EQ settings.
- Normalize and peak-condition the quiet, high-crest-factor rain recording.
- Add smoothed volume/EQ changes and an equal-power 200 ms source crossfade.
- Move settings synchronization out of the per-sample path and use non-blocking snapshots once per callback buffer.
- Add a bounded soft limiter and sample-rate-aware EQ behavior.

### Changed

- Rename the `Vanilla` source to `White Noise` while retaining settings compatibility.
- Replace the fixed "Fletcher-Munson" claim with an explicitly heuristic listening contour.
- Add host selection, sound-style selection, percentage volume, EQ reset, and deterministic device matching.
- Make the native Rust toolchain the primary documented build path; retain Nix as optional packaging.
- Update CPAL, Crossterm, Rand, Dirs, TOML, and the lockfile.
- Split the monolithic executable into audio, device, settings, UI, and lifecycle modules.

### Verification

- Add regression tests for white-noise statistics, rain level/resampling, neutral EQ, PCM conversion, interleaved channel handling, settings migration, and extreme DSP settings.
- Add formatting, lint, test, and release-build CI.

## 0.1.0 - 2025-12-24

- Initial interactive white-noise generator and embedded rain loop.
