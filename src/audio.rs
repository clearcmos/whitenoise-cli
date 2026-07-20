use std::f32::consts::{FRAC_PI_2, PI};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail, ensure};
use cpal::traits::DeviceTrait;
use cpal::{Device, FromSample, I24, Sample, SampleFormat, SizedSample, Stream, StreamConfig, U24};
use rand::prelude::{RngExt, SmallRng};

use crate::settings::{AudioSettings, FREQUENCY_BANDS, SoundStyle, slider_to_db};

const RAIN_WAV_DATA: &[u8] = include_bytes!("../assets/rain_loop.wav");
const WHITE_NOISE_GAIN: f32 = 0.28;
// Matches the white source RMS (0.28 / sqrt(3)) so switching styles keeps a
// comparable signal level.
const COLORED_NOISE_TARGET_RMS: f32 = 0.16;
// RMS of the uniform [-1, 1) white input that drives the colored sources.
const UNIFORM_INPUT_RMS: f64 = 0.577_350_269_189_625_8;
const PINK_LADDER_START_HZ: f64 = 8.0;
const PINK_LADDER_RATIO: f64 = 4.0;
const BROWN_LEAK_HZ: f64 = 8.0;
const RAIN_TARGET_RMS: f32 = 0.12;
const RAIN_PEAK_THRESHOLD: f32 = 0.28;
const RAIN_PEAK_RATIO: f32 = 4.0;
const PARAMETER_RAMP_SECONDS: f32 = 0.05;
const STYLE_CROSSFADE_SECONDS: f32 = 0.20;
const EQ_SMOOTHING_SECONDS: f32 = 0.03;
const EQ_GAIN_SNAP_DB: f32 = 0.01;

// A deliberately gentle convenience curve. Equal-loudness contours depend on
// playback level, so presenting fixed gains as "Fletcher-Munson correction"
// would be misleading.
const LISTENING_CONTOUR_DB: [f32; FREQUENCY_BANDS.len()] =
    [4.0, 2.5, 1.0, 0.0, -0.5, -1.0, 0.0, 1.0];

#[derive(Clone, Copy, Debug)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn peaking(sample_rate: f32, frequency: f32, q: f32, gain_db: f32) -> Self {
        if gain_db.abs() < f32::EPSILON || frequency >= sample_rate * 0.48 {
            return Self::IDENTITY;
        }

        let omega = 2.0 * PI * frequency / sample_rate;
        let (sin_omega, cos_omega) = omega.sin_cos();
        let alpha = sin_omega / (2.0 * q.max(0.1));
        let amplitude = 10.0_f32.powf(gain_db / 40.0);

        let b0 = 1.0 + alpha * amplitude;
        let b1 = -2.0 * cos_omega;
        let b2 = 1.0 - alpha * amplitude;
        let a0 = 1.0 + alpha / amplitude;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha / amplitude;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

