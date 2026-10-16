mod library;
mod mpris;
mod store;

use base64::Engine;
use library::Track;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

#[derive(Default)]
struct Data {
    folder: Option<String>,
    tracks: Vec<Track>,
    covers: HashMap<String, PathBuf>,
}

struct AppData(Mutex<Data>);

/// Controles media du systeme (touches du clavier, widget du bureau).
struct Media(Mutex<mpris::Mpris>);

/// Transforme le chemin du fichier de pochette en URL utilisable par MPRIS.
fn cover_url_for(data: &Data, id: &str) -> Option<String> {
    let path = data.covers.get(id)?;
    Some(format!("file://{}", path.to_string_lossy()))
}

#[derive(Serialize)]
struct Snapshot {
    folder: Option<String>,
    folder_missing: bool,
    tracks: Vec<Track>,
}

fn snapshot(data: &Data) -> Snapshot {
    let folder_missing = match &data.folder {
        Some(f) => !Path::new(f).is_dir(),
        None => false,
    };
    Snapshot {
        folder: data.folder.clone(),
        folder_missing,
        tracks: data.tracks.clone(),
    }
}

fn config_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn cache_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_cache_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Relit le dossier courant et remplit la memoire + le cache disque.
fn do_scan(app: &tauri::AppHandle, data: &mut Data) -> Snapshot {
    let Some(folder) = data.folder.clone() else {
        return snapshot(data);
    };
    let path = Path::new(&folder);
    if !path.is_dir() {
        data.tracks.clear();
        data.covers.clear();
        return snapshot(data);
    }

    let covers = store::covers_dir(&cache_dir(app));
    let (tracks, cover_map) = library::scan(path, &covers);
    data.tracks = tracks;
    data.covers = cover_map;

    store::save_cache(
        &config_dir(app),
        &store::Cache {
            folder: folder.clone(),
            tracks: data.tracks.clone(),
        },
    );

    snapshot(data)
}

#[tauri::command]
fn get_state(state: State<AppData>) -> Snapshot {
    let data = state.0.lock().unwrap();
    snapshot(&data)
}

#[tauri::command]
fn set_folder(
    app: tauri::AppHandle,
    state: State<AppData>,
    path: String,
) -> Result<Snapshot, String> {
    let p = Path::new(&path);
    if !p.is_dir() {
        return Err("Ce dossier n'existe pas.".into());
    }
    {
        let mut data = state.0.lock().unwrap();
        data.folder = Some(path.clone());
        data.tracks.clear();
        data.covers.clear();
    }
    store::save_config(
        &config_dir(&app),
        &store::Config {
            music_folder: Some(path),
        },
    );
    let _ = app.emit("scanning", true);
    let snapshot = {
        let mut data = state.0.lock().unwrap();
        do_scan(&app, &mut data)
    };
    let _ = app.emit("scanning", false);
    Ok(snapshot)
}

#[tauri::command]
fn rescan(app: tauri::AppHandle, state: State<AppData>) -> Snapshot {
    let _ = app.emit("scanning", true);
    let snapshot = {
        let mut data = state.0.lock().unwrap();
        do_scan(&app, &mut data)
    };
    let _ = app.emit("scanning", false);
    snapshot
}

#[tauri::command]
fn set_now_playing(
    state: State<AppData>,
    media: State<Media>,
    title: String,
    artist: String,
    album: String,
    cover: Option<String>,
    duration_ms: u64,
) {
    let data = state.0.lock().unwrap();
    let cover_url = cover.as_deref().and_then(|id| cover_url_for(&data, id));
    drop(data);

    let mut controls = media.0.lock().unwrap();
    controls.set_metadata(
        &title,
        &artist,
        &album,
        cover_url,
        Some(std::time::Duration::from_millis(duration_ms)),
    );
}

#[tauri::command]
fn set_playback(media: State<Media>, playing: bool, position_ms: u64) {
    let mut controls = media.0.lock().unwrap();
    controls.set_playback(
        playing,
        Some(std::time::Duration::from_millis(position_ms)),
    );
}

#[tauri::command]
fn get_cover(state: State<AppData>, id: String) -> Option<String> {
    let data = state.0.lock().unwrap();
    let path = data.covers.get(&id)?;
    let bytes = std::fs::read(path).ok()?;
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
    {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppData(Mutex::new(Data::default())))
        .setup(|app| {
            let handle = app.handle().clone();

            // Touches media du clavier et widget du bureau.
            let mut controls = mpris::Mpris::new();
            let handle_events = app.handle().clone();
            controls.attach(move |event| {
                if let Some(label) = mpris::describe(&event) {
                    let _ = handle_events.emit("media-control", label);
                }
            });
            app.manage(Media(Mutex::new(controls)));
            let cfg = store::load_config(&config_dir(&handle));
            {
                let state = handle.state::<AppData>();
                let mut data = state.0.lock().unwrap();
                data.folder = cfg.music_folder.clone();

                // Demarrage rapide : on affiche le cache tout de suite.
                if let Some(cache) = store::load_cache(&config_dir(&handle)) {
                    if Some(cache.folder.clone()) == cfg.music_folder {
                        data.tracks = cache.tracks;
                        let covers = store::covers_dir(&cache_dir(&handle));
                        let ids: Vec<String> = data
                            .tracks
                            .iter()
                            .filter_map(|t| t.cover.clone())
                            .collect();
                        for id in ids {
                            data.covers.insert(id.clone(), covers.join(&id));
                        }
                    }
                }
            }

            // Puis on rescanne en tache de fond si le dossier existe.
            let needs_scan = {
                let state = handle.state::<AppData>();
                let data = state.0.lock().unwrap();
                match &data.folder {
                    Some(f) => Path::new(f).is_dir(),
                    None => false,
                }
            };
            if needs_scan {
                let handle2 = handle.clone();
                std::thread::spawn(move || {
                    let _ = handle2.emit("scanning", true);
                    let result = {
                        let state = handle2.state::<AppData>();
                        let mut data = state.0.lock().unwrap();
                        do_scan(&handle2, &mut data)
                    };
                    let _ = handle2.emit("library-updated", &result);
                    let _ = handle2.emit("scanning", false);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_folder,
            rescan,
            get_cover,
            set_now_playing,
            set_playback
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de MusicApp");
}
