// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Output device enumeration and resolution.

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::HostTrait;
use cpal::Device;

/// Summarized output device for UI / CLI listing.
#[derive(Debug, Clone)]
pub struct OutputDeviceInfo {
    /// Index in the host output device list.
    pub index: usize,
    /// Display name.
    pub name: String,
}

/// List host output devices.
pub fn list_output_devices() -> Result<Vec<OutputDeviceInfo>> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    for (index, device) in host.output_devices()?.enumerate() {
        let name = output_device_name(&device);
        devices.push(OutputDeviceInfo { index, name });
    }
    Ok(devices)
}

/// Display name for a CPAL device.
pub fn output_device_name(device: &Device) -> String {
    device.to_string()
}

/// Resolve `--output` spec (index, exact name, or substring) to a device.
pub fn resolve_output_device(spec: Option<&str>) -> Result<Device> {
    let host = cpal::default_host();
    let Some(spec) = spec else {
        return host
            .default_output_device()
            .context("no default output audio device found");
    };

    let devices: Vec<Device> = host.output_devices()?.collect();
    if devices.is_empty() {
        bail!("no output audio devices found");
    }

    if let Ok(index) = spec.parse::<usize>() {
        return devices
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow!("output device index {index} out of range"));
    }

    let names: Vec<String> = devices.iter().map(output_device_name).collect();

    if let Some(index) = names.iter().position(|n| n == spec) {
        return Ok(devices[index].clone());
    }

    if let Some(index) = names
        .iter()
        .position(|n| n.to_lowercase().contains(&spec.to_lowercase()))
    {
        return Ok(devices[index].clone());
    }

    Err(anyhow!(
        "output device not found: {spec:?} (use --list-devices)"
    ))
}

/// Print devices as `[index] name` lines.
pub fn print_output_devices() -> Result<()> {
    for info in list_output_devices()? {
        println!("[{}] {}", info.index, info.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_none_uses_host_default() {
        let host = cpal::default_host();
        let Some(expected) = host.default_output_device() else {
            return;
        };
        let resolved = resolve_output_device(None).expect("default output device");
        assert_eq!(output_device_name(&resolved), output_device_name(&expected));
    }
}
