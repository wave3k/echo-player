//! Pochettes cherchées en ligne, pour les morceaux qui n'en ont pas (ou seulement une
//! image tirée d'un clip).
//!
//! MusicBrainz d'abord : il connaît chaque sortie d'un morceau (album, single, EP,
//! compilation) et sa date, et le Cover Art Archive en fournit l'image. Deezer en
//! secours, plus complet sur les sorties récentes. Une pochette n'est posée toute seule
//! que si le résultat ressemble vraiment au morceau : une mauvaise pochette est pire
//! qu'aucune. Les réponses sont gardées en cache, « introuvable » compris.

use crate::lyrics::{artist_variants, fold, get_json, title_variants, tokens};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Pochette voulue : celle de l'album ou celle du single.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefer {
    Album,
    Single,
}

impl Prefer {
    pub fn as_str(self) -> &'static str {
        match self {
            Prefer::Album => "album",
            Prefer::Single => "single",
        }
    }
}

/// Où chercher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// L'album des tags, puis le morceau.
    Full,
    /// L'album seulement : sa pochette vaudra pour tous ses morceaux.
    AlbumOnly,
    /// Le morceau seulement (l'album des tags est introuvable en ligne).
    TrackOnly,
}

/// Ce qu'on cherche.
pub struct Want {
    pub artist: String,
    pub title: String,
    /// Album des tags, nettoyé (vide s'il n'apprend rien, voir `clean_album`).
    pub album: String,
    pub duration_ms: u64,
    /// Clip vidéo : sa durée (intro, scènes) ne départage pas les versions.
    pub video: bool,
    pub prefer: Prefer,
    pub scope: Scope,
}

/// Album des tags, vide quand il n'apprend rien : identique au titre, ou valeur
/// générique d'un téléchargement (« YouTube », « Unknown Album »…).
pub fn clean_album(album: &str, title: &str) -> String {
    let a = fold(album);
    let generic = ["", "youtube", "unknown", "unknown album", "album inconnu", "inconnu", "single", "singles", "music", "musique"];
    if generic.contains(&a.as_str()) || a == fold(title) {
        return String::new();
    }
    album.trim().to_string()
}

/// Réglage « auto » : l'album des tags s'il y en a un, sinon le single. Un album
/// nommé « … - Single » (iTunes) est un single.
pub fn resolve(setting: &str, album: &str, title: &str) -> Prefer {
    match setting {
        "album" => Prefer::Album,
        "single" => Prefer::Single,
        _ => {
            let a = fold(album);
            if clean_album(album, title).is_empty() || a.ends_with(" single") || a.ends_with(" ep") {
                Prefer::Single
            } else {
                Prefer::Album
            }
        }
    }
}

/// Une pochette possible.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    /// "musicbrainz" ou "deezer".
    pub source: String,
    /// "album", "ep", "single" ou "other".
    pub kind: String,
    /// Titre de la sortie (album ou single).
    pub title: String,
    pub artist: String,
    pub year: String,
    /// Adresse de l'image en 500 px, et en 250 px pour l'aperçu.
    pub full: String,
    pub small: String,
    /// Aperçu téléchargé sur le disque (fenêtre de choix).
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(skip)]
    score: f64,
    #[serde(skip)]
    date: String,
    /// Sortie Deezer dont le type n'est pas encore connu.
    #[serde(skip)]
    deezer_album: Option<u64>,
}

/* ---------------- ressemblance ---------------- */

