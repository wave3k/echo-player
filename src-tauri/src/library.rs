use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Un morceau de la bibliotheque (audio ou video).
#[derive(Serialize, Deserialize, Clone)]
pub struct Track {
    /// Identifiant stable : le chemin complet du fichier.
    pub id: String,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    /// "audio" ou "video"
    pub kind: String,
    /// Nom du fichier de pochette dans le dossier de cache, si trouve.
    pub cover: Option<String>,
    /// true si le fichier doit etre lu par la vue video.
    pub is_video: bool,
}

const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "mp4a", "aac", "flac", "ogg", "oga", "opus", "wav", "wv", "aiff", "aif", "m4b",
    "ape", "mpc", "wma",
];
const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "webm", "mkv", "mov", "avi", "ogv"];

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_audio(ext: &str) -> bool {
    AUDIO_EXTS.contains(&ext)
}

fn is_video(ext: &str) -> bool {
    VIDEO_EXTS.contains(&ext)
}

/// Balaye un dossier et en extrait la liste des morceaux lisibles.
/// Les fichiers non media sont ignores en silence.
pub fn scan(folder: &Path, covers_dir: &Path) -> (Vec<Track>, HashMap<String, PathBuf>) {
    let mut tracks: Vec<Track> = Vec::new();
    let mut covers: HashMap<String, PathBuf> = HashMap::new();

    for entry in WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = ext_of(path);
        let video = is_video(&ext);
        if !video && !is_audio(&ext) {
            continue;
        }

        let fallback_title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Sans titre")
            .to_string();

        let mut track = Track {
            id: path.to_string_lossy().to_string(),
            path: path.to_string_lossy().to_string(),
            title: fallback_title.clone(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            kind: if video { "video".into() } else { "audio".into() },
            cover: None,
            is_video: video,
        };

        if let Some(meta) = read_metadata(path, covers_dir, &mut covers) {
            if !meta.title.is_empty() {
                track.title = meta.title;
            }
            track.artist = meta.artist;
            track.album = meta.album;
            track.duration_ms = meta.duration_ms;
            track.cover = meta.cover;
        }

        tracks.push(track);
    }

    tracks.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
    });

    (tracks, covers)
}

struct Meta {
    title: String,
    artist: String,
    album: String,
    duration_ms: u64,
    cover: Option<String>,
}

fn read_metadata(
    path: &Path,
    covers_dir: &Path,
    covers: &mut HashMap<String, PathBuf>,
) -> Option<Meta> {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::probe::Probe;
    use lofty::tag::ItemKey;

    let tagged = Probe::open(path).ok()?.read().ok()?;

    let duration_ms = tagged.properties().duration().as_millis() as u64;

    let mut meta = Meta {
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration_ms,
        cover: None,
    };

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    if let Some(tag) = tag {
        meta.title = tag.get_string(&ItemKey::TrackTitle).unwrap_or("").to_string();
        meta.artist = tag
            .get_string(&ItemKey::TrackArtist)
            .or_else(|| tag.get_string(&ItemKey::AlbumArtist))
            .unwrap_or("")
            .to_string();
        meta.album = tag.get_string(&ItemKey::AlbumTitle).unwrap_or("").to_string();

        if let Some(pic) = tag.pictures().first() {
            let data = pic.data();
            if !data.is_empty() {
                let mut hasher = DefaultHasher::new();
                data.hash(&mut hasher);
                let hash = hasher.finish();
                let ext = match pic
                    .mime_type()
                    .map(|m| m.as_str())
                    .unwrap_or("image/jpeg")
                {
                    "image/png" => "png",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    _ => "jpg",
                };
                let name = format!("{:016x}.{}", hash, ext);
                let dest = covers_dir.join(&name);
                if !covers.contains_key(&name) {
                    if !dest.exists() {
                        let _ = std::fs::write(&dest, data);
                    }
                    covers.insert(name.clone(), dest);
                }
                meta.cover = Some(name);
            }
        }
    }

    Some(meta)
}
