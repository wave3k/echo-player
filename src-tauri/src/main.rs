// Pas de console à côté de la fenêtre sous Windows (version publiée).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod audio_only;
mod discord;
#[cfg(target_os = "linux")]
mod frames;
mod library;
mod lyrics;
mod matroska;
mod mp4;
mod mpris;
#[cfg_attr(not(target_os = "linux"), path = "player_stub.rs")]
mod player;
mod store;
mod thumbs;
mod titles;
mod updates;
mod watch;

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

/// Ce qui tourne en arrière-plan selon les réglages : surveillance des dossiers,
/// icône de la barre du système, statut Discord.
struct Services {
    watch: Mutex<Option<watch::FolderWatch>>,
    /// Dossiers surveillés (vide : rien) ; la surveillance n'est refaite que s'ils changent.
    watch_key: Mutex<String>,
    tray: Mutex<Option<tauri::tray::TrayIcon>>,
    discord: discord::Discord,
    /// Ce qui joue, pour Discord : titres d'un côté, position de l'autre.
    now: Mutex<Option<discord::NowPlaying>>,
}

/// Met en route ou arrête les services d'arrière-plan d'après les réglages.
fn apply_services(app: &tauri::AppHandle) {
    let settings = app.state::<AppSettings>().0.lock().unwrap().clone();
    let services = app.state::<Services>();

    let key = if settings.watch { settings.folders.join("\n") } else { String::new() };
    let mut current = services.watch_key.lock().unwrap();
    if *current != key {
        let mut slot = services.watch.lock().unwrap();
        *slot = None;
        if settings.watch {
            let handle = app.clone();
            *slot = watch::FolderWatch::start(&settings.folders, move || {
                let snap = scan_with_events(&handle);
                let _ = handle.emit("library-updated", &snap);
            });
        }
        *current = key;
    }
    drop(current);

    services.discord.set_enabled(settings.discord);

    // L'icône de la barre du système se crée sur le fil principal (GTK).
    let handle = app.clone();
    let want = settings.tray;
    let _ = app.run_on_main_thread(move || set_tray(&handle, want));
}

/// Affiche la fenêtre et la met au premier plan (la recrée si elle a été fermée
/// pendant qu'Écho jouait depuis la barre du système).
fn show_window(app: &tauri::AppHandle) {
    let window = match app.get_webview_window("main") {
        Some(window) => window,
        None => {
            let Some(config) = app.config().app.windows.first().cloned() else { return };
            match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
                Ok(window) => {
                    attach_window(&window);
                    IN_TRAY.store(false, std::sync::atomic::Ordering::SeqCst);
                    window
                }
                Err(_) => return,
            }
        }
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Touche média, menu de l'icône ou widget du bureau : le lecteur du cœur s'en charge,
/// sinon la page (qui lit alors la musique elle-même).
fn media_control(app: &tauri::AppHandle, action: &str) {
    if !player::media_control(app, action) {
        let _ = app.emit("media-control", action);
    }
}

fn set_tray(app: &tauri::AppHandle, want: bool) {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let services = app.state::<Services>();
    let mut slot = services.tray.lock().unwrap();
    if !want {
        if let Some(tray) = slot.take() {
            let _ = tray.set_visible(false);
        }
        return;
    }
    if slot.is_some() {
        return;
    }
    let build = || -> tauri::Result<tauri::tray::TrayIcon> {
        let menu = Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, "toggle", "Lecture / pause", true, None::<&str>)?,
                &MenuItem::with_id(app, "next", "Suivant", true, None::<&str>)?,
                &MenuItem::with_id(app, "previous", "Précédent", true, None::<&str>)?,
                &PredefinedMenuItem::separator(app)?,
                &MenuItem::with_id(app, "show", "Afficher Écho", true, None::<&str>)?,
                &MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?,
            ],
        )?;
        let mut builder = TrayIconBuilder::with_id("echo")
            .tooltip("Écho")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "show" => show_window(app),
                "quit" => app.exit(0),
                action => media_control(app, action),
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    show_window(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(app)
    };
    // Sans bibliothèque d'icônes de barre (appindicator), on continue sans icône.
    *slot = build().ok();
}

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
        .map(|mut t| {
            if t.cover.as_deref().is_some_and(library::is_stale_frame) {
                t.cover = None;
            }
            t
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
    let out = tauri::async_runtime::spawn_blocking(move || {
        let out = f();
        release_memory();
        out
    })
    .await
    .expect("tâche de fond interrompue");
    out
}

/// Rend au système la mémoire libérée par un travail lourd (scan, copie audio, image
/// d'une vidéo). Sans ça, l'allocateur de glibc la garde pour lui : des dizaines de Mo.
fn release_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Deux réserves de mémoire en tout, au lieu d'une par thread : moins de mémoire
/// éparpillée, sans effet sensible pour une app de ce genre.
fn limit_allocator() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
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
    release_memory();
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
    /// Faux tant que l'identifiant de l'application Discord n'est pas renseigné.
    discord_ready: bool,
    /// Version d'Écho (À propos, mises à jour).
    version: String,
    /// Vrai : la musique est lue par le cœur (Linux), la page n'est qu'un écran.
    native_audio: bool,
}

