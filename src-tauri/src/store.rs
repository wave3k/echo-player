use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Tout ce qui se regle dans l'application et se garde entre deux lancements.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Dossiers balayes pour trouver la musique et les clips.
    pub folders: Vec<String>,
    /// "system", "light" ou "dark"
    pub theme: String,
    /// "nuit", "aurore", "menthe" ou "prune"
    pub palette: String,
    /// "space", "sora" ou "outfit"
    pub font: String,
    /// "normal", "compact" ou "minimal"
    pub mode: String,
    pub animations: bool,
    /// Arrondi des cartes et des pochettes, en pixels (0 a 20).
    pub radius: u32,
    /// "petit", "normal" ou "grand"
    pub text_size: String,
    pub show_covers: bool,

    pub resume_last: bool,
    pub remember_position: bool,
    pub shuffle_default: bool,
    /// "off", "all" ou "one"
    pub repeat_default: String,
    pub skip_seconds: u32,
    pub next_on_error: bool,

    pub media_keys: bool,
    pub autostart: bool,
    /// true : on relit les dossiers a chaque demarrage.
    pub refresh_on_start: bool,
    /// true : on demande avant de fermer. false : on ferme tout de suite.
    pub ask_before_quit: bool,
    /// Extensions lues par la bibliotheque.
    pub formats: Vec<String>,

    pub volume: u32,
    pub last_track: Option<String>,
    pub last_position_ms: u64,
    pub last_playing: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            theme: "dark".into(),
            palette: "nuit".into(),
            font: "space".into(),
            mode: "normal".into(),
            animations: true,
            radius: 12,
            text_size: "normal".into(),
            show_covers: true,
            resume_last: true,
            remember_position: true,
            shuffle_default: false,
            repeat_default: "off".into(),
            skip_seconds: 10,
            next_on_error: true,
            media_keys: true,
            autostart: false,
            refresh_on_start: true,
            ask_before_quit: false,
            formats: vec!["mp3".into(), "mp4".into()],
            volume: 90,
            last_track: None,
            last_position_ms: 0,
            last_playing: false,
        }
    }
}

/// Les formats proposes dans les reglages, dans l'ordre d'affichage.
pub const FORMAT_CATALOG: &[&str] = &["mp3", "mp4", "flac", "m4a", "ogg", "wav", "webm", "mkv"];

/// La bibliotheque mise en cache pour demarrer vite.
#[derive(Serialize, Deserialize, Default)]
pub struct Cache {
    pub folder: String,
    pub tracks: Vec<crate::library::Track>,
}

fn settings_file(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

fn cache_file(dir: &Path) -> PathBuf {
    dir.join("library.json")
}

pub fn covers_dir(cache_dir: &Path) -> PathBuf {
    let dir = cache_dir.join("covers");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn load_settings(dir: &Path) -> Settings {
    std::fs::read_to_string(settings_file(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(dir: &Path, settings: &Settings) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(settings_file(dir), json);
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

/// Reprend les dossiers de l'ancienne version (MusicApp) au premier lancement d'Echo.
/// Ancien dossier de configuration : "com.musicapp.player", nouveau : "com.echo.player".
pub fn migrate_from_musicapp(config_dir: &Path, cache_dir: &Path) {
    if settings_file(config_dir).exists() {
        return;
    }
    let Some(parent) = config_dir.parent() else {
        return;
    };

    let old_config = parent.join("com.musicapp.player");
    let old_settings = old_config.join("config.json");
    if old_settings.exists() {
        #[derive(Deserialize, Default)]
        struct Old {
            #[serde(default)]
            music_folder: Option<String>,
        }
        if let Ok(text) = std::fs::read_to_string(&old_settings) {
            if let Ok(old) = serde_json::from_str::<Old>(&text) {
                let mut settings = Settings::default();
                if let Some(folder) = old.music_folder {
                    if Path::new(&folder).is_dir() {
                        settings.folders.push(folder);
                    }
                }
                save_settings(config_dir, &settings);
            }
        }
        // La bibliotheque deja analysee et les pochettes : on les deplace
        // pour eviter de tout refaire au premier demarrage.
        let old_library = old_config.join("library.json");
        if old_library.exists() && !cache_file(config_dir).exists() {
            let _ = std::fs::rename(&old_library, cache_file(config_dir));
        }
        if let Some(cache_parent) = cache_dir.parent() {
            let old_covers = cache_parent.join("com.musicapp.player").join("covers");
            let new_covers = cache_dir.join("covers");
            if old_covers.is_dir() && !new_covers.exists() {
                let _ = std::fs::rename(&old_covers, &new_covers);
            }
        }
    }
}