fn uniq(words: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words {
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// Mots d'un titre sans invités ni parenthèses.
fn bare_words(title: &str) -> Vec<String> {
    uniq(tokens(title_variants(title).last().map(String::as_str).unwrap_or("")))
}

/// Ressemblance d'un titre trouvé avec le titre cherché (0 à 1) : les mots cherchés
/// doivent y être, et chaque mot en trop (« Remix », « Acoustique »…) coûte.
fn title_score(wanted: &str, found: &str, artist: &str) -> f64 {
    let want = bare_words(wanted);
    let got = bare_words(found);
    if want.is_empty() || got.is_empty() {
        return 0.0;
    }
    let artist_words: Vec<String> = artist_variants(artist).iter().flat_map(|a| tokens(a)).collect();
    let contained = want.iter().filter(|w| got.contains(w)).count() as f64 / want.len() as f64;
    let extra = got.iter().filter(|w| !want.contains(w) && !artist_words.contains(w)).count() as f64;
    (contained - 0.12 * extra.min(5.0)).max(0.0)
}

/// L'artiste trouvé est bien le nôtre, en comparant sans espaces ni accents :
/// « Tiakola x Dave » ≈ « Dave & Tiakola » ≈ « Dave ».
fn artist_ok(ours: &str, theirs: &str) -> bool {
    let compact = |t: &str| fold(t).replace(' ', "");
    let t = compact(theirs);
    let full = compact(ours);
    if t.is_empty() || full.is_empty() {
        return false;
    }
    artist_variants(ours).iter().map(|a| compact(a)).filter(|a| a.len() >= 2).any(|a| t.contains(&a))
        || (t.len() >= 3 && full.contains(&t))
}

/// Pénalité de durée : rien à 3 s près, un peu jusqu'à 8 s, `None` au-delà (autre version).
fn duration_penalty(want: &Want, found_ms: u64) -> Option<f64> {
    if want.video || want.duration_ms == 0 || found_ms == 0 {
        return Some(0.0);
    }
    let diff = want.duration_ms.abs_diff(found_ms);
    if diff <= 3000 {
        Some(0.0)
    } else if diff <= 8000 {
        Some(0.1)
    } else {
        None
    }
}

/// Rang d'un type de sortie selon la préférence (0 = celui qu'on veut).
fn kind_rank(kind: &str, prefer: Prefer) -> u8 {
    match (prefer, kind) {
        (Prefer::Album, "album") | (Prefer::Single, "single") => 0,
        (_, "ep") => 1,
        (Prefer::Album, "single") | (Prefer::Single, "album") => 2,
        _ => 3,
    }
}

/// Seuil pour poser une pochette toute seule.
const AUTO: f64 = 0.8;
/// Seuil pour la proposer dans la fenêtre de choix.
const LOOSE: f64 = 0.45;

/// Meilleures d'abord : ressemblance (par paliers), type voulu, sortie la plus ancienne
/// (l'originale plutôt qu'une réédition), MusicBrainz avant Deezer à égalité.
fn rank(list: &mut [Candidate], prefer: Prefer) {
    list.sort_by(|a, b| {
        let bucket = |c: &Candidate| -((c.score * 10.0).round() as i64);
        let date = |c: &Candidate| if c.date.is_empty() { "9999".to_string() } else { c.date.clone() };
        bucket(a)
            .cmp(&bucket(b))
            .then(kind_rank(&a.kind, prefer).cmp(&kind_rank(&b.kind, prefer)))
            .then(date(a).cmp(&date(b)))
            .then((a.source != "musicbrainz").cmp(&(b.source != "musicbrainz")))
    });
}

/// Une même image (même sortie) n'apparaît qu'une fois, avec son meilleur score.
fn dedupe(list: Vec<Candidate>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for c in list {
        match out.iter_mut().find(|o| o.full == c.full) {
            Some(o) => {
                if c.score > o.score {
                    o.score = c.score;
                }
                if !c.date.is_empty() && (o.date.is_empty() || c.date < o.date) {
                    o.date = c.date.clone();
                    o.year = c.year.clone();
                }
            }
            None => out.push(c),
        }
    }
    out
}

/* ---------------- MusicBrainz ---------------- */

const MB: &str = "https://musicbrainz.org/ws/2";
const CAA: &str = "https://coverartarchive.org";

/// MusicBrainz demande une requête par seconde au plus : chaque appel attend son tour.
static NEXT_MB: Mutex<Option<Instant>> = Mutex::new(None);

async fn mb_turn() {
    let wait = {
        let mut next = NEXT_MB.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let at = next.map_or(now, |t| t.max(now));
        *next = Some(at + Duration::from_millis(1100));
        at - now
    };
    tokio::time::sleep(wait).await;
}

/// Expression exacte pour la recherche de MusicBrainz (guillemets échappés).
fn phrase(text: &str) -> String {
    format!("\"{}\"", text.replace(['\\', '"'], " "))
}

fn null_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct MbCredit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: Option<String>,
}

fn credit(list: &[MbCredit]) -> String {
    list.iter().map(|c| format!("{}{}", c.name, c.joinphrase.as_deref().unwrap_or(""))).collect::<String>().trim().to_string()
}

#[derive(Deserialize)]
struct MbGroup {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "primary-type", default)]
    primary: Option<String>,
    #[serde(rename = "secondary-types", default, deserialize_with = "null_vec")]
    secondary: Vec<String>,
    #[serde(rename = "first-release-date", default)]
    first: Option<String>,
    #[serde(rename = "artist-credit", default, deserialize_with = "null_vec")]
    artist_credit: Vec<MbCredit>,
}

