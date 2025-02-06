use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use ringbuf::traits::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log;
use hound;
use std::path::Path;

use crate::AudioState;
use crate::wakeword_listener::get_wakeword_listener;

// Get the input config
pub fn get_input_config() -> cpal::SupportedStreamConfig {
    let host = cpal::default_host();
    let device = host.default_input_device()
        .expect("Failed to get default input device");
    device.default_input_config()
        .expect("Failed to get default input config")
}

// Audio capture function
pub async fn capture_audio(state: Arc<AudioState>) {
    log::info!("Initializing audio capture");
    let host = cpal::default_host();
    let device = host.default_input_device()
        .expect("Failed to get default input device");
    
    log::info!("Using input device: {}", device.name().unwrap_or_default());
    
    let config = device.default_input_config()
        .expect("Failed to get default input config");
    
    log::debug!("Audio config: {:?}", config);

    // Initialize Porcupine
    let porcupine = get_wakeword_listener();
    let frame_length = porcupine.frame_length() as usize;
    log::info!("Porcupine initialized with frame length: {}", frame_length);
    
    let state_clone = Arc::clone(&state);
    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &_| {
            // Store in recording buffer if recording
            if state_clone.is_recording.load(Ordering::Relaxed) {
                let mut buffer = state_clone.buffer.lock();
                for &sample in data {
                    buffer.push_overwrite(sample);
                }
            }

            // Convert samples to i16, logging any potential conversion issues
            let i16_samples: Vec<i16> = data.iter()
                .map(|&x| {
                    let scaled = x * i16::MAX as f32;
                    if scaled > i16::MAX as f32 || scaled < i16::MIN as f32 {
                        log::warn!("Sample value {} out of i16 range after scaling", scaled);
                    }
                    scaled as i16
                })
                .collect();
            
            // Process with Porcupine in chunks of the required size
            for chunk in i16_samples.chunks(frame_length) {
                if chunk.len() == frame_length {
                    match porcupine.process(chunk) {
                        Ok(keyword_index) => {
                            if keyword_index >= 0 {
                                log::info!("Wakeword detected: {}", keyword_index);
                            }
                        }
                        Err(err) => {
                            log::error!("Error processing audio: {:?}", err);
                        }
                    }
                }
            }
        },

        |err| log::error!("Error in audio stream: {}", err),
        Some(Duration::from_secs(1)),


    ).expect("Failed to build input stream");

    log::info!("Starting audio stream");
    stream.play().expect("Failed to start audio stream");

    // Keep the stream alive until the server is halted
    while !state.is_halting.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    log::info!("Shutting down capture audio thread");
    
    // Explicitly drop the stream before the function ends
    drop(stream);
}

pub fn save_audio_to_file(
    state: &AudioState,
    filepath: &Path,
    config: &cpal::SupportedStreamConfig
) -> std::io::Result<usize> {
    let spec = hound::WavSpec {
        channels: config.channels() as u16,
        sample_rate: config.sample_rate().0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    log::debug!("Creating WAV with spec: {:?}", spec);

    // Create output directory if it doesn't exist
    if let Some(parent) = filepath.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut writer = hound::WavWriter::create(filepath, spec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Convert ring buffer to vec and write to file
    let buffer_contents: Vec<f32> = {
        let buffer = state.buffer.lock();
        buffer.iter().copied().collect()
    };
    
    log::info!("Writing {} samples to WAV file", buffer_contents.len());
    for &sample in &buffer_contents {
        writer.write_sample(sample)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }

    writer.finalize()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    
    Ok(buffer_contents.len())
}
