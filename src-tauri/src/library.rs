use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Un morceau de la bibliotheque. Tout est lu comme de l'audio : pour un mp4, seule la piste son est jouee.
#[derive(Serialize, Deserialize, Clone)]
pub struct Track {
    /// Identifiant stable : le chemin complet du fichier.
    pub id: String,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    /// Nom du fichier de pochette dans le dossier de cache, si trouve.
    pub cover: Option<String>,
    /// Date de modification et taille du fichier au moment de l'analyse.
    /// Si rien n'a changé, on ne relit pas ses tags.
    #[serde(default)]
    pub mtime: u64,
    #[serde(default)]
    pub size: u64,
    /// Faux pour une vidéo téléchargée sans son (fichier muet).
    #[serde(default = "yes")]
    pub has_audio: bool,
    /// Vrai si le fichier a une piste image : sa pochette peut venir d'une image de la vidéo.
    #[serde(default)]
    pub video: bool,
    /// Vrai quand l'image de la vidéo a déjà été cherchée sans succès.
    #[serde(default)]
    pub no_frame: bool,
    /// Version des règles de titres appliquées (voir `titles::VERSION`).
    #[serde(default)]
    pub parsed: u32,
}

fn yes() -> bool {
    true
}

/// Conteneurs qui peuvent porter une image : leur pochette peut être tirée de la vidéo.
const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "webm", "mkv", "mov", "avi", "ogv"];

pub fn is_video(path: &Path) -> bool {
    VIDEO_EXTS.contains(&ext_of(path).as_str())
}

/// Nom du fichier de pochette tiré d'une vidéo : change avec le fichier.
pub fn frame_name(id: &str, mtime: u64) -> String {
    format!("still-{:016x}-{mtime}.jpg", stable_hash(id.as_bytes()))
}

/// Pochettes « frame-… » des versions 1.1.0 : souvent noires (vidéos AV1), on les refait.
pub fn is_stale_frame(name: &str) -> bool {
    name.starts_with("frame-")
}

/// Marque « aucune image trouvée » pour ne pas recommencer à chaque lancement.
pub fn no_frame_marker(id: &str, mtime: u64) -> String {
    format!("still-{:016x}-{mtime}.none", stable_hash(id.as_bytes()))
}

/// Extensions lues par la bibliotheque. Les conteneurs video (mp4, mkv...) sont lus
/// pour leur piste son seulement.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "mp4a", "aac", "flac", "ogg", "oga", "opus", "wav", "wv", "aiff", "aif", "m4b", "ape", "mpc", "wma",
    "mp4", "m4v", "webm", "mkv", "mov", "avi", "ogv",
];

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_audio(ext: &str) -> bool {
    AUDIO_EXTS.contains(&ext)
}

/// Vrai si ce fichier a une extension que la bibliothèque sait lire.
pub fn is_media_path(path: &Path) -> bool {
    is_audio(&ext_of(path))
}

/// Vrai si ce fichier serait gardé par le scan avec ces formats.
pub fn is_listed(path: &Path, formats: &[String]) -> bool {
    let ext = ext_of(path);
    is_audio(&ext) && allowed(&ext, formats)
}

/// Le format des réglages dont relève une extension (oga → ogg, aif → aiff…).
/// Les extensions rares sont rangées sous « autres ».
pub fn format_key(ext: &str) -> &str {
    match ext {
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "aiff" | "wma" | "mp4" | "webm" | "mkv"
        | "mov" => ext,
        "oga" | "ogv" => "ogg",
        "aif" => "aiff",
        "m4b" | "mp4a" => "m4a",
        "m4v" => "mp4",
        _ => "autres",
    }
}

/// Vrai si l'extension fait partie des formats autorises dans les reglages.
fn allowed(ext: &str, formats: &[String]) -> bool {
    let key = format_key(ext);
    formats.iter().any(|f| f == key)
}

