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

/// Ce que la bibliotheque garde en mémoire pendant que l'app tourne.
#[derive(Default)]
struct Data {
    tracks: Vec<Track>,
    covers: HashMap<String, PathBuf>,
}

struct AppData(Mutex<Data>);

/// Les reglages, relus et enregistres dans settings.json.
struct AppSettings(Mutex<store::Settings>);

/// Controles media du systeme (touches du clavier, widget du bureau).
struct Media(Mutex<mpris::Mpris>);

/// Transforme le chemin du fichier de pochette en URL utilisable par MPRIS.
fn cover_url_for(data: &Data, id: &str) -> Option<String> {
    let path = data.covers.get(id)?;
    Some(format!("file://{}", path.to_string_lossy()))
}

#[derive(Serialize)]
struct Snapshot {
    folders: Vec<String>,
    folders_missing: Vec<String>,
    tracks: Vec<Track>,
}

fn snapshot(settings: &store::Settings, data: &Data) -> Snapshot {
    Snapshot {
        folders: settings.folders.clone(),
        folders_missing: settings
            .folders
            .iter()
            .filter(|f| !Path::new(f).is_dir())
            .cloned()
            .collect(),
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

/// Relit les dossiers choisis et remplit la memoire + le cache disque.
fn do_scan(app: &tauri::AppHandle, settings: &store::Settings, data: &mut Data) -> Snapshot {
    let folders: Vec<PathBuf> = settings
        .folders
        .iter()
        .filter(|f| Path::new(f).is_dir())
        .map(PathBuf::from)
        .collect();

    if folders.is_empty() {
        data.tracks.clear();
        data.covers.clear();
        return snapshot(settings, data);
    }

    let covers = store::covers_dir(&cache_dir(app));
    let (tracks, cover_map) = library::scan(&folders, &covers, &settings.formats);
    data.tracks = tracks;
    data.covers = cover_map;

    store::save_cache(
        &config_dir(app),
        &store::Cache {
            folder: folders
                .iter()
                .map(|f| f.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            tracks: data.tracks.clone(),
        },
    );

    snapshot(settings, data)
}

fn scan_with_events(app: &tauri::AppHandle) -> Snapshot {
    let _ = app.emit("scanning", true);
    let result = {
        let settings = app.state::<AppSettings>().0.lock().unwrap().clone();
        let state = app.state::<AppData>();
        let mut data = state.0.lock().unwrap();
        do_scan(app, &settings, &mut data)
    };
    let _ = app.emit("scanning", false);
    result
}

/// Taille du dossier des pochettes, en octets.
fn folder_size(path: &Path) -> u64 {
    let Ok(read) = std::fs::read_dir(path) else {
        return 0;
    };
    read.filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[derive(Serialize)]
struct Bootstrap {
    settings: store::Settings,
    snapshot: Snapshot,
    cache_bytes: u64,
    formats: Vec<String>,
}

#[tauri::command]
fn bootstrap(app: tauri::AppHandle, state: State<AppData>, settings: State<AppSettings>) -> Bootstrap {
    let cfg = settings.0.lock().unwrap().clone();
    let data = state.0.lock().unwrap();
    Bootstrap {
        settings: cfg,
        snapshot: snapshot(&settings.0.lock().unwrap(), &data),
        cache_bytes: folder_size(&store::covers_dir(&cache_dir(&app))),
        formats: store::FORMAT_CATALOG.iter().map(|f| f.to_string()).collect(),
    }
}

#[tauri::command]
fn get_state(state: State<AppData>, settings: State<AppSettings>) -> Snapshot {
    let data = state.0.lock().unwrap();
    let cfg = settings.0.lock().unwrap();
    snapshot(&cfg, &data)
}

/// Enregistre les reglages et applique ce qui doit l'etre tout de suite.
#[tauri::command]
fn set_settings(
    app: tauri::AppHandle,
    settings: State<AppSettings>,
    media: State<Media>,
    new_settings: store::Settings,
) -> Snapshot {
    let (folders_changed, formats_changed, media_keys) = {
        let mut current = settings.0.lock().unwrap();
        let folders_changed = current.folders != new_settings.folders;
        let formats_changed = current.formats != new_settings.formats;
        *current = new_settings.clone();
        (
            folders_changed,
            formats_changed,
            new_settings.media_keys,
        )
    };

    store::save_settings(&config_dir(&app), &new_settings);
    media.0.lock().unwrap().set_enabled(media_keys);
    let _ = set_autostart_impl(&app, new_settings.autostart);

    if folders_changed || formats_changed {
        scan_with_events(&app)
    } else {
        let state = app.state::<AppData>();
        let data = state.0.lock().unwrap();
        snapshot(&new_settings, &data)
    }
}

#[tauri::command]
fn add_folder(
    app: tauri::AppHandle,
    settings: State<AppSettings>,
    path: String,
) -> Result<Snapshot, String> {
    if !Path::new(&path).is_dir() {
        return Err("Ce dossier n'existe pas.".into());
    }
    {
        let mut cfg = settings.0.lock().unwrap();
        if !cfg.folders.iter().any(|f| f == &path) {
            cfg.folders.push(path.clone());
        }
        store::save_settings(&config_dir(&app), &cfg);
    }
    Ok(scan_with_events(&app))
}

#[tauri::command]
fn remove_folder(app: tauri::AppHandle, settings: State<AppSettings>, path: String) -> Snapshot {
    {
        let mut cfg = settings.0.lock().unwrap();
        cfg.folders.retain(|f| f != &path);
        store::save_settings(&config_dir(&app), &cfg);
    }
    scan_with_events(&app)
}

#[tauri::command]
fn rescan(app: tauri::AppHandle) -> Snapshot {
    scan_with_events(&app)
}

/// Efface les pochettes mises en cache et renvoie la nouvelle taille (zero).
#[tauri::command]
fn clear_cover_cache(app: tauri::AppHandle, state: State<AppData>) -> u64 {
    let dir = store::covers_dir(&cache_dir(&app));
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.filter_map(|e| e.ok()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let mut data = state.0.lock().unwrap();
    data.covers.clear();
    for track in data.tracks.iter_mut() {
        track.cover = None;
    }
    0
}

#[tauri::command]
fn export_settings(settings: State<AppSettings>, path: String) -> Result<(), String> {
    let cfg = settings.0.lock().unwrap().clone();
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Impossible d'écrire ce fichier : {e}"))
}

#[tauri::command]
fn import_settings(
    app: tauri::AppHandle,
    settings: State<AppSettings>,
    media: State<Media>,
    path: String,
) -> Result<Bootstrap, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Fichier illisible : {e}"))?;
    let imported: store::Settings =
        serde_json::from_str(&text).map_err(|_| "Ce fichier n'est pas un réglage d'Écho.".to_string())?;

    {
        let mut cfg = settings.0.lock().unwrap();
        *cfg = imported.clone();
        store::save_settings(&config_dir(&app), &cfg);
    }
    media.0.lock().unwrap().set_enabled(imported.media_keys);
    let _ = set_autostart_impl(&app, imported.autostart);
    let snap = scan_with_events(&app);

    Ok(Bootstrap {
        settings: imported,
        snapshot: snap,
        cache_bytes: folder_size(&store::covers_dir(&cache_dir(&app))),
        formats: store::FORMAT_CATALOG.iter().map(|f| f.to_string()).collect(),
    })
}

#[tauri::command]
fn reset_settings(app: tauri::AppHandle, settings: State<AppSettings>, media: State<Media>) -> Bootstrap {
    let fresh = store::Settings::default();
    {
        let mut cfg = settings.0.lock().unwrap();
        *cfg = fresh.clone();
        store::save_settings(&config_dir(&app), &cfg);
    }
    media.0.lock().unwrap().set_enabled(fresh.media_keys);
    let _ = set_autostart_impl(&app, fresh.autostart);
    let snap = scan_with_events(&app);

    Bootstrap {
        settings: fresh,
        snapshot: snap,
        cache_bytes: folder_size(&store::covers_dir(&cache_dir(&app))),
        formats: store::FORMAT_CATALOG.iter().map(|f| f.to_string()).collect(),
    }
}

#[tauri::command]
fn open_config_dir(app: tauri::AppHandle) -> Result<(), String> {
    let dir = config_dir(&app);
    std::process::Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Ecrit ou efface le raccourci de demarrage automatique.
fn set_autostart_impl(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let dir = home.join(".config/autostart");
    let file = dir.join("echo.desktop");
    if !enabled {
        if file.exists() {
            let _ = std::fs::remove_file(&file);
        }
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(&dir);
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Écho\nComment=Musique et vidéos locales\nExec={}\nIcon=echo\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.to_string_lossy()
    );
    std::fs::write(&file, contents).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    set_autostart_impl(&app, enabled)
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// On garde de quoi reprendre la lecture au prochain demarrage.
#[tauri::command]
fn save_progress(
    app: tauri::AppHandle,
    settings: State<AppSettings>,
    track_id: Option<String>,
    position_ms: u64,
    playing: bool,
) {
    let mut cfg = settings.0.lock().unwrap();
    cfg.last_track = track_id;
    cfg.last_position_ms = position_ms;
    cfg.last_playing = playing;
    let snapshot = cfg.clone();
    drop(cfg);
    store::save_settings(&config_dir(&app), &snapshot);
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
        .manage(AppSettings(Mutex::new(store::Settings::default())))
        .setup(|app| {
            let handle = app.handle().clone();

            // Reprise de l'ancienne version (MusicApp) si besoin.
            store::migrate_from_musicapp(&config_dir(&handle), &cache_dir(&handle));

            let settings = store::load_settings(&config_dir(&handle));

            // Touches media du clavier et widget du bureau.
            let mut controls = mpris::Mpris::new(&handle);
            controls.set_enabled(settings.media_keys);
            app.manage(Media(Mutex::new(controls)));

            {
                let state = handle.state::<AppSettings>();
                *state.0.lock().unwrap() = settings.clone();
            }

            {
                let state = handle.state::<AppData>();
                let mut data = state.0.lock().unwrap();

                // Demarrage rapide : on affiche le cache tout de suite.
                if let Some(cache) = store::load_cache(&config_dir(&handle)) {
                    let same_folders = cache.folder == settings.folders.join("\n");
                    if same_folders {
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

            // Puis on rescanne en tache de fond si au moins un dossier existe.
            let needs_scan = settings.refresh_on_start
                && settings.folders.iter().any(|f| Path::new(f).is_dir());
            if needs_scan {
                let handle2 = handle.clone();
                std::thread::spawn(move || {
                    let snap = scan_with_events(&handle2);
                    let _ = handle2.emit("library-updated", &snap);
                });
            }

            // Fermeture de la fenetre : on demande confirmation si le reglage le dit.
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let ask = win
                            .state::<AppSettings>()
                            .0
                            .lock()
                            .map(|s| s.ask_before_quit)
                            .unwrap_or(false);
                        if ask {
                            api.prevent_close();
                            let _ = win.emit("close-requested", ());
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            get_state,
            set_settings,
            add_folder,
            remove_folder,
            rescan,
            clear_cover_cache,
            export_settings,
            import_settings,
            reset_settings,
            open_config_dir,
            set_autostart,
            quit_app,
            save_progress,
            get_cover,
            set_now_playing,
            set_playback
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement d'Écho");
}
