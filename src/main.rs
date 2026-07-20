#![forbid(unsafe_code)]

mod audio;
mod device;
mod settings;
mod ui;

use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cpal::traits::{DeviceTrait, StreamTrait};

use crate::audio::build_output_stream;
use crate::device::{list_audio_devices, list_hosts, select_host, select_output_device};
use crate::settings::{AudioSettings, SoundStyle, SourceMix, load_settings, save_settings};
use crate::ui::InteractiveUi;

#[derive(Debug, Parser)]
#[command(name = "whitenoise", version)]
#[command(about = "Interactive white/pink/brown noise and rain ambience generator")]
struct Args {
    /// List audio backends compiled into this build
    #[arg(long)]
    list_hosts: bool,

    /// List audio devices visible to the selected host
    #[arg(short, long)]
    list_devices: bool,

    /// Audio backend to use (for example: alsa or pulseaudio)
    #[arg(long, value_name = "HOST")]
    host: Option<String>,

    /// Output device name (an unambiguous substring is accepted)
    #[arg(short, long, value_name = "DEVICE")]
    device: Option<String>,

    /// Run without the terminal interface, using saved settings
    #[arg(long)]
    non_interactive: bool,

    /// Initial master volume as a percentage from 0 to 100
    #[arg(short, long, value_name = "PERCENT", value_parser = parse_percentage)]
    volume: Option<f32>,

    /// Initial sound source
    #[arg(short, long, value_enum, conflicts_with = "mix")]
    style: Option<SoundStyle>,

    /// Play several sources at once, as SOURCE=PERCENT pairs
    /// (example: --mix rain=60,brown=40)
    #[arg(short, long, value_name = "MIX", value_parser = parse_mix)]
    mix: Option<SourceMix>,
}

fn parse_percentage(value: &str) -> std::result::Result<f32, String> {
    let percent = value
        .parse::<f32>()
        .map_err(|_| "volume must be a number from 0 to 100".to_owned())?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return Err("volume must be a number from 0 to 100".to_owned());
    }
    Ok(percent / 100.0)
}