impl MbGroup {
    /// "album", "ep", "single", ou `None` pour ce qu'on ne veut jamais (compilation,
    /// live, remix : la pochette n'est pas celle du morceau).
    fn kind(&self) -> Option<&'static str> {
        let skip = ["Compilation", "Live", "Remix", "DJ-mix", "Interview", "Spokenword", "Audiobook"];
        if self.secondary.iter().any(|s| skip.contains(&s.as_str())) {
            return None;
        }
        match self.primary.as_deref() {
            Some("Album") => Some("album"),
            Some("EP") => Some("ep"),
            Some("Single") => Some("single"),
            _ => Some("other"),
        }
    }
}

#[derive(Deserialize)]
struct MbRelease {
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "release-group", default)]
    group: Option<MbGroup>,
}

#[derive(Deserialize)]
struct MbRecording {
    #[serde(default)]
    title: String,
    #[serde(default)]
    length: Option<u64>,
    #[serde(rename = "artist-credit", default, deserialize_with = "null_vec")]
    artist_credit: Vec<MbCredit>,
    #[serde(default, deserialize_with = "null_vec")]
    releases: Vec<MbRelease>,
}

#[derive(Deserialize)]
struct MbRecordings {
    #[serde(default, deserialize_with = "null_vec")]
    recordings: Vec<MbRecording>,
}

#[derive(Deserialize)]
struct MbGroups {
    #[serde(rename = "release-groups", default, deserialize_with = "null_vec")]
    groups: Vec<MbGroup>,
}

fn year_of(date: &str) -> String {
    date.get(..4).unwrap_or("").to_string()
}

fn mb_candidate(group: &MbGroup, title: String, artist: String, date: String, score: f64) -> Option<Candidate> {
    let kind = group.kind()?;
    Some(Candidate {
        source: "musicbrainz".into(),
        kind: kind.into(),
        title,
        artist,
        year: year_of(&date),
        full: format!("{CAA}/release-group/{}/front-500", group.id),
        small: format!("{CAA}/release-group/{}/front-250", group.id),
        thumb: None,
        score,
        date,
        deezer_album: None,
    })
}

