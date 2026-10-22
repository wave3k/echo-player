use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

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
    /// Les nouveaux fichiers des dossiers apparaissent tout seuls.
    pub watch: bool,
    /// Icône dans la barre du système, et fermeture qui y range la fenêtre.
    pub tray: bool,
    pub close_to_tray: bool,
    /// Notification du système à chaque nouveau morceau.
    pub notify: bool,
    /// Statut « Écoute … » sur Discord.
    pub discord: bool,

    /// Fondu enchaîné entre deux morceaux, en secondes (0 : aucun).
    pub crossfade: u32,
    /// Volume égal : rattrape les différences de niveau entre les fichiers.
    pub loudness: bool,
    /// Égaliseur : "flat", "bass", "vocal", "treble", "rap" ou "acoustic".
    pub eq: String,
    /// Mises à jour : "auto" (installées toutes seules), "notify" (prévenir) ou "off".
    pub updates: String,
    /// Paroles cherchées en ligne (LRCLIB) quand le fichier n'en a pas.
    pub lyrics_online: bool,
    /// Pochettes cherchées en ligne (MusicBrainz, Deezer) pour les morceaux sans pochette.
    pub covers_online: bool,
    /// Pochette voulue : "auto" (album s'il est dans les tags, sinon single), "album" ou "single".
    pub covers_prefer: String,
    /// Visualiseur : "bars" (barres), "wave" (onde) ou "off".
    pub visualizer: String,
    /// Photos des artistes cherchées en ligne (Deezer) pour la vue Artistes.
    pub artist_photos: bool,

    /// Vitesse de lecture (0,5 à 2).
    pub speed: f64,
    /// Tonalité liée à la vitesse (« slowed », « sped up ») : comme un disque plus lent.
    pub pitch_linked: bool,
    /// Réverbération, de 0 (aucune) à 100.
    pub reverb: u32,

    pub volume: u32,
    pub last_track: Option<String>,
    pub last_position_ms: u64,
    pub last_playing: bool,
    /// Version de la liste des formats : les formats ajoutés depuis sont cochés une fois.
    /// Absente d'un ancien fichier, elle vaut 0 (et non la valeur par défaut des réglages).
    #[serde(default)]
    pub formats_version: u32,
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
            formats: FORMAT_CATALOG.iter().map(|f| f.to_string()).collect(),
            watch: true,
            tray: false,
            close_to_tray: false,
            notify: false,
            discord: false,
            crossfade: 0,
            loudness: true,
            eq: "flat".into(),
            updates: "auto".into(),
            lyrics_online: false,
            covers_online: false,
            covers_prefer: "auto".into(),
            visualizer: "bars".into(),
            artist_photos: false,
            speed: 1.0,
            pitch_linked: false,
            reverb: 0,
            volume: 90,
            last_track: None,
            last_position_ms: 0,
            last_playing: false,
            formats_version: FORMATS_VERSION,
        }
    }
}

/// Les formats proposes dans les reglages, dans l'ordre d'affichage.
/// « autres » regroupe les extensions plus rares (wv, ape, mpc, avi…).
pub const FORMAT_CATALOG: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "aiff", "wma", "mp4", "webm", "mkv", "mov", "autres",
];

/// Formats de la première version : ceux qui n'y étaient pas sont ajoutés aux anciens réglages.
const FORMATS_V1: &[&str] = &["mp3", "mp4", "flac", "m4a", "ogg", "wav", "webm", "mkv"];
pub const FORMATS_VERSION: u32 = 2;

