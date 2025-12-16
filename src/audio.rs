use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

use crate::config::Config;

/// Audio data container with samples and sample rate
#[derive(Debug, Clone)]
pub struct AudioData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Record audio from microphone
/// If duration is 0, records until Ctrl+C
#[allow(dead_code)] // Used by CLI binary
pub fn record(duration_secs: u64, config: &Config) -> Result<AudioData> {
    let host = cpal::default_host();

    // Get input device
    let device = match &config.audio.device {
        Some(name) => host
            .input_devices()?
            .find(|d| d.name().map(|n| n.contains(name)).unwrap_or(false))
            .ok_or_else(|| anyhow!("Device '{}' not found", name))?,
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow!("No default input device"))?,
    };

    info!("Using device: {}", device.name().unwrap_or_default());

    // Configure stream for Whisper (16kHz mono f32)
    let target_sample_rate = config.audio.sample_rate;
    let stream_config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(target_sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    // Shared buffer for audio samples
    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&samples);

    // Flag to stop recording
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    // Setup Ctrl+C handler
    let running_ctrlc = Arc::clone(&running);
    ctrlc::set_handler(move || {
        info!("Stopping recording...");
        running_ctrlc.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    // Build input stream
    let err_fn = |err| eprintln!("Stream error: {}", err);

    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if running_clone.load(Ordering::SeqCst) {
                    if let Ok(mut buffer) = samples_clone.lock() {
                        buffer.extend_from_slice(data);
                    }
                }
            },
            err_fn,
            None,
        )
        .context("Failed to build input stream")?;

    stream.play().context("Failed to start stream")?;
    debug!("Recording started at {}Hz", target_sample_rate);

    // Wait for duration or Ctrl+C
    if duration_secs > 0 {
        let start = std::time::Instant::now();
        while running.load(Ordering::SeqCst)
            && start.elapsed().as_secs() < duration_secs
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    } else {
        // Wait indefinitely until Ctrl+C
        while running.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    running.store(false, Ordering::SeqCst);
    drop(stream);

    let final_samples = samples
        .lock()
        .map_err(|_| anyhow!("Failed to lock samples"))?
        .clone();

    info!("Recorded {} samples", final_samples.len());

    Ok(AudioData {
        samples: final_samples,
        sample_rate: target_sample_rate,
    })
}

/// List available audio input devices
#[allow(dead_code)] // Used by CLI binary
pub fn list_devices() -> Result<()> {
    let host = cpal::default_host();

    println!("Available input devices:");
    for device in host.input_devices()? {
        let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        let is_default = host
            .default_input_device()
            .map(|d| d.name().ok() == device.name().ok())
            .unwrap_or(false);

        if is_default {
            println!("  * {} (default)", name);
        } else {
            println!("    {}", name);
        }

        // Show supported configs
        if let Ok(configs) = device.supported_input_configs() {
            for config in configs {
                debug!(
                    "    - {} channels, {}-{}Hz",
                    config.channels(),
                    config.min_sample_rate().0,
                    config.max_sample_rate().0
                );
            }
        }
    }

    Ok(())
}
