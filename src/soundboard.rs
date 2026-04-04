use std::process::Command;

use log::{error, info};

/// Play a sound file asynchronously via afplay (for daemon use)
pub fn play(sound_path: &str) {
    info!("Playing sound: {}", sound_path);
    let path = sound_path.to_string();
    std::thread::spawn(move || {
        let result = Command::new("afplay")
            .arg(&path)
            .output();
        if let Err(e) = result {
            error!("Failed to play sound '{}': {}", path, e);
        }
    });
}

/// Play a sound file synchronously (for CLI subcommand use)
pub fn play_sync(sound_path: &str) {
    let result = Command::new("afplay")
        .arg(sound_path)
        .status();
    if let Err(e) = result {
        eprintln!("Failed to play sound: {}", e);
    }
}

/// Play a named sound from the assets/sounds directory
/// Searches for .wav, .mp3, .aiff, .caf extensions
pub fn play_named(name: &str) {
    if let Some(path) = find_sound(name) {
        play(&path);
    } else {
        error!("Sound not found: {}", name);
    }
}

/// Play a named sound synchronously (blocks until done)
pub fn play_named_sync(name: &str) {
    if let Some(path) = find_sound(name) {
        play_sync(&path);
    } else {
        eprintln!("Sound not found: {}", name);
    }
}

pub fn list_sounds() {
    let dir = format!("{}/assets/sounds", env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let ext = name.rsplit('.').next()?;
            if ["wav", "mp3", "aiff", "caf", "m4a"].contains(&ext) {
                Some(name.rsplit_once('.').unwrap().0.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names.dedup();
    println!("Available sounds:");
    for name in names {
        println!("  {}", name);
    }
}

fn find_sound(name: &str) -> Option<String> {
    let base = format!("{}/assets/sounds/{}", env!("CARGO_MANIFEST_DIR"), name);
    for ext in &["wav", "mp3", "aiff", "caf", "m4a"] {
        let path = format!("{}.{}", base, ext);
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}
