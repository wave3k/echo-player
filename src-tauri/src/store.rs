use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Ce qu'on garde entre deux lancements : le dossier choisi.
#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub music_folder: Option<String>,
}

/// La bibliotheque mise en cache pour demarrer vite.
#[derive(Serialize, Deserialize, Default)]
pub struct Cache {
    pub folder: String,
    pub tracks: Vec<crate::library::Track>,
}

fn config_file(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

fn cache_file(dir: &Path) -> PathBuf {
    dir.join("library.json")
}

pub fn covers_dir(cache_dir: &Path) -> PathBuf {
    let dir = cache_dir.join("covers");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn load_config(dir: &Path) -> Config {
    std::fs::read_to_string(config_file(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(dir: &Path, config: &Config) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_string(config) {
        let _ = std::fs::write(config_file(dir), json);
    }
}

pub fn load_cache(dir: &Path) -> Option<Cache> {
    let s = std::fs::read_to_string(cache_file(dir)).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn save_cache(dir: &Path, cache: &Cache) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = std::fs::write(cache_file(dir), json);
    }
}