/// Morceaux du même nom chez MusicBrainz, et chaque sortie où ils apparaissent.
async fn mb_recordings(client: &reqwest::Client, want: &Want, strict: bool) -> Result<Vec<Candidate>, String> {
    let mut out = Vec::new();
    let artists = artist_variants(&want.artist);
    let titles = title_variants(&want.title);
    let mut tried = 0;
    for t in titles.iter().rev() {
        for a in artists.iter().rev() {
            if tried >= 3 {
                return Ok(out);
            }
            tried += 1;
            let query = format!("recording:{} AND artist:{}", phrase(t), phrase(a));
            let url = reqwest::Url::parse_with_params(&format!("{MB}/recording"), &[("query", query.as_str()), ("fmt", "json"), ("limit", "20")])
                .map_err(|e| e.to_string())?;
            mb_turn().await;
            let found = get_json::<MbRecordings>(client, url).await?.map(|r| r.recordings).unwrap_or_default();
            for rec in &found {
                let artist = credit(&rec.artist_credit);
                if !artist_ok(&want.artist, &artist) {
                    continue;
                }
                let Some(penalty) = duration_penalty(want, rec.length.unwrap_or(0)).or(if strict { None } else { Some(0.3) }) else {
                    continue;
                };
                let score = title_score(&want.title, &rec.title, &want.artist) - penalty;
                for rel in &rec.releases {
                    if matches!(rel.status.as_deref(), Some("Bootleg") | Some("Pseudo-Release")) {
                        continue;
                    }
                    let Some(group) = &rel.group else { continue };
                    let date = rel.date.clone().unwrap_or_default();
                    let title = group.title.clone().unwrap_or_else(|| rel.title.clone());
                    out.extend(mb_candidate(group, title, artist.clone(), date, score));
                }
            }
            if out.iter().any(|c| c.score >= AUTO) {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// Albums de ce nom chez MusicBrainz (quand les tags donnent l'album).
async fn mb_albums(client: &reqwest::Client, want: &Want) -> Result<Vec<Candidate>, String> {
    let artist = artist_variants(&want.artist).pop().unwrap_or_default();
    let query = format!("releasegroup:{} AND artist:{}", phrase(&want.album), phrase(&artist));
    let url = reqwest::Url::parse_with_params(&format!("{MB}/release-group/"), &[("query", query.as_str()), ("fmt", "json"), ("limit", "10")])
        .map_err(|e| e.to_string())?;
    mb_turn().await;
    let groups = get_json::<MbGroups>(client, url).await?.map(|g| g.groups).unwrap_or_default();
    Ok(groups
        .iter()
        .filter_map(|g| {
            let artist = credit(&g.artist_credit);
            if !artist_ok(&want.artist, &artist) {
                return None;
            }
            let title = g.title.clone().unwrap_or_default();
            let score = title_score(&want.album, &title, &want.artist);
            mb_candidate(g, title, artist, g.first.clone().unwrap_or_default(), score)
        })
        .collect())
}

/* ---------------- Deezer ---------------- */

const DEEZER: &str = "https://api.deezer.com";

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct DzList<T> {
    #[serde(default = "Vec::new", deserialize_with = "null_vec")]
    data: Vec<T>,
    /// Quota dépassé : Deezer répond 200 avec une erreur.
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct DzArtist {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct DzAlbum {
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cover_big: Option<String>,
    #[serde(default)]
    cover_medium: Option<String>,
    /// Vide quand l'album n'a pas de pochette (Deezer montre alors une image grise).
    #[serde(default)]
    md5_image: Option<String>,
    #[serde(default)]
    record_type: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    artist: Option<DzArtist>,
}

#[derive(Deserialize)]
struct DzTrack {
    #[serde(default)]
    title: String,
    #[serde(default)]
    duration: u64,
    artist: DzArtist,
    album: DzAlbum,
}

fn dz_kind(record_type: Option<&str>) -> &'static str {
    match record_type {
        Some("album") => "album",
        Some("ep") => "ep",
        Some("single") => "single",
        _ => "other",
    }
}

fn dz_candidate(album: &DzAlbum, artist: String, kind: &str, score: f64) -> Option<Candidate> {
    if album.md5_image.as_deref().is_some_and(str::is_empty) {
        return None;
    }
    let full = album.cover_big.clone().filter(|u| !u.is_empty())?;
    let date = album.release_date.clone().unwrap_or_default();
    Some(Candidate {
        source: "deezer".into(),
        kind: kind.into(),
        title: album.title.clone(),
        artist,
        year: year_of(&date),
        small: album.cover_medium.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| full.clone()),
        full,
        thumb: None,
        score,
        date,
        deezer_album: Some(album.id),
    })
}

async fn dz_get<T: serde::de::DeserializeOwned>(client: &reqwest::Client, path: &str, q: &str) -> Result<Vec<T>, String> {
    let url = reqwest::Url::parse_with_params(&format!("{DEEZER}/{path}"), &[("q", q), ("limit", "15")]).map_err(|e| e.to_string())?;
    match get_json::<DzList<T>>(client, url).await? {
        Some(list) if list.error.is_some() => Err("deezer : quota".into()),
        Some(list) => Ok(list.data),
        None => Ok(Vec::new()),
    }
}

/// Morceaux du même nom sur Deezer. Le type de sortie (album, single) n'est pas dans
/// la réponse : il est deviné, puis vérifié pour les meilleurs (`settle_deezer`).
async fn dz_tracks(client: &reqwest::Client, want: &Want, strict: bool) -> Result<Vec<Candidate>, String> {
    let artist = artist_variants(&want.artist).pop().unwrap_or_default();
    let title = title_variants(&want.title).pop().unwrap_or_default();
    let found: Vec<DzTrack> = dz_get(client, "search", &format!("{artist} {title}")).await?;
    Ok(found
        .iter()
        .filter(|t| artist_ok(&want.artist, &t.artist.name))
        .filter_map(|t| {
            let penalty = duration_penalty(want, t.duration * 1000).or(if strict { None } else { Some(0.3) })?;
            let score = title_score(&want.title, &t.title, &want.artist) - penalty;
            let guess = if fold(&t.album.title) == fold(&t.title) { "single" } else { "album" };
            dz_candidate(&t.album, t.artist.name.clone(), guess, score)
        })
        .collect())
}

/// Albums de ce nom sur Deezer.
async fn dz_albums(client: &reqwest::Client, want: &Want) -> Result<Vec<Candidate>, String> {
    let artist = artist_variants(&want.artist).pop().unwrap_or_default();
    let found: Vec<DzAlbum> = dz_get(client, "search/album", &format!("{artist} {}", want.album)).await?;
    Ok(found
        .iter()
        .filter_map(|a| {
            let name = a.artist.as_ref().map(|x| x.name.clone()).unwrap_or_default();
            if !artist_ok(&want.artist, &name) {
                return None;
            }
            let score = title_score(&want.album, &a.title, &want.artist);
            dz_candidate(a, name, dz_kind(a.record_type.as_deref()), score)
        })
        .collect())
}

/// Type et date exacts des premières sorties Deezer (une requête par album, trois au plus).
async fn settle_deezer(client: &reqwest::Client, list: &mut [Candidate]) {
    let mut asked = 0;
    for c in list.iter_mut() {
        let Some(id) = c.deezer_album.take() else { continue };
        if asked >= 3 || c.score < LOOSE {
            continue;
        }
        asked += 1;
        let Ok(url) = reqwest::Url::parse(&format!("{DEEZER}/album/{id}")) else { continue };
        if let Ok(Some(album)) = get_json::<DzAlbum>(client, url).await {
            c.kind = dz_kind(album.record_type.as_deref()).into();
            if let Some(date) = album.release_date.filter(|d| !d.is_empty()) {
                c.year = year_of(&date);
                c.date = date;
            }
        }
    }
}

/* ---------------- recherche ---------------- */

/// Toutes les pochettes possibles, les meilleures d'abord. `thorough` : on interroge
/// toutes les sources (fenêtre de choix) au lieu de s'arrêter à la première qui convient.
async fn gather(client: &reqwest::Client, want: &Want, thorough: bool) -> Result<Vec<Candidate>, String> {
    gather_all(client, want, thorough).await.map(|(list, _)| list)
}

/// Comme `gather`, et vrai en plus si une source n'a pas répondu (résultat partiel).
async fn gather_all(client: &reqwest::Client, want: &Want, thorough: bool) -> Result<(Vec<Candidate>, bool), String> {
    let strict = !thorough;
    let floor = if thorough { LOOSE } else { AUTO };
    let mut list: Vec<Candidate> = Vec::new();
    let mut failure: Option<String> = None;
    let good = |list: &[Candidate]| list.iter().any(|c| c.score >= AUTO && kind_rank(&c.kind, want.prefer) == 0);

    let album_first = !want.album.is_empty() && want.prefer == Prefer::Album && want.scope != Scope::TrackOnly;
    if album_first {
        match mb_albums(client, want).await {
            Ok(found) => list.extend(found),
            Err(e) => failure = Some(e),
        }
        if want.scope == Scope::AlbumOnly && !good(&list) {
            match dz_albums(client, want).await {
                Ok(found) => list.extend(found),
                Err(e) => failure = Some(e),
            }
        }
    }
    if want.scope == Scope::AlbumOnly {
        // Seuls les albums de ce nom comptent : un résultat par morceau ne vaudrait pas pour les autres.
        list.retain(|c| c.score >= floor && c.kind != "single");
        let mut list = dedupe(list);
        rank(&mut list, want.prefer);
        return match failure {
            Some(e) if list.is_empty() => Err(e),
            _ => Ok((list, failure.is_some())),
        };
    }
    if thorough || !good(&list) {
        match mb_recordings(client, want, strict).await {
            Ok(found) => list.extend(found),
            Err(e) => failure = Some(e),
        }
    }
    if thorough || !good(&list) {
        if album_first {
            match dz_albums(client, want).await {
                Ok(found) => list.extend(found),
                Err(e) => failure = Some(e),
            }
        }
        match dz_tracks(client, want, strict).await {
            Ok(mut found) => {
                rank(&mut found, want.prefer);
                settle_deezer(client, &mut found).await;
                list.extend(found);
            }
            Err(e) => failure = Some(e),
        }
    }
    list.retain(|c| c.score >= floor);
    let mut list = dedupe(list);
    rank(&mut list, want.prefer);
    // Rien de sûr alors qu'une source n'a pas répondu : on ne conclut pas.
    if list.is_empty() {
        if let Some(e) = failure {
            return Err(e);
        }
    }
    Ok((list, failure.is_some()))
}

/// Télécharge une image (8 Mo au plus) et vérifie que c'en est bien une.
/// `None` : l'image n'existe pas (pas de pochette pour cette sortie).
async fn fetch_image(client: &reqwest::Client, url: &str) -> Result<Option<(Vec<u8>, &'static str)>, String> {
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if res.status().as_u16() == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("image : {}", res.status()));
    }
    // Après redirection (le Cover Art Archive renvoie vers archive.org), l'image doit
    // toujours venir d'un des services connus.
    if !image_host(res.url()) {
        return Ok(None);
    }
    let Some(bytes) = crate::lyrics::read_capped(res, 8 * 1024 * 1024).await? else { return Ok(None) };
    if bytes.len() < 200 {
        return Ok(None);
    }
    let ext = match image::guess_format(&bytes) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::Png) => "png",
        Ok(image::ImageFormat::WebP) => "webp",
        _ => return Ok(None),
    };
    Ok(Some((bytes, ext)))
}

