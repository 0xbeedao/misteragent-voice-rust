use porcupine::{Porcupine, PorcupineBuilder, BuiltinKeywords};
use std::env;
use std::path::Path;

pub fn get_wakeword_listener() -> Porcupine {
    let access_key = env::var("PICOVOICE_ACCESS_KEY").unwrap_or_else(|_| {
        panic!("PICOVOICE_ACCESS_KEY is not set");
    });
    let dir = env!("CARGO_MANIFEST_DIR");
    let ppn_file = env::var("PORCUPINE_MODEL_PATH").unwrap_or_else(|_| {
        panic!("PORCUPINE_MODEL_PATH is not set");
    });
    let full_path = Path::new(dir).join(ppn_file);
    log::info!("Porcupine model path: {}", full_path.display());
    
    PorcupineBuilder::new_with_keywords(
        access_key, 
        &[BuiltinKeywords::Porcupine]
    ).init().expect("Unable to create Porcupine")

    // PorcupineBuilder::new_with_keyword_paths(
    //     &access_key,
    //     &[full_path],
    // ).init().expect("Failed to create Porcupine instance")
}