#[derive(Debug)]
struct Biquad {
    sample_rate: f32,
    frequency: f32,
    q: f32,
    current_gain_db: f32,
    target_gain_db: f32,
    smoothing: f32,
    coefficients: Coefficients,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(sample_rate: f32, frequency: f32, q: f32, gain_db: f32) -> Self {
        let smoothing = 1.0 - (-1.0 / (EQ_SMOOTHING_SECONDS * sample_rate)).exp();
        Self {
            sample_rate,
            frequency,
            q,
            current_gain_db: gain_db,
            target_gain_db: gain_db,
            smoothing,
            coefficients: Coefficients::peaking(sample_rate, frequency, q, gain_db),
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn set_target_gain(&mut self, gain_db: f32) {
        self.target_gain_db = gain_db;
    }

    fn process(&mut self, input: f32) -> f32 {
        // Smooth in the gain domain and rebuild the coefficients from the
        // smoothed gain. Interpolating raw biquad coefficients is unstable for
        // the near-unit-circle poles of the low bands; every filter produced
        // this way is a genuine peaking filter and therefore stable.
        if self.current_gain_db != self.target_gain_db {
            self.current_gain_db += (self.target_gain_db - self.current_gain_db) * self.smoothing;
            if (self.current_gain_db - self.target_gain_db).abs() < EQ_GAIN_SNAP_DB {
                self.current_gain_db = self.target_gain_db;
            }
            self.coefficients = Coefficients::peaking(
                self.sample_rate,
                self.frequency,
                self.q,
                self.current_gain_db,
            );
        }

        let c = self.coefficients;
        let output =
            c.b0 * input + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;

        if output.is_finite() {
            output
        } else {
            // A non-finite value in the feedback state would poison the band
            // forever; flush it so the filter recovers on the next sample.
            self.x1 = 0.0;
            self.x2 = 0.0;
            self.y1 = 0.0;
            self.y2 = 0.0;
            0.0
        }
    }
}

fn band_gain_db(settings: AudioSettings, index: usize) -> f32 {
    let contour = if settings.listening_contour {
        LISTENING_CONTOUR_DB[index]
    } else {
        0.0
    };
    (slider_to_db(settings.frequency_bands[index]) + contour).clamp(-18.0, 12.0)
}

#[derive(Debug)]
struct GraphicEq {
    filters: [Biquad; FREQUENCY_BANDS.len()],
    last_values: [f32; FREQUENCY_BANDS.len()],
    last_contour: bool,
}

impl GraphicEq {
    fn new(sample_rate: f32, settings: AudioSettings) -> Self {
        Self {
            filters: std::array::from_fn(|index| {
                let band = FREQUENCY_BANDS[index];
                Biquad::new(
                    sample_rate,
                    band.center_frequency(),
                    band.q(),
                    band_gain_db(settings, index),
                )
            }),
            last_values: settings.frequency_bands,
            last_contour: settings.listening_contour,
        }
    }

    fn update(&mut self, settings: AudioSettings) {
        if self.last_values == settings.frequency_bands
            && self.last_contour == settings.listening_contour
        {
            return;
        }

        for (index, filter) in self.filters.iter_mut().enumerate() {
            filter.set_target_gain(band_gain_db(settings, index));
        }

        self.last_values = settings.frequency_bands;
        self.last_contour = settings.listening_contour;
    }

    fn process(&mut self, mut sample: f32) -> f32 {
        for filter in &mut self.filters {
            sample = filter.process(sample);
        }
        sample
    }
}

// One matched-Z first-order stage: H(z) = (1 - zero*z^-1) / (1 - pole*z^-1).
#[derive(Debug, Clone, Copy)]
struct OnePoleZero {
    zero: f32,
    pole: f32,
    x1: f32,
    y1: f32,
}

impl OnePoleZero {
    fn process(&mut self, input: f32) -> f32 {
        let output = input - self.zero * self.x1 + self.pole * self.y1;
        self.x1 = input;
        self.y1 = output;
        output
    }
}

fn stage_power(zero: f64, pole: f64, cos_omega: f64) -> f64 {
    (1.0 - 2.0 * zero * cos_omega + zero * zero) / (1.0 - 2.0 * pole * cos_omega + pole * pole)
}

fn ladder_power(stages: &[(f64, f64)], cos_omega: f64) -> f64 {
    stages
        .iter()
        .map(|&(zero, pole)| stage_power(zero, pole, cos_omega))
        .product()
}

// Mean of |H|^2 over the digital band, i.e. the white-to-output variance gain.
fn ladder_variance_gain(stages: &[(f64, f64)]) -> f64 {
    const STEPS: usize = 16_384;
    (0..STEPS)
        .map(|step| {
            let omega = std::f64::consts::PI * (step as f64 + 0.5) / STEPS as f64;
            ladder_power(stages, omega.cos())
        })
        .sum::<f64>()
        / STEPS as f64
}

/// Pink noise (-3 dB per octave) built for the actual output sample rate: a
/// ladder of matched-Z pole/zero stages spaced two octaves apart approximates
/// the slope, and one correction zero solved at startup flattens the response
/// near Nyquist. The result stays within about 0.25 dB of ideal pink from
/// 20 Hz to 20 kHz at any common sample rate.
#[derive(Debug)]
struct PinkNoise {
    stages: Vec<OnePoleZero>,
    gain: f32,
}

impl PinkNoise {
    fn new(sample_rate: f32, target_rms: f32) -> Self {
        let fs = f64::from(sample_rate);
        let radius = |frequency: f64| (-2.0 * std::f64::consts::PI * frequency / fs).exp();

        let mut stages: Vec<(f64, f64)> = Vec::new();
        let mut pole_hz = PINK_LADDER_START_HZ;
        while pole_hz < fs {
            let zero_hz = pole_hz * PINK_LADDER_RATIO.sqrt();
            stages.push((radius(zero_hz), radius(pole_hz)));
            pole_hz *= PINK_LADDER_RATIO;
        }

        // The raw ladder runs slightly hot approaching Nyquist. Solve one
        // correction zero (1 - a*z^-1, a <= 0) so the deviation from the ideal
        // -3 dB/octave line (anchored at 1 kHz) is zero at the band top.
        let deviation_db = |correction: f64, frequency: f64| {
            let response = |f: f64| {
                let cos_omega = (2.0 * std::f64::consts::PI * f / fs).cos();
                let power =
                    ladder_power(&stages, cos_omega) * stage_power(correction, 0.0, cos_omega);
                10.0 * power.log10() + 10.0 * f.log10()
            };
            response(frequency) - response(1_000.0)
        };
        let solve_at = (0.40 * fs).min(18_000.0);
        let mut low = -0.6_f64;
        let mut high = 0.0_f64;
        for _ in 0..60 {
            let mid = 0.5 * (low + high);
            if deviation_db(mid, solve_at) > 0.0 {
                high = mid;
            } else {
                low = mid;
            }
        }
        stages.push((0.5 * (low + high), 0.0));

        let gain =
            f64::from(target_rms) / (UNIFORM_INPUT_RMS * ladder_variance_gain(&stages).sqrt());

        Self {
            stages: stages
                .into_iter()
                .map(|(zero, pole)| OnePoleZero {
                    zero: zero as f32,
                    pole: pole as f32,
                    x1: 0.0,
                    y1: 0.0,
                })
                .collect(),
            gain: gain as f32,
        }
    }

    fn process(&mut self, white: f32) -> f32 {
        let mut sample = white;
        for stage in &mut self.stages {
            sample = stage.process(sample);
        }
        sample * self.gain
    }
}

/// Brown noise (-6 dB per octave): a leaky integrator with the leak below the
/// audible band. The output gain is exact, from the closed-form variance of a
/// one-pole filter driven by white noise.
#[derive(Debug)]
struct BrownNoise {
    pole: f32,
    gain: f32,
    y1: f32,
}

impl BrownNoise {
    fn new(sample_rate: f32, target_rms: f32) -> Self {
        let fs = f64::from(sample_rate);
        let pole = (-2.0 * std::f64::consts::PI * BROWN_LEAK_HZ / fs).exp();
        let variance_gain = 1.0 / (1.0 - pole * pole);
        let gain = f64::from(target_rms) / (UNIFORM_INPUT_RMS * variance_gain.sqrt());
        Self {
            pole: pole as f32,
            gain: gain as f32,
            y1: 0.0,
        }
    }

    fn process(&mut self, white: f32) -> f32 {
        self.y1 = white + self.pole * self.y1;
        self.y1 * self.gain
    }
}

#[derive(Debug)]
struct LinearRamp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_samples: u32,
}

impl LinearRamp {
    fn new(value: f32, sample_rate: f32, seconds: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
            ramp_samples: (sample_rate * seconds).round().max(1.0) as u32,
        }
    }

