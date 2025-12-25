use anyhow::Result;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Stream, StreamConfig};
use crossterm::{
    cursor, execute, queue,
    event::{self, Event, KeyCode, KeyModifiers},
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use dirs;
use rand::prelude::*;
use rand::rngs::SmallRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "whitenoise")]
#[command(about = "Interactive white noise generator with frequency band control")]
struct Args {
    #[arg(short, long)]
    list_devices: bool,

    #[arg(short, long)]
    device: Option<String>,

    #[arg(long)]
    non_interactive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum SoundStyle {
    Vanilla,
    Rain,
}

impl SoundStyle {
    fn name(&self) -> &'static str {
        match self {
            SoundStyle::Vanilla => "Vanilla",
            SoundStyle::Rain => "Rain",
        }
    }
    
    fn next(&self) -> Self {
        match self {
            SoundStyle::Vanilla => SoundStyle::Rain,
            SoundStyle::Rain => SoundStyle::Vanilla,
        }
    }
}

impl Default for SoundStyle {
    fn default() -> Self {
        SoundStyle::Vanilla
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AudioSettings {
    volume: f32,
    frequency_bands: [f32; 8], // 8 frequency bands
    perceptual_normalization: bool, // Fletcher-Munson compensation
    sound_style: SoundStyle,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            volume: 0.0, // Always start at 0% volume for safety
            frequency_bands: [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5], // Balanced
            perceptual_normalization: false, // Start with technical mode
            sound_style: SoundStyle::default(),
        }
    }
}

struct FrequencyBand {
    name: &'static str,
    min_freq: f32,
    max_freq: f32,
}

const FREQUENCY_BANDS: [FrequencyBand; 8] = [
    FrequencyBand { name: "Sub Bass", min_freq: 20.0, max_freq: 60.0 },
    FrequencyBand { name: "Bass", min_freq: 60.0, max_freq: 250.0 },
    FrequencyBand { name: "Low Mid", min_freq: 250.0, max_freq: 500.0 },
    FrequencyBand { name: "Mid", min_freq: 500.0, max_freq: 2000.0 },
    FrequencyBand { name: "Hi Mid", min_freq: 2000.0, max_freq: 4000.0 },
    FrequencyBand { name: "Presence", min_freq: 4000.0, max_freq: 6000.0 },
    FrequencyBand { name: "Brilliance", min_freq: 6000.0, max_freq: 12000.0 },
    FrequencyBand { name: "Air", min_freq: 12000.0, max_freq: 20000.0 },
];

// Simple but effective biquad filter implementation
#[derive(Clone)]
struct BiquadFilter {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}

impl BiquadFilter {
    fn bandpass(frequency: f32, q: f32, sample_rate: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * frequency / sample_rate;
        let cos_w = w.cos();
        let sin_w = w.sin();
        let alpha = sin_w / (2.0 * q);
        
        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }
    
    fn lowpass(frequency: f32, sample_rate: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * frequency / sample_rate;
        let cos_w = w.cos();
        let sin_w = w.sin();
        let alpha = sin_w / 2.0;
        
        let b0 = (1.0 - cos_w) / 2.0;
        let b1 = 1.0 - cos_w;
        let b2 = (1.0 - cos_w) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }
    
    fn highpass(frequency: f32, sample_rate: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * frequency / sample_rate;
        let cos_w = w.cos();
        let sin_w = w.sin();
        let alpha = sin_w / 2.0;
        
        let b0 = (1.0 + cos_w) / 2.0;
        let b1 = -(1.0 + cos_w);
        let b2 = (1.0 + cos_w) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha;
        
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }
    
    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
                   - self.a1 * self.y1 - self.a2 * self.y2;
        
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        
        output
    }
}


// Embedded rain sample (CC0 licensed from BigSoundBank)
// 15-second loop of rain on puddles, 44.1kHz 16-bit mono
static RAIN_WAV_DATA: &[u8] = include_bytes!("../assets/rain_loop.wav");

struct RainSamplePlayer {
    samples: Vec<f32>,
    source_sample_rate: u32,
    target_sample_rate: f32,
    resample_position: f64,
    crossfade_samples: usize, // Number of samples to crossfade
}

