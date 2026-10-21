//! Hors Linux, la page lit elle-même la musique (WebView2) : le lecteur du cœur
//! n'existe pas, ses commandes ne font rien.

use serde::Serialize;
use tauri::AppHandle;

pub struct Player;

impl Player {
    pub fn new(_app: &AppHandle) -> Self {
        Player
    }
}

#[derive(Serialize)]
pub struct Snapshot;

pub fn available(_app: &AppHandle) -> bool {
    false
}

pub fn start(_app: &AppHandle) {}

pub fn configure(_app: &AppHandle, _settings: &crate::store::Settings) {}

pub fn media_control(_app: &AppHandle, _action: &str) -> bool {
    false
}

pub fn save_now(_app: &AppHandle) {}

pub fn visual_off(_app: &AppHandle) {}

#[tauri::command]
pub fn player_play(_ids: Vec<String>, _index: usize, _autoplay: bool, _start_ms: Option<u64>, _shuffle: bool, _repeat: String) {}

#[tauri::command]
pub fn player_queue(_ids: Vec<String>, _index: usize, _shuffle: bool, _repeat: String) {}

#[tauri::command]
pub fn player_set_playing(_playing: bool) {}

#[tauri::command]
pub fn player_step(_delta: i32) {}

#[tauri::command]
pub fn player_seek(_ms: u64) {}

#[tauri::command]
pub fn player_volume(_volume: f64, _muted: bool) {}

#[tauri::command]
pub fn player_visual(_on: bool) {}

#[tauri::command]
pub fn player_snapshot() -> Option<Snapshot> {
    None
}