fn bootstrap_with(app: &tauri::AppHandle, settings: store::Settings, snapshot: Snapshot) -> Bootstrap {
    Bootstrap {
        settings,
        snapshot,
        cache_bytes: cache_size(app),
        formats: store::FORMAT_CATALOG.iter().map(|f| f.to_string()).collect(),
        discord_ready: discord::app_id().is_some(),
        version: app.package_info().version.to_string(),
        native_audio: player::available(app),
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
    apply_services(&app);
    player::configure(&app, &new_settings);

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
    apply_services(&app);
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
    apply_services(&app);
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

/// Place prise par le cache : pochettes et copies audio des clips.
fn cache_size(app: &tauri::AppHandle) -> u64 {
    let cache = cache_dir(app);
    folder_size(&store::covers_dir(&cache))
        + folder_size(&thumbs::dir(&store::covers_dir(&cache)))
        + folder_size(&cache.join("audio"))
}

/// Efface les pochettes mises en cache puis relit les fichiers pour les reconstruire.
#[tauri::command]
async fn clear_cover_cache(app: tauri::AppHandle) -> Cleared {
    offload(move || {
        let dir = store::covers_dir(&cache_dir(&app));
        // Les copies audio des clips se refont à la demande.
        let _ = std::fs::remove_dir_all(cache_dir(&app).join("audio"));
        let _ = std::fs::remove_dir_all(thumbs::dir(&dir));
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
            cache_bytes: cache_size(&app),
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
    apply_services(&app);

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
    apply_services(&app);

    let handle = app.clone();
    let snap = offload(move || scan_with_events(&handle)).await;
    bootstrap_with(&app, fresh, snap)
}

#[tauri::command]
fn open_config_dir(app: tauri::AppHandle) -> Result<(), String> {
    let dir = config_dir(&app);
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let opener = "xdg-open";
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Démarrage automatique sous Windows : une valeur dans la clé « Run » de l'utilisateur.
#[cfg(target_os = "windows")]
fn set_autostart_impl(_app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut cmd = std::process::Command::new("reg");
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        cmd.args(["add", KEY, "/v", "Echo", "/t", "REG_SZ", "/f", "/d"])
            .arg(format!("\"{}\"", exe.display()));
    } else {
        cmd.args(["delete", KEY, "/v", "Echo", "/f"]);
    }
    {
        use std::os::windows::process::CommandExt;
        // Pas de fenêtre de console qui clignote.
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd.output().map_err(|e| e.to_string())?;
    Ok(())
}

/// Ecrit ou efface le raccourci de demarrage automatique.
#[cfg(not(target_os = "windows"))]
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
    // Une mise à jour en attente s'installe à la sortie (voir RunEvent::Exit).
    app.exit(0);
}

/// Paroles du morceau : sur l'ordinateur d'abord, puis en ligne si le réglage le permet
/// (ou si l'interface le demande explicitement avec `online`).
#[tauri::command]
async fn get_lyrics(app: tauri::AppHandle, id: String, online: bool) -> lyrics::Lyrics {
    let track = {
        let state = app.state::<AppData>();
        let data = state.0.lock().unwrap();
        data.tracks.iter().find(|t| t.id == id).cloned()
    };
    let Some(track) = track else { return lyrics::Lyrics::none() };
    let track = with_override(&track, &overrides_of(&app));
    let path = PathBuf::from(&track.path);
    if let Some(mut found) = offload(move || lyrics::local(&path)).await {
        found.anchors = store::load_lyrics_sync(&config_dir(&app)).remove(&id).unwrap_or_default();
        return found;
    }
    let allowed = online || app.state::<AppSettings>().0.lock().map(|s| s.lyrics_online).unwrap_or(false);
    if !allowed || track.title.trim().is_empty() {
        return lyrics::Lyrics::none();
    }
    let query = lyrics::Query {
        artist: track.artist.clone(),
        title: track.title.clone(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        video: track.video,
    };
    let mut found = lyrics::online(&cache_dir(&app), &query).await;
    found.anchors = store::load_lyrics_sync(&config_dir(&app)).remove(&id).unwrap_or_default();
    found
}

/// Garde les calages faits à la main pour ce morceau (liste vide : on les oublie).
#[tauri::command]
async fn save_lyrics_sync(app: tauri::AppHandle, id: String, anchors: Vec<lyrics::Anchor>) -> Result<(), String> {
    offload(move || {
        let dir = config_dir(&app);
        let mut map = store::load_lyrics_sync(&dir);
        let mut anchors = anchors;
        anchors.sort_by_key(|a| a.at_ms);
        anchors.truncate(64);
        if anchors.is_empty() {
            map.remove(&id);
        } else {
            map.insert(id, anchors);
        }
        store::save_lyrics_sync(&dir, &map).map_err(|e| e.to_string())
    })
    .await
}

/// Fichier à donner au lecteur audio. Linux : un clip est remplacé par sa piste audio
/// seule (sinon WebKit décode aussi la vidéo). Ailleurs, ou en cas d'échec : l'original.
#[tauri::command]
async fn audio_source(app: tauri::AppHandle, id: String) -> Option<String> {
    let track = {
        let state = app.state::<AppData>();
        let data = state.0.lock().unwrap();
        data.tracks.iter().find(|t| t.id == id).cloned()
    }?;
    #[cfg(target_os = "linux")]
    if track.video {
        let cache = cache_dir(&app);
        let path = PathBuf::from(&track.path);
        let mtime = track.mtime;
        if let Some(audio) = offload(move || audio_only::ensure(&path, mtime, &cache)).await {
            return Some(audio.to_string_lossy().into_owned());
        }
    }
    Some(track.path)
}

/// Cherche une nouvelle version (bouton « Vérifier maintenant »).
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> updates::Status {
    updates::check(&app).await
}

/// Télécharge et installe la version trouvée ; elle s'applique au redémarrage.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> updates::Status {
    updates::install(&app).await
}

/// Relance Écho sur la version installée.
#[tauri::command]
fn restart_to_update(app: tauri::AppHandle) {
    updates::restart(&app);
}

/// Ouvre la page des versions dans le navigateur (paquet .deb : mise à jour manuelle).
#[tauri::command]
fn open_releases() {
    open_url(updates::RELEASES_URL);
}

fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// On garde de quoi reprendre la lecture au prochain demarrage.
#[tauri::command]
fn save_progress(app: tauri::AppHandle, track_id: Option<String>, position_ms: u64, playing: bool) {
    save_progress_impl(&app, track_id, position_ms, playing);
}

fn save_progress_impl(app: &tauri::AppHandle, track_id: Option<String>, position_ms: u64, playing: bool) {
    let settings = app.state::<AppSettings>();
    let mut cfg = settings.0.lock().unwrap();
    cfg.last_track = track_id;
    cfg.last_position_ms = position_ms;
    cfg.last_playing = playing;
    let snapshot = cfg.clone();
    drop(cfg);
    store::save_settings(&config_dir(app), &snapshot);
}

/// Envoie à Discord ce qui joue, après une mise à jour du titre ou de la position.
fn push_discord(services: &Services, change: impl FnOnce(&mut Option<discord::NowPlaying>)) {
    let mut now = services.now.lock().unwrap();
    change(&mut now);
    services.discord.update(now.clone());
}

#[tauri::command]
fn set_now_playing(
    app: tauri::AppHandle,
    title: String,
    artist: String,
    album: String,
    cover: Option<String>,
    duration_ms: u64,
) {
    now_playing(&app, title, artist, album, cover, duration_ms);
}

/// Le morceau du lecteur du cœur vient de démarrer : touches média, Discord, notification.
fn announce_track(app: &tauri::AppHandle, track: &Track) {
    let track = with_override(track, &overrides_of(app));
    now_playing(app, track.title.clone(), track.artist.clone(), track.album.clone(), track.cover.clone(), track.duration_ms);
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if !focused {
        notify_track(app.clone(), track.title.clone(), if track.artist.is_empty() { "Écho".into() } else { track.artist.clone() });
    }
}

fn now_playing(app: &tauri::AppHandle, title: String, artist: String, album: String, cover: Option<String>, duration_ms: u64) {
    let state = app.state::<AppData>();
    let media = app.state::<Media>();
    let services = app.state::<Services>();
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
    drop(controls);

    push_discord(&services, |now| {
        *now = if title.is_empty() {
            None
        } else {
            let playing = now.as_ref().map(|n| n.playing).unwrap_or(false);
            Some(discord::NowPlaying { title, artist, album, position_ms: 0, duration_ms, playing })
        };
    });
}

#[tauri::command]
fn set_playback(app: tauri::AppHandle, playing: bool, position_ms: u64) {
    playback_changed(&app, playing, position_ms);
}

fn playback_changed(app: &tauri::AppHandle, playing: bool, position_ms: u64) {
    let media = app.state::<Media>();
    let services = app.state::<Services>();
    let mut controls = media.0.lock().unwrap();
    controls.set_playback(
        playing,
        Some(std::time::Duration::from_millis(position_ms)),
    );
    drop(controls);
    push_discord(&services, |now| {
        if let Some(n) = now {
            n.playing = playing;
            n.position_ms = position_ms;
        }
    });
}

/// Notification du système au changement de morceau (si le réglage est actif).
#[tauri::command]
fn notify_track(app: tauri::AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    let on = app.state::<AppSettings>().0.lock().map(|s| s.notify).unwrap_or(false);
    if on {
        let mut note = app.notification().builder().title(title).body(body);
        if let Some(icon) = notify_icon(&app) {
            note = note.icon(icon);
        }
        let _ = note.show();
    }
}

/// Logo d'Écho pour les notifications : un fichier, pour qu'il s'affiche quel que soit
/// le mode d'installation (paquet, AppImage, script, ou pas installé du tout).
fn notify_icon(app: &tauri::AppHandle) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let path = cache_dir(app).join("echo-icon.png");
    if !path.is_file() {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, include_bytes!("../icons/128x128@2x.png")).ok()?;
    }
    Some(path.to_string_lossy().into_owned())
}

/// Enregistre l'image tirée d'une vidéo (JPEG) comme pochette du morceau, ou note
/// qu'elle n'en a pas (`None`). Renvoie le morceau à jour.
fn store_frame(app: &tauri::AppHandle, id: &str, jpeg: Option<Vec<u8>>) -> Option<Track> {
    let covers = store::covers_dir(&cache_dir(app));
    let state = app.state::<AppData>();
    let mut data = state.0.lock().unwrap();
    let index = data.tracks.iter().position(|t| t.id == id)?;
    let mtime = data.tracks[index].mtime;
    match jpeg.filter(|b| b.len() > 100 && b.len() < 8 * 1024 * 1024) {
        Some(bytes) => {
            let name = library::frame_name(id, mtime);
            std::fs::write(covers.join(&name), bytes).ok()?;
            data.covers.insert(name.clone(), covers.join(&name));
            data.tracks[index].cover = Some(name);
        }
        None => {
            let _ = std::fs::write(covers.join(library::no_frame_marker(id, mtime)), b"");
            data.tracks[index].no_frame = true;
        }
    }
    let track = data.tracks[index].clone();
    let folders: Vec<PathBuf> = app
        .state::<AppSettings>()
        .0
        .lock()
        .unwrap()
        .folders
        .iter()
        .map(PathBuf::from)
        .filter(|f| f.is_dir())
        .collect();
    let cache = cache_of(&data.tracks, &folders);
    drop(data);
    store::save_cache(&config_dir(app), &cache);
    Some(with_override(&track, &overrides_of(app)))
}

/// Image tirée d'une vidéo par l'interface (Windows, macOS).
#[tauri::command]
async fn save_frame(app: tauri::AppHandle, id: String, jpeg: Option<Vec<u8>>) -> Option<Track> {
    offload(move || store_frame(&app, &id, jpeg)).await
}

#[derive(Serialize)]
struct Grabbed {
    track: Option<Track>,
    /// La vidéo a une image mais le système ne sait pas la décoder.
    missing: bool,
}

/// Linux : l'image est décodée ici, par GStreamer, sans passer par la page.
#[tauri::command]
async fn grab_frame(app: tauri::AppHandle, id: String) -> Result<Grabbed, String> {
    #[cfg(target_os = "linux")]
    {
        offload(move || {
            let path = {
                let state = app.state::<AppData>();
                let data = state.0.lock().unwrap();
                data.tracks.iter().find(|t| t.id == id).map(|t| PathBuf::from(&t.path))
            };
            let Some(path) = path else {
                return Ok(Grabbed { track: None, missing: false });
            };
            Ok(match frames::grab(&path) {
                frames::Frame::Image(bytes) => Grabbed { track: store_frame(&app, &id, Some(bytes)), missing: false },
                frames::Frame::None => Grabbed { track: store_frame(&app, &id, None), missing: false },
                frames::Frame::Undecodable => Grabbed { track: None, missing: true },
            })
        })
        .await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, id);
        Err("indisponible".into())
    }
}

/// Durée trouvée par le lecteur quand les tags n'en donnaient pas.
#[tauri::command]
fn set_duration(state: State<AppData>, id: String, duration_ms: u64) {
    let mut data = state.0.lock().unwrap();
    if let Some(track) = data.tracks.iter_mut().find(|t| t.id == id && t.duration_ms == 0) {
        track.duration_ms = duration_ms;
    }
}

#[tauri::command]
fn get_playlists(app: tauri::AppHandle) -> Vec<store::Playlist> {
    store::load_playlists(&config_dir(&app))
}

#[tauri::command]
fn save_playlists(app: tauri::AppHandle, playlists: Vec<store::Playlist>) -> Vec<store::Playlist> {
    let clean = store::sanitize_playlists(playlists);
    store::save_playlists(&config_dir(&app), &clean);
    clean
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
async fn get_cover_path(app: tauri::AppHandle, id: String) -> Option<CoverPaths> {
    let path = app.state::<AppData>().0.lock().unwrap().covers.get(&id).cloned()?;
    Some(offload(move || CoverPaths::of(&path)).await)
}

/// Une pochette : l'image d'origine (grands affichages) et sa vignette (listes).
#[derive(Serialize)]
struct CoverPaths {
    full: String,
    thumb: Option<String>,
}

impl CoverPaths {
    fn of(path: &Path) -> Self {
        Self {
            full: path.to_string_lossy().to_string(),
            thumb: thumbs::ensure(path).map(|t| t.to_string_lossy().to_string()),
        }
    }
}

/// Toutes les pochettes connues d'un coup (identifiant → chemin) : l'interface les
/// pose tout de suite au défilement, sans une demande par ligne.
/// Les vignettes manquantes sont faites ici, hors du verrou de la bibliothèque.
#[tauri::command]
async fn get_cover_paths(app: tauri::AppHandle) -> std::collections::HashMap<String, CoverPaths> {
    let covers: Vec<(String, PathBuf)> = {
        let state = app.state::<AppData>();
        let data = state.0.lock().unwrap();
        data.covers.iter().map(|(id, p)| (id.clone(), p.clone())).collect()
    };
    offload(move || covers.into_iter().map(|(id, p)| (id, CoverPaths::of(&p))).collect()).await
}

/// Réglages du moteur d'affichage à poser avant d'ouvrir la fenêtre.
fn prepare_webview_env() {
    // Pilote NVIDIA propriétaire : le rendu DMA-BUF de WebKitGTK donne souvent une
    // fenêtre vide. Le rendu classique marche partout ; un réglage déjà posé est respecté.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        && Path::new("/proc/driver/nvidia/version").exists()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

/// Moins de mémoire pour le moteur d'affichage : Écho n'est pas un navigateur,
/// inutile de garder en cache les pages déjà vues ni de précharger quoi que ce soit.
fn lighten_webview(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "linux")]
    let _ = window.with_webview(|webview| {
        use webkit2gtk::{CacheModel, SecurityManagerExt, WebContextExt, WebViewExt};
        if let Some(context) = webview.inner().context() {
            context.set_cache_model(CacheModel::DocumentViewer);
            // Le lecteur audio de WebKitGTK (GStreamer) refuse les adresses asset:// :
            // sous Linux, les morceaux sont lus en file://, ce que la page de l'app
            // (tauri://) n'a le droit de faire que si son schéma est déclaré local.
            if let Some(security) = context.security_manager() {
                security.register_uri_scheme_as_local("tauri");
            }
        }
    });
    #[cfg(not(target_os = "linux"))]
    let _ = window;
}

/// Réglages et comportement de la fenêtre (au démarrage, et quand elle est recréée).
fn attach_window(window: &tauri::WebviewWindow) {
    lighten_webview(window);
    let win = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let (ask, to_tray) = win
                .state::<AppSettings>()
                .0
                .lock()
                .map(|s| (s.ask_before_quit, s.close_to_tray))
                .unwrap_or((false, false));
            if to_tray && tray_shown(win.app_handle()) {
                if player::available(win.app_handle()) {
                    // La musique est lue par le cœur : la page est fermée pour de bon
                    // (≈ 200 Mo rendus), et recréée quand on rouvre Écho.
                    IN_TRAY.store(true, std::sync::atomic::Ordering::SeqCst);
                    // WebKit garderait son processus d'affichage en réserve : on l'arrête.
                    #[cfg(target_os = "linux")]
                    let _ = win.with_webview(|webview| {
                        use webkit2gtk::WebViewExt;
                        webview.inner().terminate_web_process();
                    });
                    let _ = win.destroy();
                    // La page est démontée un peu après : la mémoire est rendue ensuite.
                    std::thread::spawn(|| {
                        for _ in 0..3 {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            release_memory();
                        }
                    });
                } else {
                    api.prevent_close();
                    let _ = win.hide();
                }
            } else if ask {
                api.prevent_close();
                let _ = win.emit("close-requested", ());
            }
        }
    });
}

