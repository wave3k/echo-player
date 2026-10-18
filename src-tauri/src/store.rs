use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
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
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("lyrics_sync.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(map).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, dir.join("lyrics_sync.json"))
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
        assert!(!tmp.0.join("settings.json.tmp").exists());
    }

    #[test]
    fn les_playlists_sont_gardees_propres() {
        let tmp = TempDir::new("playlists");
        let dirty = vec![
            Playlist { id: "a".into(), name: "  Rap FR  ".into(), tracks: vec!["/m/1".into(), "/m/1".into(), "/m/2".into()] },
            Playlist { id: "a".into(), name: "doublon".into(), tracks: vec![] },
            Playlist { id: " ".into(), name: "sans id".into(), tracks: vec![] },
            Playlist { id: "b".into(), name: "".into(), tracks: vec![] },
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

/// Une liste de lecture : un nom et des morceaux (par identifiant), dans l'ordre.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
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
            p
        })
        .collect()
}