impl Settings {
    /// Remet dans des valeurs connues ce qui vient de l'extérieur (fichier importé,
    /// interface), pour qu'une valeur fantaisiste ne casse pas l'affichage.
    pub fn sanitize(&mut self) {
        fn pick(value: &mut String, allowed: &[&str], fallback: &str) {
            if !allowed.contains(&value.as_str()) {
                *value = fallback.into();
            }
        }
        pick(&mut self.theme, &["system", "light", "dark"], "dark");
        pick(&mut self.palette, &["nuit", "aurore", "menthe", "prune"], "nuit");
        pick(&mut self.font, &["space", "sora", "outfit"], "space");
        pick(&mut self.mode, &["normal", "compact", "minimal"], "normal");
        pick(&mut self.text_size, &["petit", "normal", "grand"], "normal");
        pick(&mut self.repeat_default, &["off", "all", "one"], "off");
        pick(&mut self.eq, &["flat", "bass", "vocal", "treble", "rap", "acoustic"], "flat");
        pick(&mut self.updates, &["auto", "notify", "off"], "auto");
        pick(&mut self.covers_prefer, &["auto", "album", "single"], "auto");
        pick(&mut self.visualizer, &["bars", "wave", "off"], "bars");
        if !matches!(self.crossfade, 0 | 3 | 6 | 10) {
            self.crossfade = 0;
        }
        if !self.tray {
            self.close_to_tray = false;
        }
        if !matches!(self.skip_seconds, 5 | 10 | 30) {
            self.skip_seconds = 10;
        }
        self.radius = self.radius.min(20);
        // Vitesse : arrondie au centième, une valeur absurde revient à la normale.
        self.speed = if self.speed.is_finite() { (self.speed.clamp(0.5, 2.0) * 100.0).round() / 100.0 } else { 1.0 };
        self.reverb = self.reverb.min(100);
        self.volume = self.volume.min(100);
        if self.formats_version < FORMATS_VERSION {
            // Réglages d'avant l'arrivée d'opus, aac… : ces formats n'existaient pas, on les coche.
            for f in FORMAT_CATALOG.iter().filter(|f| !FORMATS_V1.contains(f)) {
                if !self.formats.iter().any(|x| x == f) {
                    self.formats.push((*f).to_string());
                }
            }
            self.formats_version = FORMATS_VERSION;
        }
        self.formats.retain(|f| FORMAT_CATALOG.contains(&f.as_str()));
        self.formats.dedup();
        if self.formats.is_empty() {
            self.formats = Settings::default().formats;
        }
    }

    /// Prend les réglages envoyés par l'interface en gardant ce qui appartient au cœur :
    /// les dossiers (changés seulement par add_folder / remove_folder) et la reprise.
    /// L'interface peut envoyer une copie périmée, qui ne doit rien effacer.
    pub fn keep_core_from(&mut self, current: &Settings) {
        self.folders = current.folders.clone();
        self.last_track = current.last_track.clone();
        self.last_position_ms = current.last_position_ms;
        self.last_playing = current.last_playing;
    }
}

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

/// Écrit d'abord dans un fichier temporaire puis le renomme : une coupure pendant
/// l'écriture laisse l'ancien fichier intact au lieu d'un fichier à moitié écrit.
/// Le nom temporaire est propre à chaque écriture : deux fils qui écrivent le même
/// fichier ne se marchent pas dessus.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = unique_tmp(path, "tmp");
    // Données forcées sur le disque avant le renommage : une coupure de courant juste
    // après ne laisse jamais un fichier vide à la place de l'ancien.
    let written = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// « fichier.<pid>.<n>.<ext> », à côté du fichier visé.
pub fn unique_tmp(path: &Path, ext: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.{n}.{ext}", std::process::id()));
    PathBuf::from(tmp)
}

/// Une écriture de settings.json à la fois : la copie des réglages et son écriture
/// vont ensemble, sinon une copie plus ancienne pourrait être écrite en dernier.
pub fn settings_write_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lit un fichier JSON. S'il est illisible, il est mis de côté (« .bad ») plutôt que
/// d'être écrasé à la prochaine sauvegarde : rien n'est perdu sans trace.
fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(_) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = std::fs::rename(path, PathBuf::from(bad));
            None
        }
    }
}

pub fn load_settings(dir: &Path) -> Settings {
    let mut settings: Settings = load_json(&settings_file(dir)).unwrap_or_default();
    settings.sanitize();
    settings
}

pub fn save_settings(dir: &Path, settings: &Settings) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = write_atomic(&settings_file(dir), json.as_bytes());
    }
}

pub fn load_cache(dir: &Path) -> Option<Cache> {
    load_json(&cache_file(dir))
}

pub fn save_cache(dir: &Path, cache: &Cache) {
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = write_atomic(&cache_file(dir), json.as_bytes());
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

/// Corrections faites par l'utilisateur sur un morceau. Les fichiers audio ne sont
/// jamais modifiés : ces valeurs remplacent seulement ce qui est affiché.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct TrackOverride {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Nom du fichier de pochette (dans le cache) qui remplace celle du fichier.
    pub cover: Option<String>,
    /// Pochette trouvée en ligne toute seule : la préférence suivie ("album" ou "single").
    /// Absent pour une pochette choisie à la main, qu'une recherche ne remplace jamais.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_online: Option<String>,
    /// La pochette trouvée en ligne a été retirée : on n'en cherche plus pour ce morceau.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_online: bool,
    /// Morceau masqué de la bibliothèque (doublon) : le fichier reste sur le disque.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Volume propre au morceau, en dB (−12 à +12), en plus du volume égal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
}

impl TrackOverride {
    /// Rien à garder : l'entrée peut disparaître.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.cover.is_none()
            && !self.no_online
            && !self.hidden
            && self.gain_db.is_none()
    }
}