/// Hôtes d'où une image de pochette peut venir.
fn image_host(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|h| {
            h == "coverartarchive.org" || h == "archive.org" || h.ends_with(".archive.org") || h == "dzcdn.net" || h.ends_with(".dzcdn.net")
        })
}

/// Range l'image dans le cache des pochettes, sous un nom tiré de son contenu.
fn store_image(covers: &Path, bytes: &[u8], ext: &str) -> Option<String> {
    let name = format!("{:016x}.{ext}", crate::library::stable_hash(bytes));
    let dest = covers.join(&name);
    if !dest.is_file() {
        crate::store::write_atomic(&dest, bytes).ok()?;
    }
    Some(name)
}

/* ---------------- cache des réponses ---------------- */

#[derive(Serialize, Deserialize)]
struct Cached {
    at: u64,
    /// Nom de la pochette dans le cache, ou `None` : rien de sûr trouvé.
    cover: Option<String>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Un « introuvable » est redemandé au bout d'un mois (les bases s'enrichissent).
const RETRY_MISSING: u64 = 30 * 24 * 3600;

fn cache_path(cache_dir: &Path, want: &Want) -> PathBuf {
    let key = format!(
        "v1\n{}\n{}\n{}\n{}\n{:?}\n{}",
        fold(&want.artist),
        if want.scope == Scope::AlbumOnly { String::new() } else { fold(&want.title) },
        fold(&want.album),
        want.prefer.as_str(),
        want.scope,
        if want.video || want.scope == Scope::AlbumOnly { 0 } else { want.duration_ms / 2000 }
    );
    cache_dir.join("online-covers").join(format!("{:016x}.json", crate::library::stable_hash(key.as_bytes())))
}