impl RainSamplePlayer {
    fn new(target_sample_rate: f32) -> Self {
        // Decode the embedded WAV file
        let cursor = std::io::Cursor::new(RAIN_WAV_DATA);
        let reader = hound::WavReader::new(cursor).expect("Failed to read embedded rain sample");
        let spec = reader.spec();

        // Convert samples to f32 normalized to -1.0 to 1.0
        let samples: Vec<f32> = if spec.bits_per_sample == 16 {
            reader
                .into_samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 32768.0)
                .collect()
        } else if spec.bits_per_sample == 24 {
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 8388608.0)
                .collect()
        } else {
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 2147483648.0)
                .collect()
        };

        // Crossfade duration: ~2 seconds for smooth blending
        let crossfade_samples = (spec.sample_rate as usize) * 2;

        Self {
            samples,
            source_sample_rate: spec.sample_rate,
            target_sample_rate,
            resample_position: 0.0,
            crossfade_samples,
        }
    }

    fn get_sample_interpolated(&self, pos: f64) -> f32 {
        let len = self.samples.len();
        if len == 0 {
            return 0.0;
        }

        let idx = pos as usize % len;
        let frac = pos - pos.floor();

        let sample1 = self.samples[idx];
        let sample2 = self.samples[(idx + 1) % len];

        sample1 + (sample2 - sample1) * frac as f32
    }

    fn generate_sample(&mut self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }

        let len = self.samples.len();
        let fade_start = len - self.crossfade_samples;
        let current_idx = self.resample_position as usize;

        let sample = if current_idx >= fade_start {
            // We're in the crossfade zone - blend end with beginning
            let fade_progress = (current_idx - fade_start) as f32 / self.crossfade_samples as f32;

            // Smooth S-curve crossfade (sounds more natural than linear)
            let fade_out = (std::f32::consts::PI * fade_progress / 2.0).cos();
            let fade_in = (std::f32::consts::PI * fade_progress / 2.0).sin();

            // Sample from current position (fading out)
            let end_sample = self.get_sample_interpolated(self.resample_position);

            // Sample from beginning (fading in) - offset by same amount into the file
            let begin_pos = (current_idx - fade_start) as f64
                + (self.resample_position - current_idx as f64);
            let begin_sample = self.get_sample_interpolated(begin_pos);

            end_sample * fade_out + begin_sample * fade_in
        } else {
            // Normal playback
            self.get_sample_interpolated(self.resample_position)
        };

        // Advance position with resampling ratio
        let ratio = self.source_sample_rate as f64 / self.target_sample_rate as f64;
        self.resample_position += ratio;

        // Loop back when we've finished the crossfade
        if self.resample_position >= len as f64 {
            // Jump to where the crossfade began blending in from
            self.resample_position = self.resample_position - fade_start as f64;
        }

        sample
    }
}

struct FrequencyBandGenerator {
    rng: SmallRng,
    filter: BiquadFilter,
}

impl FrequencyBandGenerator {
    fn new(band: &FrequencyBand, sample_rate: f32) -> Self {
        let filter = if band.min_freq <= 60.0 {
            // Low pass for sub bass
            BiquadFilter::lowpass(band.max_freq, sample_rate)
        } else if band.max_freq >= 16000.0 {
            // High pass for air frequencies
            BiquadFilter::highpass(band.min_freq, sample_rate)
        } else {
            // Bandpass for everything else
            let center_freq = (band.min_freq + band.max_freq) / 2.0;
            let q = 1.5; // Moderate Q for good separation without ringing
            BiquadFilter::bandpass(center_freq, q, sample_rate)
        };

        Self {
            rng: SmallRng::from_entropy(),
            filter,
        }
    }

