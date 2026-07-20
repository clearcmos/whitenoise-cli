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
use crate::settings::{AudioSettings, SoundStyle, load_settings, save_settings};
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
    #[arg(short, long, value_enum)]
    style: Option<SoundStyle>,
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
    if let Some(style) = args.style {
        initial_settings.sound_style = style;
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
            initial_settings.sound_style.label(),
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
}
