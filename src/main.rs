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


struct RainGenerator {
    rng: SmallRng,
    // Multiple filtered noise sources for realistic rain layers
    low_filter_state: f32,
    mid_filter_state: f32,
    high_filter_state: f32,
    droplet_timer: f32,
    droplet_intensity: f32,
    intensity_mod: f32,
    mod_timer: f32,
}

impl RainGenerator {
    fn new() -> Self {
        Self {
            rng: SmallRng::from_entropy(),
            low_filter_state: 0.0,
            mid_filter_state: 0.0,
            high_filter_state: 0.0,
            droplet_timer: 0.0,
            droplet_intensity: 0.0,
            intensity_mod: 1.0,
            mod_timer: 0.0,
        }
    }
    
    fn generate_sample(&mut self, sample_rate: f32) -> f32 {
        // Layer 1: Low frequency rumble (heavy rain on surfaces)
        let low_noise = (self.rng.r#gen::<f32>() - 0.5) * 0.4;
        self.low_filter_state = 0.98 * self.low_filter_state + 0.02 * low_noise;
        let low_layer = self.low_filter_state * 0.3;
        
        // Layer 2: Mid frequency body (main rain sound)
        let mid_noise = (self.rng.r#gen::<f32>() - 0.5) * 0.6;
        self.mid_filter_state = 0.85 * self.mid_filter_state + 0.15 * mid_noise;
        let mid_layer = self.mid_filter_state * 0.8;
        
        // Layer 3: High frequency texture (droplets and splashes)
        let high_noise = (self.rng.r#gen::<f32>() - 0.5) * 0.3;
        self.high_filter_state = 0.6 * self.high_filter_state + 0.4 * high_noise;
        let high_layer = self.high_filter_state * 0.4;
        
        // Layer 4: Individual droplet transients
        if self.droplet_timer <= 0.0 {
            self.droplet_intensity = 0.3 + self.rng.r#gen::<f32>() * 0.4;
            self.droplet_timer = (20.0 + self.rng.r#gen::<f32>() * 60.0) * sample_rate / 44100.0;
        } else {
            self.droplet_timer -= 1.0;
        }
        self.droplet_intensity *= 0.996;
        
        let droplet_transient = if self.droplet_intensity > 0.02 {
            (self.rng.r#gen::<f32>() - 0.5) * self.droplet_intensity * 0.3
        } else {
            0.0
        };
        
        // Layer 5: Slow intensity modulation (natural variation)
        if self.mod_timer <= 0.0 {
            self.intensity_mod = 0.7 + self.rng.r#gen::<f32>() * 0.3;
            self.mod_timer = (200.0 + self.rng.r#gen::<f32>() * 400.0) * sample_rate / 44100.0;
        } else {
            self.mod_timer -= 1.0;
        }
        
        // Combine all layers with intensity modulation
        let combined = (low_layer + mid_layer + high_layer + droplet_transient) * self.intensity_mod;
        
        // Final limiting to prevent clipping
        combined.max(-0.6).min(0.6)
    }
}

struct FrequencyBandGenerator {
    rng: SmallRng,
    filter: BiquadFilter,
    rain_generator: RainGenerator,
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
            rain_generator: RainGenerator::new(),
        }
    }
    
    fn generate_sample(&mut self, gain: f32, center_freq: f32, perceptual_normalization: bool, sound_style: SoundStyle, sample_rate: f32) -> f32 {        
        // Generate base audio based on style
        let base_audio = match sound_style {
            SoundStyle::Vanilla => {
                // Original clean white noise (frequency bands work normally)
                if gain <= 0.001 {
                    return 0.0;
                }
                (self.rng.r#gen::<f32>() - 0.5) * 2.0
            }
            SoundStyle::Rain => {
                // Rain preset - NOT affected by frequency bands
                // Generate once per band but only use center frequency band
                if center_freq >= 500.0 && center_freq < 2000.0 {
                    // Only the "Mid" band generates rain sound
                    self.rain_generator.generate_sample(sample_rate)
                } else {
                    return 0.0; // Other bands are silent for rain
                }
            }
        };
        
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
}

impl NoiseGenerator {
    fn new(settings: Arc<Mutex<AudioSettings>>, sample_rate: f32) -> Self {
        let bands = FREQUENCY_BANDS
            .iter()
            .map(|band| FrequencyBandGenerator::new(band, sample_rate))
            .collect();
        
        let center_frequencies = FREQUENCY_BANDS
            .iter()
            .map(|band| (band.min_freq + band.max_freq) / 2.0)
            .collect();
        
        Self { bands, center_frequencies, settings }
    }

    fn generate_sample(&mut self, sample_rate: f32) -> f32 {
        let settings = self.settings.lock().unwrap();
        if settings.volume == 0.0 {
            return 0.0;
        }

        let mut sample = 0.0;
        
        // Sum all frequency bands
        for (i, band) in self.bands.iter_mut().enumerate() {
            let gain = settings.frequency_bands[i];
            let center_freq = self.center_frequencies[i];
            sample += band.generate_sample(gain, center_freq, settings.perceptual_normalization, settings.sound_style, sample_rate);
        }
        
        // Apply master volume and soft limiting
        let final_sample = sample * settings.volume;
        
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
    let sample_rate = config.sample_rate.0 as f32;
    
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
                    *sample = generator_guard.generate_sample(sample_rate);
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