/// Balaye les dossiers choisis et en extrait la liste des morceaux lisibles.
/// Les fichiers non media (ou des formats decoaches) sont ignores en silence.
/// `previous` : les morceaux déjà connus. Un fichier inchangé reprend ses tags
/// sans être relu.
pub fn scan(
    folders: &[PathBuf],
    covers_dir: &Path,
    formats: &[String],
    previous: &HashMap<String, Track>,
) -> (Vec<Track>, HashMap<String, PathBuf>) {
    let mut tracks: Vec<Track> = Vec::new();
    let mut covers: HashMap<String, PathBuf> = HashMap::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for folder in folders {
        // Les liens symboliques sont suivis : beaucoup de bibliothèques sont
        // rangées dans des dossiers liés. walkdir évite les boucles.
        for entry in WalkDir::new(folder)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let ext = ext_of(path);
            if !is_audio(&ext) {
                continue;
            }
            if !allowed(&ext, formats) {
                continue;
            }

            let id = path.to_string_lossy().to_string();
            if !seen.insert(id.clone()) {
                continue;
            }

            let (mtime, size) = match entry.metadata() {
                Ok(m) => (
                    m.modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    m.len(),
                ),
                Err(_) => (0, 0),
            };

            // Fichier inchangé : on reprend ce qu'on sait déjà, sans relire les tags.
            if let Some(old) = previous.get(&id) {
                let cover_ok = old
                    .cover
                    .as_ref()
                    .map(|name| !is_stale_frame(name) && covers_dir.join(name).is_file())
                    .unwrap_or(true);
                let fresh = old.parsed == crate::titles::VERSION;
                if old.mtime == mtime && old.size == size && old.mtime != 0 && cover_ok && fresh {
                    if let Some(name) = &old.cover {
                        covers.insert(name.clone(), covers_dir.join(name));
                    }
                    tracks.push(old.clone());
                    continue;
                }
            }

            let fallback_title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Sans titre")
                .to_string();

            let mut track = Track {
                id: id.clone(),
                path: id,
                title: fallback_title.clone(),
                artist: String::new(),
                album: String::new(),
                duration_ms: 0,
                cover: None,
                mtime,
                size,
                has_audio: true,
                video: is_video(path),
                no_frame: false,
                parsed: crate::titles::VERSION,
            };

            let meta = match ext.as_str() {
                "webm" | "mkv" => read_matroska(path, covers_dir, &mut covers),
                "ogg" | "oga" | "ogv" | "opus" => read_metadata(path, covers_dir, &mut covers).or_else(|| read_ogg(path)),
                _ => read_metadata(path, covers_dir, &mut covers).or_else(|| read_mp4(path)),
            };
            let mut named_by_file = true;
            if let Some(meta) = meta {
                if !meta.title.is_empty() {
                    track.title = meta.title;
                    named_by_file = false;
                }
                track.artist = meta.artist;
                track.album = meta.album;
                track.duration_ms = meta.duration_ms;
                track.cover = meta.cover;
                track.has_audio = meta.has_audio;
                if let Some(video) = meta.has_video {
                    track.video = video;
                }
            }

            // Pas d'artiste dans les tags : on le cherche dans le titre (« Artiste - Titre »).
            if track.artist.trim().is_empty() {
                let (artist, title) = crate::titles::guess(&track.title);
                track.artist = artist;
                track.title = title;
            } else if named_by_file {
                // Artiste connu mais titre tiré du nom de fichier : « 03 - Titre » → « Titre ».
                track.title = crate::titles::strip_track_number(&track.title).to_string();
            }

            // Vidéo sans pochette : une image de la vidéo, extraite une fois par l'interface.
            if track.video && track.cover.is_none() {
                let frame = frame_name(&track.id, mtime);
                if covers_dir.join(&frame).is_file() {
                    covers.insert(frame.clone(), covers_dir.join(&frame));
                    track.cover = Some(frame);
                } else if covers_dir.join(no_frame_marker(&track.id, mtime)).is_file() {
                    track.no_frame = true;
                }
            }

            tracks.push(track);
        }
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
    has_audio: bool,
    /// `None` : on ne sait pas (on s'en tient à l'extension).
    has_video: Option<bool>,
}