fn parse_mix(value: &str) -> std::result::Result<SourceMix, String> {
    let mut mix = SourceMix {
        white: 0.0,
        pink: 0.0,
        brown: 0.0,
        rain: 0.0,
    };
    let mut seen: Vec<SoundStyle> = Vec::new();

    for entry in value.split(',') {
        let entry = entry.trim();
        let Some((name, level)) = entry.split_once('=') else {
            return Err(format!(
                "'{entry}' is not SOURCE=PERCENT (example: rain=60,brown=40)"
            ));
        };
        let style = match name.trim().to_lowercase().as_str() {
            "white" | "vanilla" => SoundStyle::White,
            "pink" => SoundStyle::Pink,
            "brown" => SoundStyle::Brown,
            "rain" => SoundStyle::Rain,
            other => {
                return Err(format!(
                    "unknown source '{other}' (valid: white, pink, brown, rain)"
                ));
            }
        };
        if seen.contains(&style) {
            return Err(format!("source '{}' is listed twice", name.trim()));
        }
        seen.push(style);
        let percent = level
            .trim()
            .parse::<f32>()
            .map_err(|_| format!("'{}' is not a percentage from 0 to 100", level.trim()))?;
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err(format!(
                "'{}' is not a percentage from 0 to 100",
                level.trim()
            ));
        }
        mix.set_level(style, percent / 100.0);
    }

    if mix.total() <= 0.0 {
        return Err("the mix is silent; give at least one source a level above 0".to_owned());
    }
    Ok(mix)
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_hosts {
        list_hosts();
        return Ok(());
    }

    let host = select_host(args.host.as_deref())?;
    if args.list_devices {
        return list_audio_devices(&host);
    }

    if !args.non_interactive && (!io::stdin().is_terminal() || !io::stdout().is_terminal()) {
        bail!("interactive mode requires a terminal; use --non-interactive");
    }

    let device = select_output_device(&host, args.device.as_deref())?;
    let device_name = device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| device.to_string());
    let supported_config = device
        .default_output_config()
        .context("failed to query the default output format")?;
    let sample_format = supported_config.sample_format();
    let stream_config = supported_config.config();

    let mut initial_settings = load_settings().unwrap_or_else(|error| {
        eprintln!("warning: {error:#}; using default settings");
        AudioSettings::default()
    });
    if let Some(mix) = args.mix {
        initial_settings.set_mix(mix);
    } else if let Some(style) = args.style {
        initial_settings.set_mix(SourceMix::solo(style));
    }
    if let Some(volume) = args.volume {
        initial_settings.volume = volume;
    } else if !args.non_interactive {
        // Starting an interactive session muted avoids headphone surprises.
        initial_settings.volume = 0.0;
    }
    if args.non_interactive && initial_settings.volume <= 0.0 {
        bail!(
            "non-interactive mode has no audible volume; pass --volume PERCENT or save a non-zero volume in interactive mode"
        );
    }
    if args.non_interactive && initial_settings.mix().total() <= 0.0 {
        bail!(
            "non-interactive mode has no audible source; every mix level is zero, pass --mix or --style"
        );
    }

    println!(
        "Using {} via {} ({} channels, {} Hz, {})",
        device_name,
        host.id(),
        stream_config.channels,
        stream_config.sample_rate,
        sample_format
    );

    let settings = Arc::new(Mutex::new(initial_settings));
    let running = Arc::new(AtomicBool::new(true));
    let signal_running = Arc::clone(&running);
    ctrlc::set_handler(move || signal_running.store(false, Ordering::Relaxed))?;

    let stream = build_output_stream(
        &device,
        stream_config,
        sample_format,
        Arc::clone(&settings),
        Arc::clone(&running),
    )?;
    stream.play().context("failed to start audio playback")?;

    if args.non_interactive {
        println!(
            "Playing {} at {:.0}% volume. Press Ctrl+C to stop.",
            initial_settings.mix().describe(),
            initial_settings.volume * 100.0
        );
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    } else {
        InteractiveUi::new(Arc::clone(&settings), Arc::clone(&running)).run()?;
    }

    running.store(false, Ordering::Relaxed);
    drop(stream);

    let final_settings = *settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Err(error) = save_settings(&final_settings) {
        eprintln!("warning: settings were not saved: {error:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentage_parser_accepts_bounds() {
        assert_eq!(parse_percentage("0").unwrap(), 0.0);
        assert_eq!(parse_percentage("25").unwrap(), 0.25);
        assert_eq!(parse_percentage("100").unwrap(), 1.0);
    }

    #[test]
    fn percentage_parser_rejects_invalid_values() {
        assert!(parse_percentage("-1").is_err());
        assert!(parse_percentage("101").is_err());
        assert!(parse_percentage("NaN").is_err());
        assert!(parse_percentage("loud").is_err());
    }

    #[test]
    fn mix_parser_accepts_pairs_and_whitespace() {
        let mix = parse_mix("rain=60, brown=40").unwrap();
        assert!((mix.rain - 0.6).abs() < 1e-6);
        assert!((mix.brown - 0.4).abs() < 1e-6);
        assert_eq!(mix.white, 0.0);
        assert_eq!(mix.pink, 0.0);

        let solo = parse_mix("PINK=100").unwrap();
        assert_eq!(solo, SourceMix::solo(SoundStyle::Pink));

        // The legacy source name still works, matching --style.
        let legacy = parse_mix("vanilla=50").unwrap();
        assert!((legacy.white - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mix_parser_rejects_malformed_input() {
        assert!(parse_mix("rain").is_err());
        assert!(parse_mix("ocean=50").is_err());
        assert!(parse_mix("rain=60,rain=40").is_err());
        assert!(parse_mix("rain=101").is_err());
        assert!(parse_mix("rain=-5").is_err());
        assert!(parse_mix("rain=loud").is_err());
        assert!(parse_mix("").is_err());
        // A mix where every listed source is zero is silent, not a mix.
        assert!(parse_mix("rain=0,brown=0").is_err());
    }
}
