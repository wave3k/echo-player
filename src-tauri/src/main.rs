mod library;
mod mpris;
mod store;

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

/// Corrections des morceaux (titre, artiste, album, pochette), gardées à part des fichiers.
struct AppOverrides(Mutex<store::Overrides>);

/// Controles media du systeme (touches du clavier, widget du bureau).
struct Media(Mutex<mpris::Mpris>);

/// Un seul scan à la fois : deux scans simultanés s'écraseraient l'un l'autre.
struct ScanLock(Mutex<()>);

/// Transforme le chemin du fichier de pochette en URL utilisable par MPRIS.
/// Les caractères spéciaux (espaces, #, ?, %…) doivent être encodés dans une URL.
fn cover_url_for(data: &Data, id: &str) -> Option<String> {
    let path = data.covers.get(id)?;
    let encoded: String = path
        .to_string_lossy()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'_' | b'-' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect();
    Some(format!("file://{}", encoded))
}

#[derive(Serialize)]
struct Snapshot {
    folders: Vec<String>,
    folders_missing: Vec<String>,
    tracks: Vec<Track>,
}

fn snapshot(settings: &store::Settings, data: &Data, overrides: &store::Overrides) -> Snapshot {
    Snapshot {
        folders: settings.folders.clone(),
        folders_missing: settings
            .folders
            .iter()
            .filter(|f| !Path::new(f).is_dir())
            .cloned()
            .collect(),
        tracks: data.tracks.iter().map(|t| with_override(t, overrides)).collect(),
    }
}

/// Le morceau tel qu'il est affiché : les corrections de l'utilisateur par-dessus les tags.
fn with_override(track: &Track, overrides: &store::Overrides) -> Track {
    let mut out = track.clone();
    if let Some(o) = overrides.get(&track.id) {
        if let Some(v) = &o.title {
            out.title = v.clone();
        }
        if let Some(v) = &o.artist {
            out.artist = v.clone();
        }
        if let Some(v) = &o.album {
            out.album = v.clone();
        }
        if let Some(v) = &o.cover {
            out.cover = Some(v.clone());
        }
    }
    out
}

/// Les morceaux du cache encore valables : ceux rangés dans un dossier choisi qui existe
/// toujours, et dans un format coché. Un dossier débranché n'efface pas les autres.
fn tracks_from_cache(cached: Vec<Track>, settings: &store::Settings) -> Vec<Track> {
    let folders: Vec<&Path> = settings
        .folders
        .iter()
        .map(Path::new)
        .filter(|f| f.is_dir())
        .collect();
    cached
        .into_iter()
        .filter(|t| {
            let path = Path::new(&t.path);
            folders.iter().any(|f| path.starts_with(f)) && library::is_listed(path, &settings.formats)
        })
        .collect()
}

/// Le cache disque garde les tags tels qu'ils sont dans les fichiers, jamais les corrections :
/// sinon, au lancement suivant, une correction deviendrait la « valeur d'origine ».
fn cache_of(tracks: &[Track], folders: &[PathBuf]) -> store::Cache {
    store::Cache {
        folder: folders
            .iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        tracks: tracks.to_vec(),
    }
}

fn overrides_of(app: &tauri::AppHandle) -> store::Overrides {
    app.state::<AppOverrides>().0.lock().unwrap().clone()
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

/// Autorise le webview à lire les dossiers de musique et le cache des pochettes.
/// Rien d'autre sur le disque n'est accessible depuis la page.
fn allow_media_paths(app: &tauri::AppHandle, folders: &[String]) {
    let scope = app.asset_protocol_scope();
    for folder in folders {
        let _ = scope.allow_directory(folder, true);
    }
    let _ = scope.allow_directory(store::covers_dir(&cache_dir(app)), true);
}

/// Lance un travail lourd hors du thread principal, pour que la fenêtre reste réactive.
async fn offload<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .expect("tâche de fond interrompue")
}