    fn generate_sample(&mut self, gain: f32, center_freq: f32, perceptual_normalization: bool) -> f32 {
        // Only used for Vanilla white noise mode
        if gain <= 0.001 {
            return 0.0;
        }

        let base_audio = (self.rng.r#gen::<f32>() - 0.5) * 2.0;

        // Apply filter and gain
        let filtered = self.filter.process(base_audio);

        // Apply Fletcher-Munson compensation if enabled
        let perceptual_gain = if perceptual_normalization {
            match center_freq {
                f if f < 100.0 => 2.8,    // Boost sub bass significantly (we barely hear this)
                f if f < 500.0 => 2.0,    // Boost bass (still not very sensitive)
                f if f < 1000.0 => 1.3,   // Slight boost low mid
                f if f < 4000.0 => 1.0,   // Mid frequencies (reference - most sensitive)
                f if f < 6000.0 => 0.8,   // Slight cut (peak sensitivity)
                f if f < 10000.0 => 1.4,  // Boost presence
                _ => 2.2,                 // Boost air frequencies (we don't hear well)
            }
        } else {
            1.0 // Technical mode - no compensation
        };

        // Apply gain with proper scaling
        filtered * gain * perceptual_gain * 0.8 // Scale down to prevent clipping when bands are summed
    }
}

struct NoiseGenerator {
    bands: Vec<FrequencyBandGenerator>,
    center_frequencies: Vec<f32>,
    settings: Arc<Mutex<AudioSettings>>,
    rain_player: RainSamplePlayer,
    rain_filters: Vec<BiquadFilter>, // EQ filters for rain sample
}

impl NoiseGenerator {
    fn new(settings: Arc<Mutex<AudioSettings>>, sample_rate: f32) -> Self {
        let bands = FREQUENCY_BANDS
            .iter()
            .map(|band| FrequencyBandGenerator::new(band, sample_rate))
            .collect();

        let center_frequencies: Vec<f32> = FREQUENCY_BANDS
            .iter()
            .map(|band| (band.min_freq + band.max_freq) / 2.0)
            .collect();

        // Create bandpass filters for rain EQ (same frequencies as white noise bands)
        let rain_filters = FREQUENCY_BANDS
            .iter()
            .map(|band| {
                if band.min_freq <= 60.0 {
                    BiquadFilter::lowpass(band.max_freq, sample_rate)
                } else if band.max_freq >= 16000.0 {
                    BiquadFilter::highpass(band.min_freq, sample_rate)
                } else {
                    let center = (band.min_freq + band.max_freq) / 2.0;
                    BiquadFilter::bandpass(center, 1.5, sample_rate)
                }
            })
            .collect();

        Self {
            bands,
            center_frequencies,
            settings,
            rain_player: RainSamplePlayer::new(sample_rate),
            rain_filters,
        }
    }

    fn generate_sample(&mut self) -> f32 {
        let settings = self.settings.lock().unwrap();
        if settings.volume == 0.0 {
            return 0.0;
        }

        let sound_style = settings.sound_style;
        let perceptual = settings.perceptual_normalization;
        let volume = settings.volume;
        let frequency_bands = settings.frequency_bands;
        drop(settings); // Release lock before generating audio

        let sample = match sound_style {
            SoundStyle::Rain => {
                // Get the rain sample
                let rain_sample = self.rain_player.generate_sample();

                // Apply EQ bands to the rain - split into bands, apply gains, recombine
                let mut sum = 0.0;
                for (i, filter) in self.rain_filters.iter_mut().enumerate() {
                    let gain = frequency_bands[i];
                    if gain <= 0.001 {
                        // Still need to run filter to keep state updated, but don't add to output
                        let _ = filter.process(rain_sample);
                        continue;
                    }

                    let filtered = filter.process(rain_sample);

                    // Apply Fletcher-Munson compensation if enabled
                    let center_freq = self.center_frequencies[i];
                    let perceptual_gain = if perceptual {
                        match center_freq {
                            f if f < 100.0 => 2.8,
                            f if f < 500.0 => 2.0,
                            f if f < 1000.0 => 1.3,
                            f if f < 4000.0 => 1.0,
                            f if f < 6000.0 => 0.8,
                            f if f < 10000.0 => 1.4,
                            _ => 2.2,
                        }
                    } else {
                        1.0
                    };

                    sum += filtered * gain * perceptual_gain * 0.8;
                }
                sum
            }
            SoundStyle::Vanilla => {
                // Sum all frequency bands for white noise
                let mut sum = 0.0;
                for (i, band) in self.bands.iter_mut().enumerate() {
                    let gain = frequency_bands[i];
                    let center_freq = self.center_frequencies[i];
                    sum += band.generate_sample(gain, center_freq, perceptual);
                }
                sum
            }
        };

        // Apply master volume and soft limiting
        let final_sample = sample * volume;

        // Soft clipping to prevent harsh clipping
        if final_sample > 0.95 {
            0.95 + 0.05 * (final_sample - 0.95).tanh()
        } else if final_sample < -0.95 {
            -0.95 + 0.05 * (final_sample + 0.95).tanh()
        } else {
            final_sample
        }
    }
}

struct InteractiveUI {
    settings: Arc<Mutex<AudioSettings>>,
    current_slider: usize,
    running: Arc<AtomicBool>,
}

impl InteractiveUI {
    fn new(settings: Arc<Mutex<AudioSettings>>, running: Arc<AtomicBool>) -> Self {
        Self {
            settings,
            current_slider: 0, // Start with volume slider
            running,
        }
    }