/// Ce que la dernière recherche avait conclu, si c'est encore valable :
/// `Some(Some(nom))` trouvée, `Some(None)` introuvable, `None` à chercher.
fn cached(cache_dir: &Path, covers: &Path, want: &Want) -> Option<Option<String>> {
    let c: Cached = serde_json::from_slice(&std::fs::read(cache_path(cache_dir, want)).ok()?).ok()?;
    match c.cover {
        // Image effacée (cache vidé) : on la retélécharge.
        Some(name) => covers.join(&name).is_file().then_some(Some(name)),
        None => (now().saturating_sub(c.at) <= RETRY_MISSING).then_some(None),
    }
}

fn remember(cache_dir: &Path, want: &Want, cover: Option<&str>) {
    let path = cache_path(cache_dir, want);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec(&Cached { at: now(), cover: cover.map(str::to_string) }) {
        let _ = crate::store::write_atomic(&path, &bytes);
    }
}

/// La pochette du morceau, si une sortie lui ressemble assez : son nom dans le cache
/// des pochettes. `Err` : réseau absent ou service en panne (rien n'est retenu).
pub async fn lookup(cache_dir: &Path, covers: &Path, want: &Want) -> Result<Option<String>, String> {
    if want.artist.trim().is_empty() || want.title.trim().is_empty() {
        return Ok(None);
    }
    if let Some(hit) = cached(cache_dir, covers, want) {
        return Ok(hit);
    }
    let client = crate::lyrics::client()?;
    let (list, partial) = gather_all(&client, want, false).await?;
    let mut found = None;
    let mut image_failed = false;
    for c in list.iter().take(4) {
        // Sortie sans image, ou image en panne : on passe à la suivante.
        match fetch_image(&client, &c.full).await {
            Ok(Some((bytes, ext))) => {
                found = store_image(covers, &bytes, ext);
                if found.is_some() {
                    break;
                }
            }
            Ok(None) => {}
            Err(_) => image_failed = true,
        }
    }
    // « Introuvable » n'est retenu que si toutes les sources ont répondu : sinon on
    // réessaiera à la prochaine passe.
    if found.is_some() || (!partial && !image_failed) {
        remember(cache_dir, want, found.as_deref());
    }
    Ok(found)
}