/// Relit les dossiers choisis et remplit la memoire + le cache disque.
/// La lecture des fichiers se fait sans tenir le verrou de la bibliothèque,
/// pour que les autres commandes ne restent pas bloquées pendant le scan.
fn do_scan(app: &tauri::AppHandle, settings: &store::Settings) -> Snapshot {
    let overrides = overrides_of(app);
    let folders: Vec<PathBuf> = settings
        .folders
        .iter()
        .filter(|f| Path::new(f).is_dir())
        .map(PathBuf::from)
        .collect();

    let state = app.state::<AppData>();

    if folders.is_empty() {
        let mut data = state.0.lock().unwrap();
        data.tracks.clear();
        data.covers.clear();
        return snapshot(settings, &data, &overrides);
    }

    let covers = store::covers_dir(&cache_dir(app));
    // Les morceaux déjà connus servent à éviter de relire les fichiers inchangés.
    let previous: HashMap<String, Track> = state
        .0
        .lock()
        .unwrap()
        .tracks
        .iter()
        .map(|t| (t.id.clone(), t.clone()))
        .collect();
    let (tracks, mut cover_map) = library::scan(&folders, &covers, &settings.formats, &previous);
    // Les pochettes choisies à la main ne sont pas dans les fichiers : on les réinscrit.
    for name in overrides.values().filter_map(|o| o.cover.clone()) {
        cover_map.insert(name.clone(), covers.join(&name));
    }
    allow_media_paths(app, &settings.folders);

    let cache = cache_of(&tracks, &folders);
    let snap = {
        let mut data = state.0.lock().unwrap();
        data.tracks = tracks;
        data.covers = cover_map;
        snapshot(settings, &data, &overrides)
    };

    store::save_cache(&config_dir(app), &cache);

    snap
}

