use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, Host};

pub fn select_host(requested: Option<&str>) -> Result<Host> {
    let Some(requested) = requested else {
        return Ok(cpal::default_host());
    };

    let matches: Vec<_> = cpal::available_hosts()
        .into_iter()
        .filter(|id| id.to_string().eq_ignore_ascii_case(requested))
        .collect();

    match matches.as_slice() {
        [host_id] => cpal::host_from_id(*host_id)
            .with_context(|| format!("failed to initialize the {host_id} audio host")),
        [] => {
            let available = host_names();
            bail!(
                "audio host '{requested}' is unavailable (available: {})",
                available.join(", ")
            )
        }
        _ => unreachable!("audio host identifiers are unique"),
    }
}

pub fn list_hosts() {
    let default = cpal::default_host().id();
    println!("Available audio hosts:");
    for host in cpal::available_hosts() {
        let suffix = if host == default { " (default)" } else { "" };
        println!("  {host}{suffix}");
    }
}

pub fn list_audio_devices(host: &Host) -> Result<()> {
    let default = host.default_output_device();
    println!("Audio devices on {}:", host.id());

    // Do not probe every ALSA plugin for a supported configuration here. Some
    // bridge plugins attempt a server connection during that probe and can block
    // device listing when PipeWire/PulseAudio is unavailable.
    for device in host.devices()? {
        let is_default = default
            .as_ref()
            .is_some_and(|candidate| candidate == &device);
        let description = device.description()?;
        let marker = if is_default { "*" } else { " " };
        println!(
            "  {marker} {} [{:?}]",
            description.name(),
            description.direction()
        );
    }
    Ok(())
}

pub fn select_output_device(host: &Host, requested: Option<&str>) -> Result<Device> {
    match requested {
        Some(name) => find_device_by_name(host, name),
        None => host
            .default_output_device()
            .context("no default output device is available"),
    }
}

fn find_device_by_name(host: &Host, requested: &str) -> Result<Device> {
    let devices: Vec<(Device, String)> = host
        .devices()?
        .map(|device| {
            let name = device
                .description()
                .map(|description| description.name().to_owned())
                .unwrap_or_else(|_| device.to_string());
            (device, name)
        })
        .collect();

    if let Some((device, _)) = devices
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(requested))
    {
        return Ok(device.clone());
    }

    let requested = requested.to_lowercase();
    let partial_matches: Vec<_> = devices
        .iter()
        .filter(|(_, name)| name.to_lowercase().contains(&requested))
        .collect();

    match partial_matches.as_slice() {
        [(device, _)] => Ok(device.clone()),
        [] => {
            let names = devices
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("output device was not found (available: {names})")
        }
        matches => {
            let names = matches
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("device name is ambiguous; it matches: {names}")
        }
    }
}

fn host_names() -> Vec<String> {
    cpal::available_hosts()
        .into_iter()
        .map(|host| host.to_string())
        .collect()
}
