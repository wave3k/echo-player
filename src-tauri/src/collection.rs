//! Outils sur l'ensemble de la bibliothèque : doublons, et playlists au format M3U
//! (celui de VLC, foobar2000, Winamp, des baladeurs…).

use crate::library::Track;
use crate::lyrics::{fold, title_variants};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/* ---------------- doublons ---------------- */

#[derive(Serialize, Clone, Debug)]
pub struct DupItem {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub path: String,
    /// Format (« FLAC », « MP3 », « WEBM »…).
    pub format: String,
    pub size: u64,
    pub duration_ms: u64,
    pub video: bool,
    /// Celui qu'on conseille de garder (le meilleur son).
    pub keep: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct DupGroup {
    pub items: Vec<DupItem>,
    /// Le même morceau en audio et en clip : souvent voulu, aucun conseil.
    pub mixed: bool,
}

/// Clé d'un morceau : artiste principal et titre sans invités ni mentions, sans
/// accents ni casse. « Meridian (feat. Dave) » de « Tiakola x Dave » = « meridian » de « tiakola ».
fn dup_key(t: &Track) -> Option<String> {
    let artist = t.artists.first().map(String::as_str).unwrap_or(&t.artist);
    let artist = fold(artist).replace(' ', "");
    let title = fold(title_variants(&t.title).last().map(String::as_str).unwrap_or(&t.title));
    if artist.is_empty() || title.is_empty() {
        return None;
    }
    Some(format!("{artist}\n{title}"))
}

/// Qualité du son, pour choisir lequel garder : un format sans perte d'abord, puis le
/// débit (taille / durée), un fichier audio avant un clip.
fn quality(t: &Track) -> (u8, u8, u64) {
    let ext = Path::new(&t.path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let lossless = matches!(ext.as_str(), "flac" | "wav" | "aiff" | "aif" | "ape" | "wv" | "alac") as u8;
    let rate = (t.size * 8).checked_div(t.duration_ms).unwrap_or(0);
    (t.has_audio as u8, lossless * 2 + (!t.video) as u8, rate)
}

/// Groupes de morceaux identiques : même artiste principal, même titre, et durées
/// proches (4 s ; un clip peut avoir une intro, jusqu'à 45 s de plus).
pub fn duplicates(tracks: &[Track]) -> Vec<DupGroup> {
    let mut by_key: HashMap<String, Vec<&Track>> = HashMap::new();
    for t in tracks {
        if let Some(key) = dup_key(t) {
            by_key.entry(key).or_default().push(t);
        }
    }
    let mut groups = Vec::new();
    for (_, mut list) in by_key {
        if list.len() < 2 {
            continue;
        }
        list.sort_by_key(|t| t.duration_ms);
        // Sous-groupes de durées voisines : deux versions (radio, longue) restent à part.
        let mut current: Vec<&Track> = Vec::new();
        let mut flush = |current: &mut Vec<&Track>| {
            if current.len() >= 2 {
                groups.push(make_group(current));
            }
            current.clear();
        };
        for t in list {
            let close = current.last().is_none_or(|prev: &&Track| {
                let tolerance = if prev.video || t.video { 45_000 } else { 4_000 };
                prev.duration_ms == 0 || t.duration_ms == 0 || t.duration_ms.abs_diff(prev.duration_ms) <= tolerance
            });
            if !close {
                flush(&mut current);
            }
            current.push(t);
        }
        flush(&mut current);
    }
    groups.sort_by(|a, b| a.items[0].title.to_lowercase().cmp(&b.items[0].title.to_lowercase()));
    groups
}

fn make_group(list: &[&Track]) -> DupGroup {
    let mixed = list.iter().any(|t| t.video) && list.iter().any(|t| !t.video);
    let best = list.iter().max_by_key(|t| quality(t)).map(|t| t.id.clone());
    DupGroup {
        mixed,
        items: list
            .iter()
            .map(|t| DupItem {
                id: t.id.clone(),
                title: t.title.clone(),
                artist: t.artist.clone(),
                path: t.path.clone(),
                format: Path::new(&t.path).extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase(),
                size: t.size,
                duration_ms: t.duration_ms,
                video: t.video,
                keep: !mixed && best.as_deref() == Some(t.id.as_str()),
            })
            .collect(),
    }
}

/* ---------------- M3U ---------------- */

/// Playlist au format M3U étendu (UTF-8) : chemins complets, durée et nom de chaque morceau.
pub fn to_m3u(name: &str, tracks: &[&Track]) -> String {
    let mut out = String::from("#EXTM3U\n");
    out.push_str(&format!("#PLAYLIST:{}\n", one_line(name)));
    for t in tracks {
        let secs = if t.duration_ms > 0 { (t.duration_ms / 1000) as i64 } else { -1 };
        let label = if t.artist.is_empty() { t.title.clone() } else { format!("{} - {}", t.artist, t.title) };
        out.push_str(&format!("#EXTINF:{secs},{}\n{}\n", one_line(&label), t.path));
    }
    out
}

fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Une entrée lue dans un fichier M3U.
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// « Artiste - Titre » de la ligne #EXTINF, s'il y en a une.
    pub label: Option<String>,
}

/// Nom et entrées d'un M3U. Les chemins relatifs partent du dossier du fichier ; les
/// adresses « file:// » et les chemins Windows sont compris. Les adresses web sont ignorées.
pub fn parse_m3u(text: &str, base: &Path) -> (Option<String>, Vec<Entry>) {
    let text = text.trim_start_matches('\u{feff}');
    let mut name = None;
    let mut label = None;
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#PLAYLIST:") {
            name = Some(rest.trim().to_string()).filter(|n| !n.is_empty());
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            label = rest.split_once(',').map(|(_, l)| l.trim().to_string()).filter(|l| !l.is_empty());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let raw = if let Some(rest) = line.strip_prefix("file://") {
            percent_decode(rest.strip_prefix("localhost").unwrap_or(rest))
        } else if line.contains("://") {
            label = None;
            continue;
        } else {
            line.to_string()
        };
        let raw = raw.replace('\\', "/");
        let path = PathBuf::from(&raw);
        let path = if path.is_absolute() || raw.chars().nth(1) == Some(':') { path } else { base.join(path) };
        entries.push(Entry { path, label: label.take() });
    }
    (name, entries)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(v) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Retrouve chaque entrée dans la bibliothèque : par chemin, puis par nom de fichier
/// (playlist venue d'un autre ordinateur), puis par « Artiste - Titre ».
pub fn match_entries(entries: &[Entry], tracks: &[Track]) -> (Vec<String>, usize) {
    let by_path: HashMap<&str, &str> = tracks.iter().map(|t| (t.path.as_str(), t.id.as_str())).collect();
    let mut by_name: HashMap<String, &str> = HashMap::new();
    let mut by_label: HashMap<String, &str> = HashMap::new();
    let mut by_title: HashMap<String, Vec<&Track>> = HashMap::new();
    for t in tracks {
        if let Some(n) = Path::new(&t.path).file_name() {
            by_name.entry(n.to_string_lossy().to_lowercase()).or_insert(&t.id);
        }
        by_label.entry(fold(&format!("{} {}", t.artist, t.title))).or_insert(&t.id);
        by_title.entry(fold(&t.title)).or_default().push(t);
    }
    // « Artiste - Titre » : même titre, et un artiste qui contient celui de la ligne
    // (« Kevin MacLeod » dans « Kevin MacLeod & Invité »). Sans artiste : titre unique.
    let by_parts = |label: &str| -> Option<&str> {
        let (artist, title) = match label.split_once(" - ") {
            Some((a, t)) => (fold(a), fold(t)),
            None => (String::new(), fold(label)),
        };
        let same = by_title.get(&title)?;
        if artist.is_empty() {
            return (same.len() == 1).then(|| same[0].id.as_str());
        }
        same.iter().find(|t| fold(&t.artist).contains(&artist)).map(|t| t.id.as_str())
    };
    let mut ids = Vec::new();
    let mut missing = 0;
    for e in entries {
        let path = e.path.to_string_lossy();
        let found = by_path
            .get(path.as_ref())
            .copied()
            .or_else(|| e.path.file_name().and_then(|n| by_name.get(&n.to_string_lossy().to_lowercase()).copied()))
            .or_else(|| e.label.as_deref().and_then(|l| by_label.get(&fold(&l.replacen(" - ", " ", 1))).copied()))
            .or_else(|| e.label.as_deref().and_then(by_parts));
        match found {
            Some(id) if !ids.iter().any(|x: &String| x == id) => ids.push(id.to_string()),
            Some(_) => {}
            None => missing += 1,
        }
    }
    (ids, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, artist: &str, title: &str, ms: u64, size: u64) -> Track {
        let mut t = crate::library::tests::track(path, title);
        t.artist = artist.into();
        t.duration_ms = ms;
        t.size = size;
        t.video = path.ends_with(".webm");
        let solo = |_: &str| true;
        (t.artists, t.featured) = crate::titles::credits(&t.artist, &t.title, &solo);
        t
    }

    #[test]
    fn les_doublons_sont_regroupes() {
        let tracks = vec![
            track("/m/a.mp3", "Tiakola", "Meridian (feat. Dave)", 255_000, 6_000_000),
            track("/m/b.flac", "Tiakola x Dave", "Meridian", 254_000, 30_000_000),
            track("/m/c.webm", "Tiakola", "Meridian (Clip officiel)", 280_000, 90_000_000),
            track("/m/d.mp3", "Tiakola", "Meridian (Remix)", 400_000, 9_000_000),
            track("/m/e.mp3", "Ninho", "Jefe", 200_000, 5_000_000),
        ];
        let groups = duplicates(&tracks);
        assert_eq!(groups.len(), 1, "{groups:?}");
        let ids: Vec<&str> = groups[0].items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["/m/b.flac", "/m/a.mp3", "/m/c.webm"]);
        assert!(groups[0].mixed, "audio et clip : pas de conseil");

        let audio_only = vec![tracks[0].clone(), tracks[1].clone()];
        let g = &duplicates(&audio_only)[0];
        assert!(!g.mixed);
        assert_eq!(g.items.iter().find(|i| i.keep).map(|i| i.id.as_str()), Some("/m/b.flac"), "le FLAC est gardé");
    }