fn scan_with_events(app: &tauri::AppHandle) -> Snapshot {
    // Les réglages sont relus une fois le verrou pris : un scan en attente part
    // des dossiers à jour, pas de ceux du moment où il a été demandé.
    let lock = app.state::<ScanLock>();
    let _guard = lock.0.lock().unwrap();
    let _ = app.emit("scanning", true);
    let settings = app.state::<AppSettings>().0.lock().unwrap().clone();
    let result = do_scan(app, &settings);
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

fn bootstrap_with(app: &tauri::AppHandle, settings: store::Settings, snapshot: Snapshot) -> Bootstrap {
    Bootstrap {
        settings,
        snapshot,
        cache_bytes: folder_size(&store::covers_dir(&cache_dir(app))),
        formats: store::FORMAT_CATALOG.iter().map(|f| f.to_string()).collect(),
    }
}

#[tauri::command]
async fn bootstrap(app: tauri::AppHandle) -> Bootstrap {
    offload(move || {
        let cfg = app.state::<AppSettings>().0.lock().unwrap().clone();
        let overrides = overrides_of(&app);
        let snap = {
            let data = app.state::<AppData>();
            let data = data.0.lock().unwrap();
            snapshot(&cfg, &data, &overrides)
        };
        bootstrap_with(&app, cfg, snap)
    })
    .await
}

#[tauri::command]
fn get_state(state: State<AppData>, settings: State<AppSettings>, overrides: State<AppOverrides>) -> Snapshot {
    let ov = overrides.0.lock().unwrap().clone();
    let data = state.0.lock().unwrap();
    let cfg = settings.0.lock().unwrap();
    snapshot(&cfg, &data, &ov)
}

/// Enregistre les reglages et applique ce qui doit l'etre tout de suite.
/// Renvoie la bibliothèque seulement si elle a été relue (formats changés) : sinon,
/// rien n'a bougé et inutile de renvoyer des milliers de morceaux.
#[tauri::command]
async fn set_settings(app: tauri::AppHandle, mut new_settings: store::Settings) -> Option<Snapshot> {
    new_settings.sanitize();
    let formats_changed = {
        let settings = app.state::<AppSettings>();
        let mut current = settings.0.lock().unwrap();
        new_settings.keep_core_from(&current);
        let formats_changed = current.formats != new_settings.formats;
        *current = new_settings.clone();
        formats_changed
    };

    store::save_settings(&config_dir(&app), &new_settings);
    app.state::<Media>()
        .0
        .lock()
        .unwrap()
        .set_enabled(new_settings.media_keys);
    let _ = set_autostart_impl(&app, new_settings.autostart);

    if formats_changed {
        Some(offload(move || scan_with_events(&app)).await)
    } else {
        None
    }
}

#[tauri::command]
async fn add_folder(app: tauri::AppHandle, path: String) -> Result<Snapshot, String> {
    if !Path::new(&path).is_dir() {
        return Err("Ce dossier n'existe pas.".into());
    }
    {
        let settings = app.state::<AppSettings>();
        let mut cfg = settings.0.lock().unwrap();
        if !cfg.folders.iter().any(|f| f == &path) {
            cfg.folders.push(path.clone());
        }
        store::save_settings(&config_dir(&app), &cfg);
    }
    allow_media_paths(&app, std::slice::from_ref(&path));
    Ok(offload(move || scan_with_events(&app)).await)
}

#[tauri::command]
async fn remove_folder(app: tauri::AppHandle, path: String) -> Snapshot {
    {
        let settings = app.state::<AppSettings>();
        let mut cfg = settings.0.lock().unwrap();
        cfg.folders.retain(|f| f != &path);
        store::save_settings(&config_dir(&app), &cfg);
    }
    offload(move || scan_with_events(&app)).await
}

#[tauri::command]
async fn rescan(app: tauri::AppHandle) -> Snapshot {
    offload(move || scan_with_events(&app)).await
}

#[derive(Serialize)]
struct Cleared {
    cache_bytes: u64,
    snapshot: Snapshot,
}

/// Efface les pochettes mises en cache puis relit les fichiers pour les reconstruire.
#[tauri::command]
async fn clear_cover_cache(app: tauri::AppHandle) -> Cleared {
    offload(move || {
        let dir = store::covers_dir(&cache_dir(&app));
        // Les pochettes choisies à la main restent : elles ne sont pas une copie à reconstruire.
        let keep: std::collections::HashSet<String> =
            overrides_of(&app).values().filter_map(|o| o.cover.clone()).collect();
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if !keep.contains(&name) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let state = app.state::<AppData>();
        let mut data = state.0.lock().unwrap();
        data.covers.clear();
        for name in &keep {
            data.covers.insert(name.clone(), dir.join(name));
        }
        for track in data.tracks.iter_mut() {
            track.cover = None;
            // Date effacée : le scan qui suit relit les tags et ré-extrait les pochettes.
            track.mtime = 0;
        }
        drop(data);
        let snapshot = scan_with_events(&app);
        Cleared {
            cache_bytes: folder_size(&dir),
            snapshot,
        }
    })
    .await
}

#[tauri::command]
fn export_settings(settings: State<AppSettings>, path: String) -> Result<(), String> {
    let cfg = settings.0.lock().unwrap().clone();
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Impossible d'écrire ce fichier : {e}"))
}

#[tauri::command]
async fn import_settings(app: tauri::AppHandle, path: String) -> Result<Bootstrap, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Fichier illisible : {e}"))?;
    let mut imported: store::Settings =
        serde_json::from_str(&text).map_err(|_| "Ce fichier n'est pas un réglage d'Écho.".to_string())?;
    imported.sanitize();

    {
        let settings = app.state::<AppSettings>();
        let mut cfg = settings.0.lock().unwrap();
        *cfg = imported.clone();
        store::save_settings(&config_dir(&app), &cfg);
    }
    app.state::<Media>()
        .0
        .lock()
        .unwrap()
        .set_enabled(imported.media_keys);
    let _ = set_autostart_impl(&app, imported.autostart);

    let handle = app.clone();
    let snap = offload(move || scan_with_events(&handle)).await;
    Ok(bootstrap_with(&app, imported, snap))
}

#[tauri::command]
async fn reset_settings(app: tauri::AppHandle) -> Bootstrap {
    let fresh = store::Settings::default();
    {
        let settings = app.state::<AppSettings>();
        let mut cfg = settings.0.lock().unwrap();
        *cfg = fresh.clone();
        store::save_settings(&config_dir(&app), &cfg);
    }
    app.state::<Media>()
        .0
        .lock()
        .unwrap()
        .set_enabled(fresh.media_keys);
    let _ = set_autostart_impl(&app, fresh.autostart);

    let handle = app.clone();
    let snap = offload(move || scan_with_events(&handle)).await;
    bootstrap_with(&app, fresh, snap)
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
    // Dans un AppImage, current_exe() pointe vers un dossier temporaire qui change
    // à chaque lancement : le vrai fichier est donné par APPIMAGE.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe().map_err(|e| e.to_string())?,
    };
    let _ = std::fs::create_dir_all(&dir);
    // Exec entre guillemets : un chemin avec des espaces doit rester un seul argument.
    let exec = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Écho\nComment=Lecteur de musique locale\nExec=\"{exec}\"\nIcon=echo\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
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

