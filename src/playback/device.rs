// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::HostTrait;
use cpal::Device;

#[derive(Debug, Clone)]
pub struct OutputDeviceInfo {
    pub index: usize,
    pub name: String,
}

pub fn list_output_devices() -> Result<Vec<OutputDeviceInfo>> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    for (index, device) in host.output_devices()?.enumerate() {
        let name = device.to_string();
        devices.push(OutputDeviceInfo { index, name });
    }
    Ok(devices)
}

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

    let names: Vec<String> = devices.iter().map(|d| d.to_string()).collect();

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

pub fn print_output_devices() -> Result<()> {
    for info in list_output_devices()? {
        println!("[{}] {}", info.index, info.name);
    }
    Ok(())
}