    #[test]
    fn un_m3u_fait_l_aller_retour() {
        let a = track("/musique/Ninho - Jefe.mp3", "Ninho", "Jefe", 200_000, 1);
        let b = track("/musique/clips/Meridian.webm", "Tiakola", "Meridian", 255_000, 1);
        let text = to_m3u("Soirée", &[&a, &b]);
        let (name, entries) = parse_m3u(&text, Path::new("/ailleurs"));
        assert_eq!(name.as_deref(), Some("Soirée"));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label.as_deref(), Some("Ninho - Jefe"));
        let (ids, missing) = match_entries(&entries, &[a.clone(), b.clone()]);
        assert_eq!((ids.len(), missing), (2, 0));
    }

    #[test]
    fn un_m3u_d_un_autre_ordinateur_est_retrouve() {
        let a = track("/home/moi/Musique/Ninho - Jefe.mp3", "Ninho", "Jefe", 200_000, 1);
        let b = track("/home/moi/Musique/autre.mp3", "Tiakola x Dave", "Meridian", 255_000, 1);
        let text = "\u{feff}#EXTM3U\r\n#EXTINF:200,Ninho - Jefe\r\nC:\\Users\\Toi\\Music\\Ninho - Jefe.mp3\r\n\
                    #EXTINF:255,Tiakola - Meridian\r\nD:\\Son\\meridian_final.mp3\r\n\
                    https://radio.example/flux\r\nfile:///inconnu/Pas%20l%C3%A0.mp3\r\n";
        let (_, entries) = parse_m3u(text, Path::new("/tmp"));
        assert_eq!(entries.len(), 3, "l'adresse web est ignorée");
        assert_eq!(entries[2].path, PathBuf::from("/inconnu/Pas là.mp3"));
        let (ids, missing) = match_entries(&entries, &[a, b]);
        assert_eq!(ids, ["/home/moi/Musique/Ninho - Jefe.mp3", "/home/moi/Musique/autre.mp3"]);
        assert_eq!(missing, 1);
    }

    #[test]
    fn un_chemin_relatif_part_du_dossier_du_fichier() {
        let (_, e) = parse_m3u("sons/a.mp3\n", Path::new("/home/moi/Playlists"));
        assert_eq!(e[0].path, PathBuf::from("/home/moi/Playlists/sons/a.mp3"));
    }
}