/// Valeurs d'origine d'un morceau, lues dans ses tags (pour rétablir les valeurs).
#[derive(Serialize)]
struct Original {
    title: String,
    artist: String,
    album: String,
    cover: Option<String>,
}

#[tauri::command]
fn track_original(state: State<AppData>, id: String) -> Option<Original> {
    let data = state.0.lock().unwrap();
    data.tracks.iter().find(|t| t.id == id).map(|t| Original {
        title: t.title.clone(),
        artist: t.artist.clone(),
        album: t.album.clone(),
        cover: t.cover.clone(),
    })
}

/// Garde la valeur saisie seulement si elle diffère de l'original (vide = original).
fn differs(value: String, original: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v == original {
        None
    } else {
        Some(v.to_string())
    }
}

/// Copie une image choisie dans le cache des pochettes et renvoie son nom.
fn store_cover(covers: &Path, source: &str) -> Result<String, String> {
    // La taille est vérifiée avant de lire : un énorme fichier ne passe pas en mémoire.
    let meta = std::fs::metadata(source).map_err(|e| format!("Image illisible : {e}"))?;
    if !meta.is_file() || meta.len() > 20 * 1024 * 1024 {
        return Err("Cette image est trop lourde (20 Mo au maximum).".into());
    }
    let bytes = std::fs::read(source).map_err(|e| format!("Image illisible : {e}"))?;
    let ext = match Path::new(source)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "png",
        Some("gif") => "gif",
        Some("webp") => "webp",
        _ => "jpg",
    };
    let name = format!("{:016x}.{}", library::stable_hash(&bytes), ext);
    let dest = covers.join(&name);
    if !dest.exists() {
        std::fs::write(&dest, &bytes).map_err(|e| format!("Impossible d'enregistrer la pochette : {e}"))?;
    }
    Ok(name)
}

fn apply_edit(
    app: &tauri::AppHandle,
    id: String,
    title: String,
    artist: String,
    album: String,
    cover_path: Option<String>,
    clear_cover: bool,
) -> Result<Track, String> {
    let original = {
        let state = app.state::<AppData>();
        let data = state.0.lock().unwrap();
        data.tracks
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .ok_or_else(|| "Ce morceau n'est plus dans la bibliothèque.".to_string())?
    };
    let covers = store::covers_dir(&cache_dir(app));
    let new_cover = match cover_path {
        Some(path) => Some(store_cover(&covers, &path)?),
        None => None,
    };

    let (overrides, cover_now) = {
        let state = app.state::<AppOverrides>();
        let mut map = state.0.lock().unwrap();
        let mut entry = map.get(&id).cloned().unwrap_or_default();
        entry.title = differs(title, &original.title);
        entry.artist = differs(artist, &original.artist);
        entry.album = differs(album, &original.album);
        if clear_cover {
            entry.cover = None;
        }
        if let Some(name) = new_cover {
            entry.cover = Some(name);
        }
        let cover_now = entry.cover.clone();
        if entry.title.is_none() && entry.artist.is_none() && entry.album.is_none() && entry.cover.is_none() {
            map.remove(&id);
        } else {
            map.insert(id, entry);
        }
        store::save_overrides(&config_dir(app), &map);
        (map.clone(), cover_now)
    };

    let state = app.state::<AppData>();
    let mut data = state.0.lock().unwrap();
    if let Some(name) = &cover_now {
        data.covers.insert(name.clone(), covers.join(name));
    }
    // Seul le morceau modifié repart vers l'interface, pas toute la bibliothèque.
    Ok(with_override(&original, &overrides))
}

#[tauri::command]
async fn edit_track(
    app: tauri::AppHandle,
    id: String,
    title: String,
    artist: String,
    album: String,
    cover_path: Option<String>,
    clear_cover: bool,
) -> Result<Track, String> {
    offload(move || apply_edit(&app, id, title, artist, album, cover_path, clear_cover)).await
}