    fn set_target(&mut self, target: f32) {
        if (self.target - target).abs() < f32::EPSILON {
            return;
        }
        self.target = target;
        self.remaining = self.ramp_samples;
        self.step = (self.target - self.current) / self.remaining as f32;
    }

    fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.current += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            }
        }
        self.current
    }
}

#[derive(Debug)]
struct RainSamplePlayer {
    samples: Vec<f32>,
    source_sample_rate: u32,
    target_sample_rate: f32,
    position: f64,
    crossfade_samples: usize,
    normalization_gain: f32,
}

impl RainSamplePlayer {
    fn embedded(target_sample_rate: f32) -> Result<Self> {
        Self::from_wav(RAIN_WAV_DATA, target_sample_rate)
            .context("failed to decode the embedded rain recording")
    }

    fn from_wav(data: &[u8], target_sample_rate: f32) -> Result<Self> {
        ensure!(
            target_sample_rate.is_finite() && target_sample_rate > 0.0,
            "invalid target sample rate"
        );

        let reader = hound::WavReader::new(Cursor::new(data))?;
        let spec = reader.spec();
        ensure!(spec.channels > 0, "rain recording has no channels");
        ensure!(
            spec.sample_rate > 0,
            "rain recording has an invalid sample rate"
        );

        let interleaved = decode_wav_samples(reader, spec)?;
        let channels = usize::from(spec.channels);
        ensure!(
            interleaved.len() % channels == 0,
            "rain recording ends with an incomplete audio frame"
        );

        let samples: Vec<f32> = interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
            .collect();
        ensure!(samples.len() >= 4, "rain recording is empty or too short");

        let rms = (samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt() as f32;
        ensure!(rms.is_finite() && rms > 0.0, "rain recording is silent");

        let requested_crossfade = spec.sample_rate as usize * 2;
        let crossfade_samples = requested_crossfade.min(samples.len() / 3).max(1);

        Ok(Self {
            samples,
            source_sample_rate: spec.sample_rate,
            target_sample_rate,
            position: 0.0,
            crossfade_samples,
            normalization_gain: (RAIN_TARGET_RMS / rms).clamp(0.25, 8.0),
        })
    }

    fn interpolated(&self, position: f64) -> f32 {
        let index = position.floor() as usize % self.samples.len();
        let fraction = (position - position.floor()) as f32;
        let first = self.samples[index];
        let second = self.samples[(index + 1) % self.samples.len()];
        first + (second - first) * fraction
    }

    fn next_sample(&mut self) -> f32 {
        let fade_start = self.samples.len() - self.crossfade_samples;
        let sample = if self.position >= fade_start as f64 {
            let fade_position = self.position - fade_start as f64;
            let progress = (fade_position / self.crossfade_samples as f64).clamp(0.0, 1.0) as f32;
            let angle = progress * FRAC_PI_2;
            self.interpolated(self.position) * angle.cos()
                + self.interpolated(fade_position) * angle.sin()
        } else {
            self.interpolated(self.position)
        };

        self.position += self.source_sample_rate as f64 / self.target_sample_rate as f64;
        while self.position >= self.samples.len() as f64 {
            self.position -= fade_start as f64;
        }

        condition_rain_sample(sample * self.normalization_gain)
    }
}

fn decode_wav_samples<R: std::io::Read>(
    reader: hound::WavReader<R>,
    spec: hound::WavSpec,
) -> Result<Vec<f32>> {
    use hound::SampleFormat;

    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => reader
            .into_samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into),
        (SampleFormat::Int, 1..=8) => {
            let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .into_samples::<i8>()
                .map(|sample| sample.map(|value| f32::from(value) / scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        }
        (SampleFormat::Int, 9..=16) => {
            let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .into_samples::<i16>()
                .map(|sample| sample.map(|value| f32::from(value) / scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        }
        (SampleFormat::Int, 17..=32) => {
            let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .into_samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        }
        _ => bail!(
            "unsupported rain WAV encoding: {:?}, {} bits",
            spec.sample_format,
            spec.bits_per_sample
        ),
    }
}

fn condition_rain_sample(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= RAIN_PEAK_THRESHOLD {
        return sample;
    }

    let compressed =
        RAIN_PEAK_THRESHOLD * (magnitude / RAIN_PEAK_THRESHOLD).powf(1.0 / RAIN_PEAK_RATIO);
    sample.signum() * compressed
}

#[derive(Debug)]
struct AudioEngine {
    rng: SmallRng,
    pink: PinkNoise,
    brown: BrownNoise,
    rain_player: RainSamplePlayer,
    eq: GraphicEq,
    volume: LinearRamp,
    // One gain ramp per SoundStyle::ALL entry. All ramps share one duration
    // and retarget together, so the linear gains always sum to 1 and the
    // sqrt-gain mix stays equal-power, even when the style changes mid-fade.
    style_gains: [LinearRamp; SoundStyle::ALL.len()],
}

impl AudioEngine {
    fn new(sample_rate: f32, settings: AudioSettings) -> Result<Self> {
        ensure!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "invalid output sample rate"
        );
        let settings = settings.sanitize();

        let mut volume = LinearRamp::new(0.0, sample_rate, PARAMETER_RAMP_SECONDS);
        volume.set_target(settings.volume);

        Ok(Self {
            rng: rand::make_rng(),
            pink: PinkNoise::new(sample_rate, COLORED_NOISE_TARGET_RMS),
            brown: BrownNoise::new(sample_rate, COLORED_NOISE_TARGET_RMS),
            rain_player: RainSamplePlayer::embedded(sample_rate)?,
            eq: GraphicEq::new(sample_rate, settings),
            volume,
            style_gains: SoundStyle::ALL.map(|style| {
                LinearRamp::new(
                    settings.mix().level(style),
                    sample_rate,
                    STYLE_CROSSFADE_SECONDS,
                )
            }),
        })
    }

    fn update_settings(&mut self, settings: AudioSettings) {
        let settings = settings.sanitize();
        self.eq.update(settings);
        self.volume.set_target(settings.volume);
        for (style, ramp) in SoundStyle::ALL.iter().zip(self.style_gains.iter_mut()) {
            ramp.set_target(settings.mix().level(*style));
        }
    }

    fn next_sample(&mut self) -> f32 {
        let mut mixed = 0.0;
        for (style, ramp) in SoundStyle::ALL.iter().zip(self.style_gains.iter_mut()) {
            let gain = ramp.next().clamp(0.0, 1.0);
            if gain <= 0.0 {
                continue;
            }
            let source = match style {
                SoundStyle::White => (self.rng.random::<f32>() * 2.0 - 1.0) * WHITE_NOISE_GAIN,
                SoundStyle::Pink => self.pink.process(self.rng.random::<f32>() * 2.0 - 1.0),
                SoundStyle::Brown => self.brown.process(self.rng.random::<f32>() * 2.0 - 1.0),
                SoundStyle::Rain => self.rain_player.next_sample(),
            };
            mixed += source * gain.sqrt();
        }

        let shaped = self.eq.process(mixed);
        soft_limit(shaped * self.volume.next())
    }
}

fn soft_limit(sample: f32) -> f32 {
    if !sample.is_finite() {
        return 0.0;
    }

    const KNEE: f32 = 0.8;
    let magnitude = sample.abs();
    if magnitude <= KNEE {
        sample
    } else {
        let limited = KNEE + (1.0 - KNEE) * (1.0 - (-(magnitude - KNEE) / (1.0 - KNEE)).exp());
        sample.signum() * limited.min(1.0)
    }
}

pub fn build_output_stream(
    device: &Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    settings: Arc<Mutex<AudioSettings>>,
    running: Arc<AtomicBool>,
) -> Result<Stream> {
    match sample_format {
        SampleFormat::I8 => build_typed_stream::<i8>(device, config, settings, running),
        SampleFormat::I16 => build_typed_stream::<i16>(device, config, settings, running),
        SampleFormat::I24 => build_typed_stream::<I24>(device, config, settings, running),
        SampleFormat::I32 => build_typed_stream::<i32>(device, config, settings, running),
        SampleFormat::I64 => build_typed_stream::<i64>(device, config, settings, running),
        SampleFormat::U8 => build_typed_stream::<u8>(device, config, settings, running),
        SampleFormat::U16 => build_typed_stream::<u16>(device, config, settings, running),
        SampleFormat::U24 => build_typed_stream::<U24>(device, config, settings, running),
        SampleFormat::U32 => build_typed_stream::<u32>(device, config, settings, running),
        SampleFormat::U64 => build_typed_stream::<u64>(device, config, settings, running),
        SampleFormat::F32 => build_typed_stream::<f32>(device, config, settings, running),
        SampleFormat::F64 => build_typed_stream::<f64>(device, config, settings, running),
        SampleFormat::DsdU8 | SampleFormat::DsdU16 | SampleFormat::DsdU32 => {
            bail!("DSD output formats are not supported")
        }
        _ => bail!("unsupported output sample format: {sample_format}"),
    }
}

fn build_typed_stream<T>(
    device: &Device,
    config: StreamConfig,
    settings: Arc<Mutex<AudioSettings>>,
    running: Arc<AtomicBool>,
) -> Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let initial_settings = settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .sanitize();
    let mut latest_settings = initial_settings;
    let mut engine = AudioEngine::new(config.sample_rate as f32, initial_settings)?;
    let audio_running = Arc::clone(&running);
    let error_running = Arc::clone(&running);

    device
        .build_output_stream::<T, _, _>(
            config,
            move |data, _| {
                if !audio_running.load(Ordering::Relaxed) {
                    data.fill(T::from_sample(0.0));
                    return;
                }

                // Never wait for the UI thread from the real-time callback. If it is
                // updating a setting, use the previous snapshot for this buffer.
                if let Ok(current) = settings.try_lock() {
                    let current = current.sanitize();
                    if current != latest_settings {
                        latest_settings = current;
                        engine.update_settings(current);
                    }
                }

                write_interleaved_frames(data, channels, || engine.next_sample());
            },
            move |error| {
                eprintln!("audio stream error: {error}");
                error_running.store(false, Ordering::Relaxed);
            },
            None,
        )
        .context("failed to open the output audio stream")
}

fn write_interleaved_frames<T, F>(data: &mut [T], channels: usize, mut next_sample: F)
where
    T: Sample + FromSample<f32>,
    F: FnMut() -> f32,
{
    for frame in data.chunks_mut(channels.max(1)) {
        let sample = T::from_sample(next_sample());
        frame.fill(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SourceMix;
    use rand::SeedableRng;

    #[test]
    fn one_generator_sample_is_written_per_audio_frame() {
        let mut output = [0.0_f32; 8];
        let mut next = 0.0;
        write_interleaved_frames(&mut output, 2, || {
            next += 1.0;
            next
        });

        assert_eq!(output, [1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0]);
    }

    #[test]
    fn output_is_converted_to_integer_pcm() {
        let mut signed = [0_i16; 4];
        write_interleaved_frames(&mut signed, 2, || 0.5);
        assert!(signed.iter().all(|sample| *sample > 16_000));
        assert!(signed.windows(2).all(|pair| pair[0] == pair[1]));

        let mut unsigned = [0_u16; 4];
        write_interleaved_frames(&mut unsigned, 2, || 0.0);
        assert_eq!(unsigned, [32_768; 4]);
    }

    #[test]
    fn embedded_rain_has_expected_shape_and_gain_conditioning() {
        let player = RainSamplePlayer::embedded(48_000.0).unwrap();

        assert_eq!(player.source_sample_rate, 44_100);
        assert_eq!(player.samples.len(), 44_100 * 15);
        assert_eq!(player.crossfade_samples, 44_100 * 2);
        assert!(player.normalization_gain > 1.0);
        assert!(player.normalization_gain <= 8.0);
    }

    #[test]
    fn rain_resampling_advances_once_per_target_frame() {
        let mut player = RainSamplePlayer::embedded(48_000.0).unwrap();
        for _ in 0..48_000 {
            player.next_sample();
        }

        assert!((player.position - 44_100.0).abs() < 0.01);
    }

    #[test]
    fn neutral_eq_is_transparent() {
        let settings = AudioSettings::default();
        let mut eq = GraphicEq::new(48_000.0, settings);
        let input = [0.0, 0.25, -0.5, 0.75, -0.1];
        let output = input.map(|sample| eq.process(sample));

        assert_eq!(input, output);
    }

    #[test]
    fn neutral_white_source_has_expected_statistics() {
        let settings = AudioSettings {
            volume: 1.0,
            ..AudioSettings::default()
        };
        let mut engine = AudioEngine::new(48_000.0, settings).unwrap();
        engine.rng = SmallRng::seed_from_u64(42);

        // Let the startup volume ramp finish before measuring the source.
        for _ in 0..3_000 {
            engine.next_sample();
        }

        let count = 200_000;
        let mut sum = 0.0_f64;
        let mut sum_of_squares = 0.0_f64;
        let mut lag_product = 0.0_f64;
        let mut previous = f64::from(engine.next_sample());
        for _ in 0..count {
            let sample = f64::from(engine.next_sample());
            sum += sample;
            sum_of_squares += sample * sample;
            lag_product += sample * previous;
            previous = sample;
        }

        let mean = sum / count as f64;
        let variance = sum_of_squares / count as f64 - mean * mean;
        let rms = (sum_of_squares / count as f64).sqrt();
        let lag_one_correlation = (lag_product / count as f64 - mean * mean) / variance;

        assert!(mean.abs() < 0.003, "white-noise DC offset was {mean}");
        assert!((0.155..0.168).contains(&rms), "white-noise RMS was {rms}");
        assert!(
            lag_one_correlation.abs() < 0.015,
            "white-noise lag-one correlation was {lag_one_correlation}"
        );
    }

    #[test]
    fn conditioned_rain_has_a_usable_ambient_level() {
        let settings = AudioSettings {
            volume: 1.0,
            sound_style: SoundStyle::Rain,
            ..AudioSettings::default()
        };
        let mut engine = AudioEngine::new(48_000.0, settings).unwrap();

        for _ in 0..3_000 {
            engine.next_sample();
        }

        let count = 480_000;
        let sum_of_squares = (0..count)
            .map(|_| f64::from(engine.next_sample()).powi(2))
            .sum::<f64>();
        let rms = (sum_of_squares / count as f64).sqrt();

        assert!(
            (0.08..0.16).contains(&rms),
            "conditioned rain RMS was {rms}"
        );
    }

    #[test]
    fn engine_stays_finite_and_bounded_at_extreme_settings() {
        for style in SoundStyle::ALL {
            let settings = AudioSettings {
                volume: 1.0,
                frequency_bands: [1.0; FREQUENCY_BANDS.len()],
                listening_contour: true,
                sound_style: style,
                ..AudioSettings::default()
            };
            let mut engine = AudioEngine::new(48_000.0, settings).unwrap();

            for _ in 0..100_000 {
                let sample = engine.next_sample();
                assert!(sample.is_finite());
                assert!(sample.abs() <= 1.0);
            }
        }
    }

    fn collect_colored(mut source: impl FnMut(f32) -> f32, count: usize) -> Vec<f32> {
        let mut rng = SmallRng::seed_from_u64(1234);
        (0..count)
            .map(|_| source(rng.random::<f32>() * 2.0 - 1.0))
            .collect()
    }

    // Deterministic magnitude response of the realized (f32) filter: capture
    // its impulse response and evaluate the DFT at each target frequency.
    fn impulse_octave_slopes_db(mut source: impl FnMut(f32) -> f32, sample_rate: f32) -> Vec<f64> {
        let length = 1 << 18;
        let impulse_response: Vec<f32> = (0..length)
            .map(|index| source(if index == 0 { 1.0 } else { 0.0 }))
            .collect();

        let response_db = |frequency: f64| {
            let omega = 2.0 * std::f64::consts::PI * frequency / f64::from(sample_rate);
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for (index, &h) in impulse_response.iter().enumerate() {
                let phase = omega * index as f64;
                re += f64::from(h) * phase.cos();
                im -= f64::from(h) * phase.sin();
            }
            10.0 * (re * re + im * im).log10()
        };

        let frequencies = [125.0_f64, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0];
        let levels: Vec<f64> = frequencies.iter().map(|f| response_db(*f)).collect();
        levels.windows(2).map(|pair| pair[1] - pair[0]).collect()
    }

    #[test]
    fn pink_noise_falls_three_db_per_octave() {
        for sample_rate in [44_100.0_f32, 48_000.0, 192_000.0] {
            let mut pink = PinkNoise::new(sample_rate, COLORED_NOISE_TARGET_RMS);
            let slopes = impulse_octave_slopes_db(|sample| pink.process(sample), sample_rate);
            for (octave, slope) in slopes.iter().enumerate() {
                assert!(
                    (slope - -3.01).abs() < 0.5,
                    "pink octave {octave} slope was {slope:.2} dB at {sample_rate} Hz"
                );
            }
        }
    }

    #[test]
    fn brown_noise_falls_six_db_per_octave() {
        for sample_rate in [44_100.0_f32, 48_000.0, 192_000.0] {
            let mut brown = BrownNoise::new(sample_rate, COLORED_NOISE_TARGET_RMS);
            let slopes = impulse_octave_slopes_db(|sample| brown.process(sample), sample_rate);
            for (octave, slope) in slopes.iter().enumerate() {
                // A digital one-pole flattens slightly approaching Nyquist,
                // so the top octave sits near -5.7 dB at 44.1 kHz.
                assert!(
                    (slope - -6.02).abs() < 0.6,
                    "brown octave {octave} slope was {slope:.2} dB at {sample_rate} Hz"
                );
            }
        }
    }

    #[test]
    fn colored_noise_levels_match_the_white_source() {
        for sample_rate in [44_100.0_f32, 48_000.0, 192_000.0] {
            let mut pink = PinkNoise::new(sample_rate, COLORED_NOISE_TARGET_RMS);
            let mut brown = BrownNoise::new(sample_rate, COLORED_NOISE_TARGET_RMS);
            for (name, samples) in [
                (
                    "pink",
                    collect_colored(|white| pink.process(white), 480_000),
                ),
                (
                    "brown",
                    collect_colored(|white| brown.process(white), 480_000),
                ),
            ] {
                // Skip the leaky integrator's settle-in before measuring.
                let settled = &samples[samples.len() / 4..];
                let rms = (settled.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>()
                    / settled.len() as f64)
                    .sqrt();
                assert!(
                    (0.145..0.175).contains(&rms),
                    "{name} RMS was {rms:.4} at {sample_rate} Hz"
                );
            }
        }
    }

    #[test]
    fn mixed_sources_add_in_power() {
        // White and brown are independent, so a 50/50 power mix must measure
        // close to sqrt(0.5*rms_white^2 + 0.5*rms_brown^2), about 0.16 given
        // both sources are level-matched. A linear-amplitude mixer would read
        // about 3 dB low here.
        let mut settings = AudioSettings {
            volume: 1.0,
            ..AudioSettings::default()
        };
        settings.set_mix(SourceMix {
            white: 0.5,
            pink: 0.0,
            brown: 0.5,
            rain: 0.0,
        });
        let mut engine = AudioEngine::new(48_000.0, settings).unwrap();
        engine.rng = SmallRng::seed_from_u64(11);

        // Let the volume ramp and the brown integrator settle.
        for _ in 0..48_000 {
            engine.next_sample();
        }
        let count = 400_000;
        let sum_of_squares = (0..count)
            .map(|_| f64::from(engine.next_sample()).powi(2))
            .sum::<f64>();
        let rms = (sum_of_squares / f64::from(count)).sqrt();
        assert!((0.145..0.175).contains(&rms), "mixed RMS was {rms}");
    }

    #[test]
    fn full_mix_of_every_source_stays_bounded() {
        let mut settings = AudioSettings {
            volume: 1.0,
            frequency_bands: [1.0; FREQUENCY_BANDS.len()],
            listening_contour: true,
            ..AudioSettings::default()
        };
        settings.set_mix(SourceMix {
            white: 1.0,
            pink: 1.0,
            brown: 1.0,
            rain: 1.0,
        });
        let mut engine = AudioEngine::new(48_000.0, settings).unwrap();

        for _ in 0..100_000 {
            let sample = engine.next_sample();
            assert!(sample.is_finite());
            assert!(sample.abs() <= 1.0);
        }
    }

    #[test]
    fn solo_to_mix_transition_stays_bounded() {
        let mut settings = AudioSettings {
            volume: 1.0,
            ..AudioSettings::default()
        };
        let mut engine = AudioEngine::new(48_000.0, settings).unwrap();
        for _ in 0..10_000 {
            engine.next_sample();
        }

        settings.set_mix(SourceMix {
            white: 0.0,
            pink: 0.3,
            brown: 0.3,
            rain: 0.4,
        });
        engine.update_settings(settings);
        for _ in 0..50_000 {
            let sample = engine.next_sample();
            assert!(sample.is_finite());
            assert!(sample.abs() <= 1.0);
        }
    }

    #[test]
    fn style_switching_stays_bounded_through_partial_crossfades() {
        let mut settings = AudioSettings {
            volume: 1.0,
            ..AudioSettings::default()
        };
        let mut engine = AudioEngine::new(48_000.0, settings).unwrap();

        // Retarget faster than the 200 ms crossfade completes, repeatedly.
        let mut style = settings.sound_style;
        for _ in 0..40 {
            style = style.next();
            settings.sound_style = style;
            engine.update_settings(settings);
            for _ in 0..4_800 {
                let sample = engine.next_sample();
                assert!(sample.is_finite());
                assert!(sample.abs() <= 1.0);
            }
        }
    }

    #[test]
    fn eq_stays_bounded_while_sub_bass_slider_moves() {
        for sample_rate in [44_100.0_f32, 48_000.0, 96_000.0, 192_000.0] {
            let mut settings = AudioSettings::default();
            let mut eq = GraphicEq::new(sample_rate, settings);
            let mut rng = SmallRng::seed_from_u64(7);
            // Roughly one keypress per key-repeat interval, as the UI does.
            let keypress_samples = (sample_rate * 0.033) as usize;

            let mut slider_steps = Vec::new();
            for _ in 0..2 {
                let mut value = 0.5_f32;
                while value < 1.0 {
                    value = (value + 0.05).min(1.0);
                    slider_steps.push(value);
                }
                while value > 0.0 {
                    value = (value - 0.05).max(0.0);
                    slider_steps.push(value);
                }
                while value < 0.5 {
                    value = (value + 0.05).min(1.0);
                    slider_steps.push(value);
                }
            }

            for value in slider_steps {
                settings.frequency_bands[0] = value;
                eq.update(settings);
                for _ in 0..keypress_samples {
                    let input = (rng.random::<f32>() * 2.0 - 1.0) * WHITE_NOISE_GAIN;
                    let sample = eq.process(input);
                    assert!(sample.is_finite());
                    assert!(
                        sample.abs() < 4.0,
                        "EQ transient reached {sample} at {sample_rate} Hz"
                    );
                }
            }
        }
    }

    #[test]
    fn eq_recovers_after_non_finite_input() {
        let settings = AudioSettings {
            frequency_bands: [1.0; FREQUENCY_BANDS.len()],
            ..AudioSettings::default()
        };
        let mut eq = GraphicEq::new(48_000.0, settings);
        for _ in 0..1_000 {
            eq.process(0.1);
        }

        eq.process(f32::NAN);
        eq.process(f32::INFINITY);

        for _ in 0..1_000 {
            assert!(eq.process(0.1).is_finite());
        }
    }

    #[test]
    fn soft_limiter_is_continuous_and_bounded() {
        assert_eq!(soft_limit(0.8), 0.8);
        assert!(soft_limit(0.800_001) >= 0.8);
        assert!(soft_limit(100.0) <= 1.0);
        assert!(soft_limit(-100.0) >= -1.0);
        assert_eq!(soft_limit(f32::NAN), 0.0);
    }
}