/// Enregistre une pochette (octets d'image) dans le cache et renvoie son nom.
fn save_cover(data: &[u8], mime: &str, covers_dir: &Path, covers: &mut HashMap<String, PathBuf>) -> String {
    let ext = match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    };
    let name = format!("{:016x}.{}", stable_hash(data), ext);
    let dest = covers_dir.join(&name);
    if !covers.contains_key(&name) {
        if !dest.exists() {
            let _ = std::fs::write(&dest, data);
        }
        covers.insert(name.clone(), dest);
    }
    name
}

/// MP4 que `lofty` refuse (vidéo sans son) : au moins sa durée, et le fait qu'il est muet.
fn read_mp4(path: &Path) -> Option<Meta> {
    let info = crate::mp4::read(path)?;
    Some(Meta {
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration_ms: info.duration_ms,
        cover: None,
        has_audio: info.has_audio,
        has_video: Some(info.has_video),
    })
}

/// Ogg que `lofty` refuse (première piste vidéo) : ses commentaires et sa durée.
fn read_ogg(path: &Path) -> Option<Meta> {
    let info = crate::ogg::read(path)?;
    Some(Meta {
        title: info.title,
        artist: info.artist,
        album: info.album,
        duration_ms: info.duration_ms,
        cover: None,
        has_audio: info.has_audio,
        has_video: Some(info.has_video),
    })
}

fn read_matroska(path: &Path, covers_dir: &Path, covers: &mut HashMap<String, PathBuf>) -> Option<Meta> {
    let info = crate::matroska::read(path)?;
    Some(Meta {
        title: info.title,
        artist: info.artist,
        album: info.album,
        duration_ms: info.duration_ms,
        cover: info.cover.map(|(mime, data)| save_cover(&data, &mime, covers_dir, covers)),
        has_audio: info.has_audio.unwrap_or(true),
        has_video: info.has_audio.map(|_| info.has_video),
    })
}