/// Pochettes proposées dans la fenêtre de choix (8 au plus), avec leur aperçu
/// téléchargé dans `covers/candidates` (la page ne charge rien depuis Internet).
pub async fn candidates(covers: &Path, want: &Want) -> Result<Vec<Candidate>, String> {
    let client = crate::lyrics::client()?;
    let mut list = gather(&client, want, true).await?;
    // La même sortie chez les deux services (même nom, même type) : une seule fois,
    // la mieux placée (MusicBrainz à égalité).
    let mut seen = std::collections::HashSet::new();
    list.retain(|c| seen.insert((fold(&c.title), c.kind.clone())));
    let dir = covers.join("candidates");
    let _ = std::fs::create_dir_all(&dir);
    let mut tasks = Vec::new();
    for c in list.into_iter().take(10) {
        let client = client.clone();
        let dir = dir.clone();
        tasks.push(tauri::async_runtime::spawn(async move {
            let (bytes, ext) = fetch_image(&client, &c.small).await.ok()??;
            let name = format!("{:016x}.{ext}", crate::library::stable_hash(&bytes));
            let dest = dir.join(name);
            if !dest.is_file() {
                crate::store::write_atomic(&dest, &bytes).ok()?;
            }
            Some(Candidate { thumb: Some(dest.to_string_lossy().into_owned()), ..c })
        }));
    }
    let mut out = Vec::new();
    for task in tasks {
        if let Ok(Some(c)) = task.await {
            out.push(c);
        }
    }
    out.truncate(8);
    Ok(out)
}

/// Seules les images de ces deux services peuvent être posées depuis la fenêtre de choix.
fn trusted(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    u.scheme() == "https"
        && match u.host_str() {
            Some(h) => h == "coverartarchive.org" || h == "dzcdn.net" || h.ends_with(".dzcdn.net"),
            None => false,
        }
}

/// Télécharge la pochette choisie dans la fenêtre et la range dans le cache.
pub async fn download(covers: &Path, url: &str) -> Result<String, String> {
    if !trusted(url) {
        return Err("Adresse d'image refusée.".into());
    }
    let client = crate::lyrics::client()?;
    let (bytes, ext) = fetch_image(&client, url).await?.ok_or("Cette pochette n'est plus disponible.")?;
    store_image(covers, &bytes, ext).ok_or_else(|| "Impossible d'enregistrer la pochette.".into())
}

