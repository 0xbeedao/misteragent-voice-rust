use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{traits::*, HeapRb};
use tokio;
use actix_web::{web, App, HttpServer, HttpResponse};
use hound;
use log;
use env_logger;
use parking_lot;

// Structure to hold our audio data and state
struct AudioState {
    buffer: parking_lot::Mutex<HeapRb<f32>>,
    is_recording: AtomicBool,
    is_halting: AtomicBool,
}

impl AudioState {
    fn new(capacity: usize) -> Self {
        AudioState {
            buffer: parking_lot::Mutex::new(HeapRb::new(capacity)),
            is_recording: AtomicBool::new(true),
            is_halting: AtomicBool::new(false),
        }
    }
}

// Get the input config
fn get_input_config() -> cpal::SupportedStreamConfig {
    let host = cpal::default_host();
    let device = host.default_input_device()
        .expect("Failed to get default input device");
    device.default_input_config()
        .expect("Failed to get default input config")
}

// Audio capture function
async fn capture_audio(state: Arc<AudioState>) {
    log::info!("Initializing audio capture");
    let host = cpal::default_host();
    let device = host.default_input_device()
        .expect("Failed to get default input device");
    
    log::info!("Using input device: {}", device.name().unwrap_or_default());
    
    let config = device.default_input_config()
        .expect("Failed to get default input config");
    
    log::debug!("Audio config: {:?}", config);
    
    let state_clone = Arc::clone(&state);
    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &_| {
            if state_clone.is_recording.load(Ordering::Relaxed) {
                log::trace!("Recording {} samples", data.len());
                let mut buffer = state_clone.buffer.lock();
                for &sample in data {
                    buffer.push_overwrite(sample);
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

// HTTP endpoint handlers
async fn start_recording(state: web::Data<Arc<AudioState>>) -> HttpResponse {
    log::info!("Starting recording");
    state.is_recording.store(true, Ordering::Relaxed);
    HttpResponse::Ok().body("Recording started")
}

async fn stop_recording(state: web::Data<Arc<AudioState>>) -> HttpResponse {
    log::info!("Stopping recording");
    state.is_recording.store(false, Ordering::Relaxed);
    HttpResponse::Ok().body("Recording stopped")
}

async fn save_audio(state: web::Data<Arc<AudioState>>) -> HttpResponse {
    log::info!("Saving audio report to output.wav");
    
    let config = get_input_config();
    log::debug!("Using input config: {:?}", config);

    let spec = hound::WavSpec {
        channels: config.channels() as u16,
        sample_rate: config.sample_rate().0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    log::debug!("Creating WAV with spec: {:?}", spec);

    let mut writer = hound::WavWriter::create("output.wav", spec)
        .expect("Failed to create WAV file");

    // Convert ring buffer to vec and write to file
    let buffer_contents: Vec<f32> = {
        let buffer = state.buffer.lock();
        buffer.iter().copied().collect()
    };
    
    log::info!("Writing {} samples to WAV file", buffer_contents.len());
    for &sample in &buffer_contents {
        writer.write_sample(sample).expect("Failed to write sample");
    }

    writer.finalize().expect("Failed to finalize WAV file");
    log::info!("Successfully saved audio to output.wav");
    HttpResponse::Ok().body("Audio saved to output.wav")
}

async fn halt_server(state: web::Data<Arc<AudioState>>) -> HttpResponse {
    log::info!("Halting server");
    // First stop recording
    state.is_recording.store(false, Ordering::Relaxed);
    // Signal the capture thread to stop
    state.is_halting.store(true, Ordering::Relaxed);
    
    // Give a brief moment for the recording thread to clean up
    tokio::time::sleep(Duration::from_millis(200)).await;
    HttpResponse::Ok().body("Server halted");
    // Exit process cleanly
    std::process::exit(0);
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize logger
    env_logger::init();
    log::info!("Starting audio recording application");

    // Calculate buffer size using the input config
    let config = get_input_config();
    let buffer_size: usize = config.sample_rate().0 as usize * 60;
    log::info!("Initializing buffer with size: {}", buffer_size);
    let state = Arc::new(AudioState::new(buffer_size));
    let state_clone = Arc::clone(&state);

    // Spawn audio capture task in a dedicated thread
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(capture_audio(state_clone));
    });

    // Set up ctrl-c handler
    let state_clone = Arc::clone(&state);
    ctrlc::set_handler(move || {
        state_clone.is_recording.store(false, Ordering::Relaxed);
        std::process::exit(0);
    }).expect("Failed to set Ctrl-C handler");

    log::info!("Starting HTTP server on port 8000");
    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(Arc::clone(&state)))
            .route("/stop", web::post().to(stop_recording))
            .route("/save", web::post().to(save_audio))
            .route("/halt", web::post().to(halt_server))
            .route("/start", web::post().to(start_recording))
    })
    .bind("127.0.0.1:8000")?
    .run()
    .await
}