/// Empreinte FNV-1a 64 bits : stable d'une version de Rust à l'autre,
/// contrairement à DefaultHasher, ce qui évite de dupliquer les pochettes.
pub fn stable_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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

    let properties = tagged.properties();
    let duration_ms = properties.duration().as_millis() as u64;

    let mut meta = Meta {
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration_ms,
        cover: None,
        // Une vidéo mp4 sans piste son n'annonce aucun canal audio.
        has_audio: properties.channels().is_some() || properties.sample_rate().is_some(),
        // lofty ne dit rien de l'image : pour un mp4, on regarde ses pistes.
        has_video: is_video(path).then(|| crate::mp4::read(path).map(|i| i.has_video)).flatten(),
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
                let mime = pic.mime_type().map(|m| m.as_str()).unwrap_or("image/jpeg");
                meta.cover = Some(save_cover(data, mime, covers_dir, covers));
            }
        }
    }

    Some(meta)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::{Tag, TagType};

    /// Un dossier temporaire propre à chaque test, effacé à la fin.
    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("echo-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Écrit un WAV d'une seconde, avec un titre et une pochette dans ses tags.
    pub fn tagged_wav(path: &Path, title: &str) {
        let n: u32 = 8000;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + n * 2).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(8000u32.to_le_bytes());
        b.extend(16000u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend((n * 2).to_le_bytes());
        b.extend(vec![0u8; (n * 2) as usize]);
        std::fs::write(path, b).unwrap();

        let mut file = lofty::read_from_path(path).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::TrackTitle, title.into());
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            vec![137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3],
        ));
        file.insert_tag(tag);
        file.save_to_path(path, WriteOptions::default()).unwrap();
    }

    fn all_formats() -> Vec<String> {
        crate::store::Settings::default().formats
    }

    #[test]
    fn lit_le_titre_et_la_pochette_des_tags() {
        let tmp = TempDir::new("tags");
        let covers = tmp.0.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        tagged_wav(&tmp.0.join("a.wav"), "Titre tag");

        let (tracks, map) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &HashMap::new());
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Titre tag");
        let cover = tracks[0].cover.clone().expect("pochette extraite");
        assert!(covers.join(&cover).is_file());
        assert!(map.contains_key(&cover));
    }

    /// Bug de l'audit : après « Vider le cache », les pochettes ne revenaient jamais.
    #[test]
    fn les_pochettes_reviennent_apres_vidage_du_cache() {
        let tmp = TempDir::new("covers");
        let covers = tmp.0.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        tagged_wav(&tmp.0.join("a.wav"), "A");
        let (first, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &HashMap::new());

        // Ce que fait clear_cover_cache : fichiers effacés, pochette et date oubliées.
        for entry in std::fs::read_dir(&covers).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        let previous: HashMap<String, Track> = first
            .into_iter()
            .map(|mut t| {
                t.cover = None;
                t.mtime = 0;
                (t.id.clone(), t)
            })
            .collect();

        let (again, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &previous);
        let cover = again[0].cover.clone().expect("la pochette doit revenir");
        assert!(covers.join(cover).is_file());
    }

    #[test]
    fn un_fichier_inchange_n_est_pas_relu() {
        let tmp = TempDir::new("incremental");
        let covers = tmp.0.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        tagged_wav(&tmp.0.join("a.wav"), "Vrai titre");
        let (first, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &HashMap::new());
        let mut known = first[0].clone();
        known.title = "Déjà connu".into();
        let previous = HashMap::from([(known.id.clone(), known)]);
        let (again, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &previous);
        assert_eq!(again[0].title, "Déjà connu");
    }

    /// Bug de l'audit : opus, aac, aiff… n'étaient jamais lus.
    #[test]
    fn opus_aac_et_aiff_sont_listes() {
        let tmp = TempDir::new("formats");
        let covers = tmp.0.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        for name in ["a.opus", "b.aac", "c.aif", "d.m4b", "e.wv", "f.txt"] {
            std::fs::write(tmp.0.join(name), b"x").unwrap();
        }
        let (tracks, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &HashMap::new());
        let mut names: Vec<String> = tracks.iter().map(|t| t.title.clone()).collect();
        names.sort();
        assert_eq!(names, ["a", "b", "c", "d", "e"]);
    }

    /// Bug de l'audit : « 13 - Funkorama », aux tags illisibles, avait l'artiste « 13 ».
    #[test]
    fn le_numero_de_piste_n_est_pas_un_artiste() {
        let tmp = TempDir::new("piste");
        let covers = tmp.0.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        std::fs::write(tmp.0.join("13 - Funkorama.mp3"), b"illisible").unwrap();
        let (tracks, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &HashMap::new());
        assert_eq!(tracks[0].artist, "");
        assert_eq!(tracks[0].title, "Funkorama");
    }

    /// Bug de l'audit : un Ogg avec une piste image Theora perdait tous ses tags.
    #[test]
    fn un_ogg_avec_piste_video_garde_ses_tags() {
        let tmp = TempDir::new("ogg-scan");
        let covers = tmp.0.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        crate::ogg::tests::theora_then_vorbis(&tmp.0.join("01 - piste.ogg"), &["TITLE=Funkorama", "ARTIST=Quelqu'un"]);
        let (tracks, _) = scan(std::slice::from_ref(&tmp.0), &covers, &all_formats(), &HashMap::new());
        assert_eq!(tracks[0].title, "Funkorama");
        assert_eq!(tracks[0].artist, "Quelqu'un");
        assert!(tracks[0].has_audio && tracks[0].video);
        assert_eq!(tracks[0].duration_ms, 3000);
    }

    #[test]
    fn les_formats_decoches_sont_ignores() {
        let formats = vec!["mp3".to_string()];
        assert!(is_listed(Path::new("/m/a.MP3"), &formats));
        assert!(!is_listed(Path::new("/m/a.opus"), &formats));
        assert_eq!(format_key("oga"), "ogg");
        assert_eq!(format_key("aif"), "aiff");
        assert_eq!(format_key("ape"), "autres");
    }

    #[test]
    fn l_empreinte_des_pochettes_est_stable() {
        // FNV-1a de « a » : valeur publiée, elle ne doit jamais changer.
        assert_eq!(stable_hash(b"a"), 0xaf63dc4c8601ec8c);
    }
}
