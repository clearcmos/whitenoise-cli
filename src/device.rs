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

    let names: Vec<String> = devices.iter().map(|(_, name)| name.clone()).collect();
    let index = match_device_name(&names, requested)?;
    Ok(devices[index].0.clone())
}

// The name-matching contract, kept separate from CPAL so it is testable:
// a case-insensitive exact match wins, then a unique substring match; zero
// or multiple substring matches fail with the candidates listed rather than
// selecting an arbitrary device.
fn match_device_name(names: &[String], requested: &str) -> Result<usize> {
    if let Some(index) = names
        .iter()
        .position(|name| name.eq_ignore_ascii_case(requested))
    {
        return Ok(index);
    }

    let requested = requested.to_lowercase();
    let partial_matches: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.to_lowercase().contains(&requested))
        .map(|(index, _)| index)
        .collect();

    match partial_matches.as_slice() {
        [index] => Ok(*index),
        [] => {
            let names = names
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            bail!("output device was not found (available: {names})")
        }
        matches => {
            let names = matches
                .iter()
                .map(|index| names[*index].as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn exact_match_is_case_insensitive() {
        let devices = names(&["Family 17h HD Audio Controller", "USB Headphones"]);
        let index = match_device_name(&devices, "usb headphones").unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn exact_match_wins_over_substring_matches() {
        // "USB" is a substring of both names, but an exact (case-insensitive)
        // name match must win instead of failing as ambiguous.
        let devices = names(&["USB Headphones", "USB"]);
        let index = match_device_name(&devices, "usb").unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn unique_substring_match_is_accepted() {
        let devices = names(&["HDMI Output", "USB Headphones"]);
        let index = match_device_name(&devices, "headph").unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn ambiguous_substring_match_is_rejected_with_candidates() {
        let devices = names(&["USB Headphones", "USB Speakers", "HDMI Output"]);
        let error = match_device_name(&devices, "usb ").unwrap_err().to_string();
        assert!(error.contains("ambiguous"), "unexpected error: {error}");
        assert!(error.contains("USB Headphones"));
        assert!(error.contains("USB Speakers"));
        assert!(!error.contains("HDMI Output"));
    }

    #[test]
    fn no_match_lists_available_devices() {
        let devices = names(&["HDMI Output", "USB Headphones"]);
        let error = match_device_name(&devices, "bluetooth")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not found"), "unexpected error: {error}");
        assert!(error.contains("HDMI Output, USB Headphones"));
    }

    #[test]
    fn duplicate_names_resolve_to_the_first_device() {
        // ALSA can expose identical descriptions; matching must stay
        // deterministic rather than depending on iteration luck.
        let devices = names(&["Duplicate", "Duplicate"]);
        let index = match_device_name(&devices, "duplicate").unwrap();
        assert_eq!(index, 0);
    }
}