/// Aperçus d'une fenêtre de choix précédente : effacés quand ils ont plus d'un jour.
pub fn prune_candidates(covers: &Path) {
    let Ok(read) = std::fs::read_dir(covers.join("candidates")) else { return };
    for entry in read.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(24 * 3600));
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(artist: &str, title: &str, album: &str, prefer: Prefer) -> Want {
        Want { artist: artist.into(), title: title.into(), album: album.into(), duration_ms: 200_000, video: false, prefer, scope: Scope::Full }
    }

    fn cand(kind: &str, score: f64, date: &str, source: &str) -> Candidate {
        Candidate {
            source: source.into(),
            kind: kind.into(),
            title: String::new(),
            artist: String::new(),
            year: year_of(date),
            full: format!("{kind}{date}{source}"),
            small: String::new(),
            thumb: None,
            score,
            date: date.into(),
            deezer_album: None,
        }
    }

    #[test]
    fn l_album_des_tags_est_nettoye() {
        assert_eq!(clean_album("YouTube", "Bécane"), "");
        assert_eq!(clean_album("Bécane", "Becane"), "");
        assert_eq!(clean_album(" Jefe ", "Lettre à une femme"), "Jefe");
        assert_eq!(resolve("auto", "Jefe", "Lettre à une femme"), Prefer::Album);
        assert_eq!(resolve("auto", "", "Meridian"), Prefer::Single);
        assert_eq!(resolve("auto", "Meridian - Single", "Meridian"), Prefer::Single);
        assert_eq!(resolve("single", "Jefe", "Lettre à une femme"), Prefer::Single);
        assert_eq!(resolve("album", "", "Meridian"), Prefer::Album);
    }

    #[test]
    fn les_artistes_se_reconnaissent() {
        assert!(artist_ok("Tiakola x Dave", "Dave & Tiakola"));
        assert!(artist_ok("Tiakola x Dave", "Dave"));
        assert!(artist_ok("KOBA LA D", "Koba LaD"));
        assert!(artist_ok("Yamê", "Yame"));
        assert!(!artist_ok("Ninho", "Nina Simone"));
        assert!(!artist_ok("Ninho", ""));
    }

    #[test]
    fn un_remix_ressemble_moins() {
        assert!(title_score("Meridian", "Meridian", "Tiakola") >= 0.99);
        assert!(title_score("Meridian (feat. Dave)", "Meridian", "Tiakola") >= 0.99);
        assert!(title_score("Meridian", "Meridian (Remix)", "Tiakola") >= 0.99, "parenthèses ignorées");
        assert!(title_score("Meridian", "Meridian Club Remix", "Tiakola") < AUTO);
        assert!(title_score("Meridian", "Autre chose", "Tiakola") < LOOSE);
    }

    #[test]
    fn la_duree_ecarte_une_autre_version() {
        let w = want("A", "T", "", Prefer::Single);
        assert_eq!(duration_penalty(&w, 201_000), Some(0.0));
        assert_eq!(duration_penalty(&w, 206_000), Some(0.1));
        assert_eq!(duration_penalty(&w, 260_000), None);
        let clip = Want { video: true, ..want("A", "T", "", Prefer::Single) };
        assert_eq!(duration_penalty(&clip, 260_000), Some(0.0));
    }

    #[test]
    fn la_preference_departage_les_sorties() {
        let mut list = vec![
            cand("album", 1.0, "2021-12-03", "musicbrainz"),
            cand("single", 1.0, "2021-10-01", "musicbrainz"),
            cand("album", 1.0, "2023-01-01", "musicbrainz"),
            cand("single", 0.7, "2019-01-01", "deezer"),
        ];
        rank(&mut list, Prefer::Album);
        assert_eq!((list[0].kind.as_str(), list[0].date.as_str()), ("album", "2021-12-03"), "album original avant la réédition");
        rank(&mut list, Prefer::Single);
        assert_eq!(list[0].kind, "single");
        assert_eq!(list[0].score, 1.0, "la ressemblance passe avant le type");
    }

    #[test]
    fn seules_les_images_connues_sont_acceptees() {
        assert!(trusted("https://coverartarchive.org/release-group/x/front-500"));
        assert!(trusted("https://cdn-images.dzcdn.net/images/cover/a/500x500.jpg"));
        assert!(!trusted("http://coverartarchive.org/x"));
        assert!(!trusted("https://example.com/dzcdn.net.jpg"));
        assert!(!trusted("file:///etc/passwd"));
    }

    /// Essai réel : cargo test covers_online::essai_reel -- --ignored --nocapture
    #[test]
    #[ignore]
    fn essai_reel() {
        let dir = std::env::temp_dir().join("echo-covers-essai");
        let covers = dir.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        let cases = [
            ("Tiakola x Dave", "Meridian", "", 255_000, true, Prefer::Single),
            ("Ninho", "Lettre à une femme", "Destin", 0, false, Prefer::Album),
            ("Ninho", "Jefe", "", 0, false, Prefer::Album),
            ("Daft Punk", "One More Time", "", 320_000, false, Prefer::Single),
            ("Daft Punk", "One More Time", "", 320_000, false, Prefer::Album),
        ];
        for (artist, title, album, ms, video, prefer) in cases {
            let w = Want { artist: artist.into(), title: title.into(), album: album.into(), duration_ms: ms, video, prefer, scope: Scope::Full };
            let client = crate::lyrics::client().unwrap();
            let list = tauri::async_runtime::block_on(gather(&client, &w, true));
            println!("\n== {artist} — {title} ({})", prefer.as_str());
            match list {
                Ok(list) => {
                    for c in list.iter().take(5) {
                        println!("  {:.2} {:7} {:6} {} — {} ({})", c.score, c.kind, c.source, c.artist, c.title, c.year);
                    }
                }
                Err(e) => println!("  erreur : {e}"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