    fn draw_slider(&self, name: &str, value: f32, y: u16, is_selected: bool) -> Result<()> {
        let mut stdout = io::stdout();
        
        queue!(stdout, cursor::MoveTo(2, y))?;
        
        if is_selected {
            queue!(stdout, SetForegroundColor(Color::Yellow))?;
            queue!(stdout, Print(format!("► {:<12}", name)))?;
        } else {
            queue!(stdout, SetForegroundColor(Color::White))?;
            queue!(stdout, Print(format!("  {:<12}", name)))?;
        }
        
        // Draw slider bar
        let bar_width = 30;
        let filled_width = (value * bar_width as f32) as usize;
        
        queue!(stdout, Print(" ["))?;
        queue!(stdout, SetForegroundColor(Color::Green))?;
        for _ in 0..filled_width {
            queue!(stdout, Print("█"))?;
        }
        queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
        for _ in filled_width..bar_width {
            queue!(stdout, Print("░"))?;
        }
        queue!(stdout, SetForegroundColor(Color::White))?;
        queue!(stdout, Print(format!("] {:.1}%", value * 100.0)))?;
        
        queue!(stdout, ResetColor)?;
        Ok(())
    }

    fn draw_ui(&self) -> Result<()> {
        let mut stdout = io::stdout();
        execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        
        // Header
        queue!(stdout, SetForegroundColor(Color::Cyan))?;
        queue!(stdout, Print("🎵 Interactive White Noise Generator\n\r"))?;
        queue!(stdout, ResetColor)?;
        
        let settings = self.settings.lock().unwrap();
        
        // Show current sound style
        queue!(stdout, SetForegroundColor(Color::Magenta))?;
        match settings.sound_style {
            SoundStyle::Vanilla => {
                queue!(stdout, Print("Sound Style: Vanilla (Adjustable) - Press S to switch\n\r"))?;
            }
            SoundStyle::Rain => {
                queue!(stdout, Print("Sound Style: Rain (Fixed Preset) - Press S to switch\n\r"))?;
            }
        }
        
        // Show normalization status
        if settings.perceptual_normalization {
            queue!(stdout, SetForegroundColor(Color::Green))?;
            queue!(stdout, Print("Mode: PERCEPTUAL (Fletcher-Munson) - Press N to toggle\n\r"))?;
        } else {
            queue!(stdout, SetForegroundColor(Color::Yellow))?;
            queue!(stdout, Print("Mode: TECHNICAL (Flat response) - Press N to toggle\n\r"))?;
        }
        queue!(stdout, ResetColor)?;
        queue!(stdout, Print("Controls: ↑/↓ select, ←/→ adjust, S style, N mode, Q to quit\n\r\n\r"))?;
        
        // Volume slider
        self.draw_slider("Volume", settings.volume, 4, self.current_slider == 0)?;
        
        // Frequency band sliders
        for (i, band) in FREQUENCY_BANDS.iter().enumerate() {
            self.draw_slider(
                band.name,
                settings.frequency_bands[i],
                5 + i as u16 + 1,
                self.current_slider == i + 1,
            )?;
        }
        
        // Instructions
        queue!(stdout, cursor::MoveTo(2, 15))?;
        queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
        queue!(stdout, Print("Frequency Ranges:"))?;
        queue!(stdout, cursor::MoveTo(2, 16))?;
        for (i, band) in FREQUENCY_BANDS.iter().enumerate() {
            if i % 4 == 0 && i > 0 {
                queue!(stdout, cursor::MoveTo(2, 16 + (i / 4) as u16))?;
            }
            queue!(stdout, Print(format!("{}: {:.0}-{:.0}Hz  ", band.name, band.min_freq, band.max_freq)))?;
        }
        
        queue!(stdout, ResetColor)?;
        stdout.flush()?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyCode) -> Result<bool> {
        match key {
            KeyCode::Up => {
                if self.current_slider > 0 {
                    self.current_slider -= 1;
                }
            }
            KeyCode::Down => {
                if self.current_slider < 8 { // 0 = volume + 8 frequency bands - 1
                    self.current_slider += 1;
                }
            }
            KeyCode::Left => {
                let mut settings = self.settings.lock().unwrap();
                if self.current_slider == 0 {
                    // Volume
                    settings.volume = (settings.volume - 0.05).max(0.0);
                } else {
                    // Frequency band
                    let band_index = self.current_slider - 1;
                    settings.frequency_bands[band_index] = 
                        (settings.frequency_bands[band_index] - 0.05).max(0.0);
                }
            }
            KeyCode::Right => {
                let mut settings = self.settings.lock().unwrap();
                if self.current_slider == 0 {
                    // Volume
                    settings.volume = (settings.volume + 0.05).min(1.0);
                } else {
                    // Frequency band
                    let band_index = self.current_slider - 1;
                    settings.frequency_bands[band_index] = 
                        (settings.frequency_bands[band_index] + 0.05).min(1.0);
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let mut settings = self.settings.lock().unwrap();
                settings.perceptual_normalization = !settings.perceptual_normalization;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let mut settings = self.settings.lock().unwrap();
                settings.sound_style = settings.sound_style.next();
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                return Ok(true); // Exit
            }
            _ => {}
        }
        Ok(false)
    }

    fn run(&mut self) -> Result<()> {
        execute!(io::stdout(), EnterAlternateScreen)?;
        terminal::enable_raw_mode()?;
        
        let result = self.run_loop();
        
        // Cleanup
        terminal::disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        
        result
    }

    fn run_loop(&mut self) -> Result<()> {
        loop {
            self.draw_ui()?;
            
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key_event) = event::read()? {
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) && key_event.code == KeyCode::Char('c') {
                        break;
                    }
                    if self.handle_key(key_event.code)? {
                        break;
                    }
                }
            }
            
