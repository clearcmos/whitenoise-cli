# Follow-up Work

The 0.2 audit fixed the known correctness and gain-staging defects. These items still need real-world validation or product decisions:

1. Listen on speakers, wired headphones, and Bluetooth at both 44.1 and 48 kHz. Automated tests can catch timing, bounds, and neutral-EQ errors, but not whether the rain conditioning feels natural.
2. Recover and record the original BigSoundBank catalog URL for `assets/rain_loop.wav`. The repository retains the claimed CC0 source and a checksum, but not the exact source page.
3. Decide whether to keep mono-compatible output or add intentional stereo decorrelation. Any stereo implementation must preserve one rain timeline per frame.
4. Re-evaluate CPAL's native PipeWire backend after its `libspa-sys` bindings work with current PipeWire 1.6 headers. The ALSA compatibility path and optional PulseAudio host are usable now.
5. Test macOS and Windows runners/hardware before advertising those platforms as verified rather than CPAL-supported.