/// Corrections par identifiant de morceau (son chemin).
pub type Overrides = HashMap<String, TrackOverride>;

pub fn load_overrides(dir: &Path) -> Overrides {
    load_json(&dir.join("overrides.json")).unwrap_or_default()
}

pub fn save_overrides(dir: &Path, overrides: &Overrides) {
    if let Ok(json) = serde_json::to_string_pretty(overrides) {
        let _ = write_atomic(&dir.join("overrides.json"), json.as_bytes());
    }
}

/// Calages des paroles faits à la main, par morceau (lyrics_sync.json).
pub fn load_lyrics_sync(dir: &Path) -> HashMap<String, Vec<crate::lyrics::Anchor>> {
    std::fs::read(dir.join("lyrics_sync.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_lyrics_sync(dir: &Path, map: &HashMap<String, Vec<crate::lyrics::Anchor>>) -> std::io::Result<()> {
    write_atomic(&dir.join("lyrics_sync.json"), &serde_json::to_vec(map).map_err(std::io::Error::other)?)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::library::tests::TempDir;

    /// Bug de l'audit : opus et aac étaient retirés des réglages.
    #[test]
    fn sanitize_garde_les_formats_du_catalogue() {
        let mut s = Settings::default();
        s.formats = vec!["opus".into(), "aac".into(), "inconnu".into()];
        s.sanitize();
        assert_eq!(s.formats, ["opus", "aac"]);
    }

    #[test]
    fn d_anciens_reglages_recoivent_les_nouveaux_formats() {
        let old: Settings = serde_json::from_str(r#"{"formats":["mp3","flac"]}"#).unwrap();
        assert_eq!(old.formats_version, 0);
        let mut s = old;
        s.sanitize();
        assert!(s.formats.contains(&"opus".to_string()));
        assert!(s.formats.contains(&"mp3".to_string()));
        assert!(!s.formats.contains(&"wav".to_string()), "un format décoché le reste");
        assert_eq!(s.formats_version, FORMATS_VERSION);
    }

    #[test]
    fn sanitize_remet_les_valeurs_fantaisistes() {
        let mut s = Settings::default();
        s.mode = "géant".into();
        s.volume = 400;
        s.skip_seconds = 7;
        s.formats.clear();
        s.sanitize();
        assert_eq!(s.mode, "normal");
        assert_eq!(s.volume, 100);
        assert_eq!(s.skip_seconds, 10);
        assert!(!s.formats.is_empty());
    }

    /// Bug de l'audit : un réglage envoyé par l'interface effaçait le dossier ajouté.
    #[test]
    fn l_interface_ne_peut_pas_effacer_les_dossiers() {
        let mut current = Settings::default();
        current.folders = vec!["/musique".into(), "/nouveau".into()];
        current.last_track = Some("/musique/a.mp3".into());
        let mut from_ui = current.clone();
        from_ui.folders = vec!["/musique".into()];
        from_ui.last_track = None;
        from_ui.volume = 40;
        from_ui.keep_core_from(&current);
        assert_eq!(from_ui.folders, ["/musique", "/nouveau"]);
        assert_eq!(from_ui.last_track.as_deref(), Some("/musique/a.mp3"));
        assert_eq!(from_ui.volume, 40);
    }

    /// Bug de l'audit : un fichier abîmé faisait tout perdre, en silence.
    #[test]
    fn un_reglage_abime_est_mis_de_cote() {
        let tmp = TempDir::new("settings");
        let mut s = Settings::default();
        s.folders.push("/ma/musique".into());
        save_settings(&tmp.0, &s);
        let text = std::fs::read_to_string(tmp.0.join("settings.json")).unwrap();
        std::fs::write(tmp.0.join("settings.json"), &text[..text.len() / 2]).unwrap();

        let loaded = load_settings(&tmp.0);
        assert!(loaded.folders.is_empty());
        let kept = std::fs::read_to_string(tmp.0.join("settings.json.bad")).unwrap();
        assert!(kept.contains("/ma/musique"), "l'ancien fichier reste récupérable");
    }

    #[test]
    fn l_ecriture_ne_laisse_pas_de_fichier_temporaire() {
        let tmp = TempDir::new("atomic");
        let mut s = Settings::default();
        s.volume = 33;
        save_settings(&tmp.0, &s);
        save_settings(&tmp.0, &s);
        assert_eq!(load_settings(&tmp.0).volume, 33);
        let left: Vec<_> = std::fs::read_dir(&tmp.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
    }

    /// Bug de l'audit : deux écritures simultanées partageaient le même fichier temporaire.
    #[test]
    fn des_ecritures_simultanees_ne_se_melangent_pas() {
        let tmp = TempDir::new("concurrent");
        let file = tmp.0.join("x.json");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let file = file.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        write_atomic(&file, format!("{i}").repeat(5000).as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.len(), 5000);
        assert!(text.chars().all(|c| c == text.chars().next().unwrap()), "jamais deux écritures mêlées");
        assert_ne!(unique_tmp(&file, "tmp"), unique_tmp(&file, "tmp"));
    }

    #[test]
    fn les_playlists_sont_gardees_propres() {
        let tmp = TempDir::new("playlists");
        let dirty = vec![
            Playlist { id: "a".into(), name: "  Rap FR  ".into(), tracks: vec!["/m/1".into(), "/m/1".into(), "/m/2".into()], query: None },
            Playlist { id: "a".into(), name: "doublon".into(), tracks: vec![], query: None },
            Playlist { id: " ".into(), name: "sans id".into(), tracks: vec![], query: None },
            Playlist { id: "b".into(), name: "".into(), tracks: vec![], query: None },
        ];
        let clean = sanitize_playlists(dirty);
        assert_eq!(clean.len(), 2);
        assert_eq!(clean[0].name, "Rap FR");
        assert_eq!(clean[0].tracks, ["/m/1", "/m/2"]);
        assert_eq!(clean[1].name, "Sans nom");
        save_playlists(&tmp.0, &clean);
        assert_eq!(load_playlists(&tmp.0), clean);
    }

    #[test]
    fn les_reglages_de_son_restent_valides() {
        let mut s = Settings::default();
        s.crossfade = 7;
        s.eq = "boum".into();
        s.close_to_tray = true;
        s.sanitize();
        assert_eq!(s.crossfade, 0);
        assert_eq!(s.eq, "flat");
        assert!(!s.close_to_tray, "pas de rangement sans icône dans la barre");
    }

    #[test]
    fn les_corrections_survivent_a_un_redemarrage() {
        let tmp = TempDir::new("overrides");
        let mut map = Overrides::new();
        map.insert(
            "/m/a.mp3".into(),
            TrackOverride { title: Some("Nouveau".into()), ..Default::default() },
        );
        save_overrides(&tmp.0, &map);
        assert_eq!(load_overrides(&tmp.0)["/m/a.mp3"].title.as_deref(), Some("Nouveau"));
    }
}

/// Écoutes d'un morceau.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Plays {
    pub count: u32,
    /// Dernière écoute (secondes depuis 1970).
    pub last: u64,
}

/// Statistiques d'écoute et favoris (stats.json), pour les playlists automatiques.
/// Rien ne quitte l'ordinateur.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Stats {
    pub plays: HashMap<String, Plays>,
    /// Morceaux aimés, le plus récent en premier.
    pub favorites: Vec<String>,
}

pub fn load_stats(dir: &Path) -> Stats {
    load_json(&dir.join("stats.json")).unwrap_or_default()
}

pub fn save_stats(dir: &Path, stats: &Stats) -> std::io::Result<()> {
    let json = serde_json::to_vec(stats).map_err(std::io::Error::other)?;
    write_atomic(&dir.join("stats.json"), &json)
}

/// Une liste de lecture : un nom et des morceaux (par identifiant), dans l'ordre.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
    /// Playlist intelligente : la recherche qui la remplit (ses morceaux sont calculés
    /// par l'interface, `tracks` reste vide).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

pub fn load_playlists(dir: &Path) -> Vec<Playlist> {
    load_json(&dir.join("playlists.json")).unwrap_or_default()
}

pub fn save_playlists(dir: &Path, playlists: &[Playlist]) {
    if let Ok(json) = serde_json::to_string_pretty(playlists) {
        let _ = write_atomic(&dir.join("playlists.json"), json.as_bytes());
    }
}

/// Remet une liste de playlists reçue de l'interface dans un état sûr.
pub fn sanitize_playlists(list: Vec<Playlist>) -> Vec<Playlist> {
    let mut seen = std::collections::HashSet::new();
    list.into_iter()
        .filter(|p| !p.id.trim().is_empty() && seen.insert(p.id.clone()))
        .map(|mut p| {
            p.name = p.name.trim().chars().take(80).collect();
            if p.name.is_empty() {
                p.name = "Sans nom".into();
            }
            let mut ids = std::collections::HashSet::new();
            p.tracks.retain(|t| ids.insert(t.clone()));
            p.query = p.query.map(|q| q.trim().chars().take(300).collect::<String>()).filter(|q| !q.is_empty());
            if p.query.is_some() {
                p.tracks.clear();
            }
            p
        })
        .collect()
}