            if !self.running.load(Ordering::Relaxed) {
                break;
            }
        }
        Ok(())
    }
}

fn get_config_path() -> PathBuf {
    let mut config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    config_dir.push("whitenoise");
    if !config_dir.exists() {
        let _ = fs::create_dir_all(&config_dir);
    }
    config_dir.push("settings.toml");
    config_dir
}

fn load_settings() -> AudioSettings {
    let config_path = get_config_path();
    if let Ok(content) = fs::read_to_string(config_path) {
        if let Ok(mut settings) = toml::from_str::<AudioSettings>(&content) {
            settings.volume = 0.0; // Always start at 0% volume for safety
            return settings;
        }
    }
    AudioSettings::default()
}

fn save_settings(settings: &AudioSettings) -> Result<()> {
    let config_path = get_config_path();
    let content = toml::to_string_pretty(settings)?;
    fs::write(config_path, content)?;
    Ok(())
}

fn list_audio_devices(host: &Host) -> Result<()> {
    println!("Available audio devices:");
    
    let default_output = host.default_output_device();
    if let Some(device) = &default_output {
        println!("  * {} (default)", device.name()?);
    }

    for device in host.output_devices()? {
        let name = device.name()?;
        let is_default = match &default_output {
            Some(default) => default.name().unwrap_or_default() == name,
            None => false,
        };
        if !is_default {
            println!("    {}", name);
        }
    }
    Ok(())
}

fn find_device_by_name(host: &Host, device_name: &str) -> Result<Device> {
    for device in host.output_devices()? {
        if device.name()?.to_lowercase().contains(&device_name.to_lowercase()) {
            return Ok(device);
        }
    }
    anyhow::bail!("Device '{}' not found", device_name);
}

fn create_audio_stream(
    device: &Device,
    config: &StreamConfig,
    settings: Arc<Mutex<AudioSettings>>,
    running: Arc<AtomicBool>,
) -> Result<Stream> {
    let generator = Arc::new(Mutex::new(NoiseGenerator::new(settings, config.sample_rate.0 as f32)));

    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if !running.load(Ordering::Relaxed) {
                for sample in data.iter_mut() {
                    *sample = 0.0;
                }
                return;
            }

            if let Ok(mut generator_guard) = generator.lock() {
                for sample in data.iter_mut() {
                    *sample = generator_guard.generate_sample();
                }
            }
        },
        move |err| {
            eprintln!("Audio stream error: {}", err);
        },
        None,
    )?;

    Ok(stream)
}

fn main() -> Result<()> {
    let args = Args::parse();

    let host = cpal::default_host();

    if args.list_devices {
        return list_audio_devices(&host);
    }

    let device = if let Some(device_name) = &args.device {
        find_device_by_name(&host, device_name)?
    } else {
        host.default_output_device()
            .ok_or_else(|| anyhow::anyhow!("No default output device available"))?
    };

    println!("Using device: {}", device.name()?);

    let config = device.default_output_config()?.into();
    
    // Load settings (volume will be 0.0 for safety)
    let settings = Arc::new(Mutex::new(load_settings()));
    
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    ctrlc::set_handler(move || {
        running_clone.store(false, Ordering::Relaxed);
    })?;

    let stream = create_audio_stream(&device, &config, settings.clone(), running.clone())?;
    stream.play()?;

    if args.non_interactive {
        println!("Playing white noise in non-interactive mode... Press Ctrl+C to stop");
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    } else {
        let mut ui = InteractiveUI::new(settings.clone(), running.clone());
        ui.run()?;
    }

    // Save settings before exit
    let final_settings = settings.lock().unwrap().clone();
    let _ = save_settings(&final_settings);

    println!("Settings saved. Goodbye!");
    Ok(())
}