/// Vrai pendant qu'Écho joue sans fenêtre, depuis la barre du système.
static IN_TRAY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn tray_shown(app: &tauri::AppHandle) -> bool {
    app.state::<Services>().tray.lock().map(|t| t.is_some()).unwrap_or(false)
}

fn main() {
    limit_allocator();
    prepare_webview_env();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Updates::default())
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

            app.manage(Services {
                watch: Mutex::new(None),
                watch_key: Mutex::new(String::new()),
                tray: Mutex::new(None),
                discord: discord::Discord::start(settings.discord),
                now: Mutex::new(None),
            });

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

            // Copies audio des clips : le cache reste sous sa limite.
            #[cfg(target_os = "linux")]
            {
                let cache = cache_dir(&handle);
                std::thread::spawn(move || audio_only::prune(&cache));
            }

            // Mises à jour : vérifiées en fond, selon le réglage du moment.
            updates::start_background(handle.clone(), |app| {
                app.state::<AppSettings>().0.lock().map(|s| s.updates.clone()).unwrap_or_else(|_| "off".into())
            });

            // Images de vidéos des anciennes versions (souvent noires) : refaites au besoin.
            if let Ok(read) = std::fs::read_dir(store::covers_dir(&cache_dir(&handle))) {
                for entry in read.flatten() {
                    if library::is_stale_frame(&entry.file_name().to_string_lossy()) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }

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

            apply_services(&handle);

            // Lecteur audio du cœur (Linux) : la musique ne dépend plus de la page.
            handle.manage(player::Player::new(&handle));
            player::start(&handle);

            // Fermeture de la fenetre : on range dans la barre, on demande, ou on quitte.
            if let Some(window) = app.get_webview_window("main") {
                attach_window(&window);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            player::player_play,
            player::player_queue,
            player::player_set_playing,
            player::player_step,
            player::player_seek,
            player::player_volume,
            player::player_snapshot,
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
            get_cover_paths,
            check_update,
            get_lyrics,
            audio_source,
            save_lyrics_sync,
            install_update,
            restart_to_update,
            open_releases,
            track_original,
            edit_track,
            set_now_playing,
            set_playback,
            notify_track,
            save_frame,
            grab_frame,
            set_duration,
            get_playlists,
            save_playlists
        ])
        .build(tauri::generate_context!())
        .expect("erreur au lancement d'Écho")
        .run(|app, event| {
            // Windows : une mise à jour téléchargée s'installe quand Écho se ferme,
            // quelle que soit la façon de le quitter (croix, barre système, menu).
            match event {
                // Plus de fenêtre, mais Écho joue depuis la barre du système : il reste ouvert.
                tauri::RunEvent::ExitRequested { code: None, api, .. }
                    if IN_TRAY.load(std::sync::atomic::Ordering::SeqCst) && tray_shown(app) =>
                {
                    api.prevent_exit();
                }
                tauri::RunEvent::Exit => {
                    player::save_now(app);
                    updates::install_on_quit(app);
                }
                _ => {}
            }
        });
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
            has_audio: true,
            video: false,
            no_frame: false,
            parsed: titles::VERSION,
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
