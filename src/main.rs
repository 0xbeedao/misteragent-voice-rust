use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use ringbuf::HeapRb;
use tokio;
use actix_web::{web, App, HttpServer, HttpResponse};
use log;
use env_logger;
use parking_lot;
use argh::FromArgs;
use chrono;
use dotenv::dotenv;

mod wakeword_listener;
mod capture_audio;
use capture_audio::{capture_audio, get_input_config};

/// Audio recording application
#[derive(FromArgs)]
struct Args {
    /// number of seconds of audio to buffer (default: 60)
    #[argh(option, default = "60")]
    seconds: u32,

    /// directory to store output WAV files (default: ".")
    #[argh(option, default = "String::from(\"captures\")")]
    output_dir: String,
}

// Structure to hold our audio data and state
struct AudioState {
    buffer: parking_lot::Mutex<HeapRb<f32>>,
    is_recording: AtomicBool,
    is_halting: AtomicBool,
    output_dir: String,
}

impl AudioState {
    fn new(capacity: usize, output_dir: String) -> Self {
        AudioState {
            buffer: parking_lot::Mutex::new(HeapRb::new(capacity)),
            is_recording: AtomicBool::new(true),
            is_halting: AtomicBool::new(false),
            output_dir,
        }
    }
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
    // Generate timestamp for unique filename
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("recording_{}.wav", timestamp);
    let filepath = std::path::Path::new(&state.output_dir).join(filename);
    
    log::info!("Saving audio to {}", filepath.display());
    
    let config = get_input_config();
    log::debug!("Using input config: {:?}", config);

    match capture_audio::save_audio_to_file(&state, &filepath, &config) {
        Ok(sample_count) => {
            log::info!("Successfully saved {} samples to {}", sample_count, filepath.display());
            HttpResponse::Ok().body(format!("Audio saved to {}", filepath.display()))
        }
        Err(e) => {
            log::error!("Failed to save audio: {}", e);
            HttpResponse::InternalServerError().body(format!("Failed to save audio: {}", e))
        }
    }
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
    // wait for response
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Exit process cleanly
    std::process::exit(0);
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load environment variables from .env file
    dotenv().ok();

    // Get command line arguments
    let args: Args = argh::from_env();
    
    // Initialize logger
    env_logger::init();
    log::info!("Starting audio recording application");

    // Calculate buffer size using the input config and CLI argument
    let config = get_input_config();
    let buffer_size: usize = config.sample_rate().0 as usize * args.seconds as usize;
    log::info!("Initializing buffer for {} seconds ({} samples)", args.seconds, buffer_size);
    
    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&args.output_dir)
        .expect("Failed to create output directory");
    log::info!("Using output directory: {}", args.output_dir);

    let state = Arc::new(AudioState::new(buffer_size, args.output_dir));
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