/// Chemin de la pochette sur le disque. L'interface la charge via le protocole
/// d'asset : rien n'est copié en mémoire ni encodé en base64.
#[tauri::command]
fn get_cover_path(state: State<AppData>, id: String) -> Option<String> {
    let data = state.0.lock().unwrap();
    data.covers
        .get(&id)
        .map(|p| p.to_string_lossy().to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppData(Mutex::new(Data::default())))
        .manage(AppSettings(Mutex::new(store::Settings::default())))
        .manage(AppOverrides(Mutex::new(store::Overrides::new())))
        .manage(ScanLock(Mutex::new(())))
        .setup(|app| {
            let handle = app.handle().clone();

            // Reprise de l'ancienne version (MusicApp) si besoin.
            store::migrate_from_musicapp(&config_dir(&handle), &cache_dir(&handle));

            let settings = store::load_settings(&config_dir(&handle));
            let overrides = store::load_overrides(&config_dir(&handle));
            *handle.state::<AppOverrides>().0.lock().unwrap() = overrides.clone();

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
                    data.tracks = tracks_from_cache(cache.tracks, &settings);
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

            allow_media_paths(&handle, &settings.folders);

            // Met à jour le chemin du démarrage automatique (ancien nom de binaire, AppImage…).
            if settings.autostart {
                let _ = set_autostart_impl(&handle, true);
            }

            // Puis on rescanne en tache de fond si au moins un dossier existe.
            // Pochettes choisies à la main : disponibles dès l'ouverture.
            {
                let covers = store::covers_dir(&cache_dir(&handle));
                let state = handle.state::<AppData>();
                let mut data = state.0.lock().unwrap();
                for name in overrides.values().filter_map(|o| o.cover.clone()) {
                    data.covers.insert(name.clone(), covers.join(&name));
                }
            }

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
            get_cover_path,
            track_original,
            edit_track,
            set_now_playing,
            set_playback
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement d'Écho");
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use library::tests::TempDir;

    fn track(path: &str, title: &str) -> Track {
        Track {
            id: path.into(),
            path: path.into(),
            title: title.into(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            cover: None,
            mtime: 1,
            size: 1,
        }
    }

    /// Bug de l'audit : un dossier débranché vidait toute la bibliothèque au démarrage.
    #[test]
    fn un_dossier_manquant_n_efface_pas_le_cache_des_autres() {
        let tmp = TempDir::new("cache");
        let present = tmp.0.join("musique");
        std::fs::create_dir_all(&present).unwrap();
        let mut settings = store::Settings::default();
        settings.folders = vec![
            present.to_string_lossy().into(),
            tmp.0.join("absent").to_string_lossy().into(),
        ];
        let inside = present.join("a.mp3").to_string_lossy().to_string();
        let gone = tmp.0.join("absent/b.mp3").to_string_lossy().to_string();
        let elsewhere = "/ailleurs/c.mp3".to_string();

        let kept = tracks_from_cache(
            vec![track(&inside, "A"), track(&gone, "B"), track(&elsewhere, "C")],
            &settings,
        );
        let titles: Vec<&str> = kept.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["A"]);
    }

    /// Bug de l'audit : le cache gardait les corrections, qui devenaient « l'original ».
    #[test]
    fn le_cache_garde_les_tags_d_origine() {
        let raw = vec![track("/m/a.mp3", "Titre du fichier")];
        let mut overrides = store::Overrides::new();
        overrides.insert(
            "/m/a.mp3".into(),
            store::TrackOverride { title: Some("Titre corrigé".into()), ..Default::default() },
        );

        let shown = with_override(&raw[0], &overrides);
        assert_eq!(shown.title, "Titre corrigé");

        let cache = cache_of(&raw, &[PathBuf::from("/m")]);
        assert_eq!(cache.tracks[0].title, "Titre du fichier");
    }

    #[test]
    fn une_valeur_identique_a_l_original_n_est_pas_une_correction() {
        assert_eq!(differs("  Titre ".into(), "Titre"), None);
        assert_eq!(differs(String::new(), "Titre"), None);
        assert_eq!(differs("Autre".into(), "Titre").as_deref(), Some("Autre"));
    }

    #[test]
    fn une_image_trop_lourde_est_refusee_sans_etre_lue() {
        let tmp = TempDir::new("cover");
        let big = tmp.0.join("big.png");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(21 * 1024 * 1024).unwrap();
        assert!(store_cover(&tmp.0, &big.to_string_lossy()).is_err());

        let small = tmp.0.join("small.PNG");
        std::fs::write(&small, b"png").unwrap();
        let name = store_cover(&tmp.0, &small.to_string_lossy()).unwrap();
        assert!(name.ends_with(".png"));
        assert!(tmp.0.join(name).is_file());
    }
}
