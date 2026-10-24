//! Paroles d'un morceau, synchronisées quand c'est possible.
//!
//! Ordre de recherche : un fichier .lrc à côté du morceau, les paroles écrites dans
//! le fichier lui-même, puis (si le réglage est actif) LRCLIB, une base libre et
//! gratuite. Les réponses en ligne sont gardées en cache, même « introuvable »,
//! pour ne pas redemander à chaque écoute.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Lyrics {
    /// Lignes horodatées, dans l'ordre (paroles synchronisées).
    pub synced: Option<Vec<Line>>,
    /// Texte brut, quand il n'y a pas de version synchronisée.
    pub plain: Option<String>,
    /// Morceau instrumental selon la source.
    #[serde(default)]
    pub instrumental: bool,
    /// "lrc", "file", "lrclib" ou "none".
    pub source: String,
    /// Durée de la version sur laquelle les paroles sont calées (LRCLIB), en secondes.
    #[serde(default)]
    pub duration_s: f64,
    /// Calages faits à la main pour ce morceau (clips plus longs que le son).
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    /// Morceau retrouvé en ligne (« Artiste — Titre »), pour vérifier d'un coup d'œil.
    #[serde(default)]
    pub matched: String,
}

/// « À tel moment du fichier, on en est à tel moment des paroles. » Chaque calage vaut
/// jusqu'au suivant : un clip coupé par une scène se recale en deux clics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub at_ms: u64,
    pub lyric_ms: u64,
}

impl Lyrics {
    pub fn none() -> Self {
        Self { source: "none".into(), ..Default::default() }
    }
    fn found(&self) -> bool {
        self.synced.is_some() || self.plain.is_some() || self.instrumental
    }
    /// Paroles venues d'un texte : synchronisées s'il est au format LRC.
    fn from_text(text: &str, source: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let lines = parse_lrc(text);
        Some(if lines.len() >= 2 {
            Self { synced: Some(lines), source: source.into(), ..Default::default() }
        } else {
            Self { plain: Some(text.to_string()), source: source.into(), ..Default::default() }
        })
    }
}

/// Format LRC : « [01:23.45]paroles », plusieurs horodatages possibles par ligne,
/// balises d'en-tête ([ar:], [ti:]…) ignorées, [offset:±ms] appliqué.
pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut offset: i64 = 0;
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            rest = &stripped[end + 1..];
            if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse().unwrap_or(0);
            } else if let Some(ms) = parse_stamp(tag) {
                stamps.push(ms);
            }
        }
        let text = rest.trim().to_string();
        for ms in stamps {
            // Un offset positif avance les paroles.
            let at = (ms as i64 - offset).max(0) as u64;
            out.push(Line { ms: at, text: text.clone() });
        }
    }
    out.sort_by_key(|l| l.ms);
    out
}

/// « mm:ss.xx » ou « mm:ss » → millisecondes.
fn parse_stamp(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let s: f64 = s.trim().parse().ok()?;
    if !(0.0..60.0).contains(&s) {
        return None;
    }
    Some(m * 60_000 + (s * 1000.0).round() as u64)
}

/// Paroles trouvées sur l'ordinateur : .lrc voisin, puis tags du fichier.
pub fn local(path: &Path) -> Option<Lyrics> {
    for candidate in [path.with_extension("lrc"), path.with_extension("LRC")] {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            if let Some(l) = Lyrics::from_text(&text, "lrc") {
                return Some(l);
            }
        }
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let embedded = if matches!(ext.as_str(), "webm" | "mkv") {
        crate::matroska::read(path).and_then(|i| i.lyrics)
    } else {
        embedded_lyrics(path)
    };
    embedded.and_then(|t| Lyrics::from_text(&t, "file"))
}

fn embedded_lyrics(path: &Path) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::ItemKey;
    let file = lofty::read_from_path(path).ok()?;
    file.tags()
        .iter()
        .find_map(|tag| tag.get_string(&ItemKey::Lyrics).map(str::to_string))
}

/// Ce qu'on demande à LRCLIB.
pub struct Query {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_ms: u64,
    /// Clip vidéo : souvent plus long que le morceau (intro, scènes), la durée ne départage pas.
    pub video: bool,
}

/// Titre sans « (feat. X) » ni mentions de clip : la recherche trouve mieux.
fn search_title(title: &str) -> String {
    let mut t = title.to_string();
    for marker in ["(feat", "[feat", "(ft.", "[ft."] {
        if let Some((i, _)) = crate::titles::find_ci(&t, marker) {
            t.truncate(i);
        }
    }
    t.trim().to_string()
}

fn cache_path(cache_dir: &Path, q: &Query) -> PathBuf {
    // « v2 » : nouvelle recherche, les « introuvable » de l'ancienne ne comptent plus.
    let key = format!("v2\n{}\n{}\n{}", q.artist.to_lowercase(), q.title.to_lowercase(), q.duration_ms / 1000);
    cache_dir.join("lyrics").join(format!("{:016x}.json", crate::library::stable_hash(key.as_bytes())))
}

#[derive(Serialize, Deserialize)]
struct Cached {
    at: u64,
    lyrics: Lyrics,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Un « introuvable » est redemandé au bout d'une semaine (la base s'enrichit).
const RETRY_MISSING: u64 = 7 * 24 * 3600;

pub fn cached(cache_dir: &Path, q: &Query) -> Option<Lyrics> {
    let c: Cached = serde_json::from_slice(&std::fs::read(cache_path(cache_dir, q)).ok()?).ok()?;
    if !c.lyrics.found() && now().saturating_sub(c.at) > RETRY_MISSING {
        return None;
    }
    Some(c.lyrics)
}

fn store(cache_dir: &Path, q: &Query, lyrics: &Lyrics) {
    let path = cache_path(cache_dir, q);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec(&Cached { at: now(), lyrics: lyrics.clone() }) {
        let _ = std::fs::write(path, bytes);
    }
}

/// Un champ `null` dans une réponse prend sa valeur par défaut, au lieu de faire
/// échouer toute la liste (LRCLIB a des fiches sans durée, par exemple).
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct Remote {
    #[serde(rename = "trackName", default, deserialize_with = "or_default")]
    track: String,
    #[serde(rename = "artistName", default, deserialize_with = "or_default")]
    artist: String,
    #[serde(rename = "plainLyrics", default)]
    plain: Option<String>,
    #[serde(rename = "syncedLyrics", default)]
    synced: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    instrumental: bool,
    #[serde(default, deserialize_with = "or_default")]
    duration: f64,
}

impl Remote {
    fn into_lyrics(self) -> Lyrics {
        let synced = self.synced.as_deref().map(parse_lrc).filter(|l| l.len() >= 2);
        let plain = self.plain.filter(|p| !p.trim().is_empty());
        Lyrics {
            synced,
            plain,
            instrumental: self.instrumental,
            source: "lrclib".into(),
            duration_s: self.duration,
            anchors: Vec::new(),
            matched: format!("{} — {}", self.artist, self.track),
        }
    }
}

const LRCLIB: &str = "https://lrclib.net/api";

/// Cherche sur LRCLIB : d'abord la correspondance exacte, puis une recherche large
/// dont on garde le résultat à la durée la plus proche (à 4 s près).
pub async fn online(cache_dir: &Path, q: &Query) -> Lyrics {
    if let Some(hit) = cached(cache_dir, q) {
        return hit;
    }
    let result = fetch(q).await;
    match result {
        Ok(lyrics) => {
            store(cache_dir, q, &lyrics);
            lyrics
        }
        // Réseau absent : rien n'est gardé, on réessaiera.
        Err(_) => Lyrics::none(),
    }
}

/// Minuscules sans accents ni ponctuation : « Yamê - Bécane » ≈ « yame becane ».
pub(crate) fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ÿ' | 'ý' => 'y',
            // « G.A.N.G » = « GANG », « Hit 'Em Up » = « Hit Em Up ».
            '’' | '\'' | '`' | '.' => continue,
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Mots qui décrivent la vidéo, pas le morceau.
const NOISE: &[&str] = &[
    "official", "officiel", "officielle", "video", "clip", "audio", "lyrics", "lyric", "visualizer", "music", "musique", "hd",
    "4k", "hq", "explicit", "dirty", "clean", "version", "live", "performance", "visual", "mixtape", "feat", "ft", "featuring",
];

pub(crate) fn tokens(text: &str) -> Vec<String> {
    fold(text).split(' ').filter(|w| !w.is_empty() && !NOISE.contains(w)).map(str::to_string).collect()
}

/// Artistes d'un nom comme « Tiakola x Dave » ou « Kendrick Lamar, SZA » : le premier d'abord.
pub(crate) fn artist_variants(artist: &str) -> Vec<String> {
    let mut out = vec![artist.trim().to_string()];
    let mut cut = artist.len();
    for sep in [" x ", ",", " & ", " feat", " ft.", " ft ", " and ", " et ", " vs "] {
        if let Some((i, _)) = crate::titles::find_ci(artist, sep) {
            cut = cut.min(i);
        }
    }
    let first = artist[..cut].trim().to_string();
    if !first.is_empty() && first != out[0] {
        out.push(first);
    }
    out
}

/// Titre sans invités, puis sans aucune parenthèse (« (Live) », « (Dirty) »…).
pub(crate) fn title_variants(title: &str) -> Vec<String> {
    let base = search_title(title);
    let mut bare = String::new();
    let mut depth = 0;
    for c in base.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    let bare = bare.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = vec![base];
    if !bare.is_empty() && bare != out[0] {
        out.push(bare);
    }
    out
}

/// Ressemblance d'un résultat avec le morceau cherché (0 à 1). Le titre doit y être
/// presque en entier ; l'artiste peut être dans le champ artiste ou dans le titre.
fn score(q: &Query, r: &Remote) -> f64 {
    let wanted: Vec<String> = title_variants(&q.title).iter().flat_map(|t| tokens(t)).collect();
    let mut wanted_set: Vec<String> = Vec::new();
    for w in tokens(title_variants(&q.title).last().map(String::as_str).unwrap_or("")) {
        if !wanted_set.contains(&w) {
            wanted_set.push(w);
        }
    }
    if wanted_set.is_empty() {
        wanted_set = wanted;
    }
    let found = tokens(&format!("{} {}", r.track, r.artist));
    let title_found = tokens(&r.track);
    if wanted_set.is_empty() || title_found.is_empty() {
        return 0.0;
    }
    let contained = wanted_set.iter().filter(|w| found.contains(w)).count() as f64 / wanted_set.len() as f64;
    // Mots du titre trouvé qui ne sont ni le morceau ni l'artiste : un autre morceau ?
    let artist_tokens: Vec<String> = artist_variants(&q.artist).iter().flat_map(|a| tokens(a)).collect();
    let extra = title_found.iter().filter(|w| !wanted_set.contains(w) && !artist_tokens.contains(w)).count() as f64;
    let title_score = contained - 0.08 * extra.min(5.0);
    // L'artiste doit s'y retrouver : sinon, c'est la chanson de quelqu'un d'autre.
    if !artist_matches(q, r) {
        return 0.0;
    }
    (0.75 * title_score + 0.25).max(0.0)
}

/// L'artiste du morceau se retrouve dans le résultat (champ artiste ou titre), en
/// comparant sans espaces : « KOBA LA D » = « Koba LaD », « Tiakola x Dave » ⊃ « Tiakola ».
fn artist_matches(q: &Query, r: &Remote) -> bool {
    let compact = |t: &str| fold(t).replace(' ', "");
    let theirs = [compact(&r.artist), compact(&r.track)];
    let ours_full = compact(&q.artist);
    artist_variants(&q.artist).iter().map(|a| compact(a)).filter(|a| a.len() >= 2).any(|a| {
        theirs.iter().any(|t| t.contains(&a)) || (!theirs[0].is_empty() && theirs[0].len() >= 3 && ours_full.contains(&theirs[0]))
    }) || fold(&r.artist).split(' ').filter(|w| w.len() >= 3).any(|w| ours_full.contains(w) && w.len() >= 4)
}

/// Le meilleur résultat, s'il ressemble assez : synchronisé d'abord, puis le plus proche
/// en durée (pour un fichier audio, à 4 s près ; un clip accepte une autre durée).
fn pick(q: &Query, found: Vec<Remote>) -> Option<Remote> {
    let target = q.duration_ms as f64 / 1000.0;
    found
        .into_iter()
        .filter(|r| r.synced.is_some() || r.plain.is_some() || r.instrumental)
        .filter(|r| q.video || q.duration_ms == 0 || (r.duration - target).abs() <= 4.0)
        .map(|r| (score(q, &r), r))
        .filter(|(sc, _)| *sc >= 0.7)
        .max_by(|(sa, a), (sb, b)| {
            let close = |r: &Remote| -(r.duration - target).abs();
            (a.synced.is_some(), (sa * 20.0).round() as i64)
                .cmp(&(b.synced.is_some(), (sb * 20.0).round() as i64))
                .then(close(a).total_cmp(&close(b)))
        })
        .map(|(_, r)| r)
}

/// Une requête, avec deux nouvelles chances si le serveur est surchargé (429, 5xx,
/// réponse coupée). `None` : rien trouvé (404).
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: reqwest::Url) -> Result<Option<T>, String> {
    let mut last = String::new();
    for attempt in 0..3u64 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(800 * attempt)).await;
        }
        let res = match client.get(url.clone()).send().await {
            Ok(res) => res,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        let status = res.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if status.as_u16() == 429 || status.is_server_error() {
            if let Some(wait) = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()) {
                tokio::time::sleep(Duration::from_secs(wait.min(5))).await;
            }
            last = format!("{} {status}", url.host_str().unwrap_or(""));
            continue;
        }
        if !status.is_success() {
            return Err(format!("{} {status}", url.host_str().unwrap_or("")));
        }
        // Réponse lue avec un plafond (2 Mo) : un serveur qui enverrait sans fin ne
        // remplit pas la mémoire.
        match read_capped(res, 2 * 1024 * 1024).await {
            Ok(Some(bytes)) => match serde_json::from_slice::<T>(&bytes) {
                Ok(v) => return Ok(Some(v)),
                Err(e) => last = e.to_string(),
            },
            Ok(None) => return Err("réponse trop grosse".into()),
            Err(e) => last = e,
        }
    }
    // Limite atteinte, panne : une erreur, jamais gardée en cache comme « introuvable ».
    Err(last)
}

/// Corps d'une réponse, lu morceau par morceau. `None` : il dépasse `cap` octets.
pub(crate) async fn read_capped(mut res: reqwest::Response, cap: usize) -> Result<Option<Vec<u8>>, String> {
    if res.content_length().is_some_and(|n| n > cap as u64) {
        return Ok(None);
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        if out.len() + chunk.len() > cap {
            return Ok(None);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Some(out))
}

pub(crate) fn client() -> Result<reqwest::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .user_agent(concat!("Echo/", env!("CARGO_PKG_VERSION"), " (https://github.com/wave3k/echo-player)"))
        .timeout(Duration::from_secs(12))
        // Redirections : 5 au plus, et jamais vers une adresse non chiffrée.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// LRCLIB : correspondance exacte, puis recherche par champs pour chaque variante,
/// puis recherche libre. On s'arrête dès qu'une version synchronisée convient.
async fn fetch(q: &Query) -> Result<Lyrics, String> {
    let client = client()?;
    let url = |path: &str, params: &[(&str, &str)]| {
        reqwest::Url::parse_with_params(&format!("{LRCLIB}/{path}"), params).map_err(|e| e.to_string())
    };
    let artists = artist_variants(&q.artist);
    let titles = title_variants(&q.title);
    let secs = (q.duration_ms / 1000).to_string();
    let mut pool: Vec<Remote> = Vec::new();

    if !q.video && q.duration_ms > 0 {
        let exact = url(
            "get",
            &[
                ("artist_name", artists[0].as_str()),
                ("track_name", titles[0].as_str()),
                ("album_name", q.album.as_str()),
                ("duration", secs.as_str()),
            ],
        )?;
        if let Ok(Some(r)) = get_json::<Remote>(&client, exact).await {
            if r.synced.is_some() {
                return Ok(r.into_lyrics());
            }
            pool.push(r);
        }
    }

    let mut queries: Vec<Vec<(&str, String)>> = Vec::new();
    for t in &titles {
        for a in &artists {
            queries.push(vec![("track_name", t.clone()), ("artist_name", a.clone())]);
        }
    }
    let primary = artists.last().cloned().unwrap_or_default();
    for t in &titles {
        queries.push(vec![("q", format!("{primary} {t}"))]);
    }
    // Titre court d'un seul mot : peut-être un sigle écrit « G.A.N.G » sur LRCLIB.
    for t in &titles {
        let word = t.trim();
        if (3..=6).contains(&word.chars().count()) && word.chars().all(|c| c.is_alphabetic()) {
            let spaced: Vec<String> = word.chars().map(|c| c.to_string()).collect();
            queries.push(vec![("q", format!("{primary} {}", spaced.join(" ")))]);
        }
    }
    // Artiste écrit autrement sur LRCLIB : par titre seul, l'artiste est vérifié ensuite.
    for t in &titles {
        queries.push(vec![("track_name", t.clone())]);
    }
    let mut seen = std::collections::HashSet::new();
    let mut failure: Option<String> = None;
    for params in queries {
        let key = format!("{params:?}");
        if !seen.insert(key) {
            continue;
        }
        let list: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        match get_json::<Vec<Remote>>(&client, url("search", &list)?).await {
            Ok(Some(found)) => pool.extend(found),
            Ok(None) => {}
            // Une variante qui échoue n'arrête pas les autres.
            Err(e) => failure = Some(e),
        }
        if let Some(best) = pick(q, std::mem::take(&mut pool)) {
            if best.synced.is_some() {
                return Ok(best.into_lyrics());
            }
            pool.push(best);
        }
    }
    if let Some(best) = pick(q, pool) {
        return Ok(best.into_lyrics());
    }
    match plain_fallback(&client, q).await {
        // Rien trouvé alors qu'une recherche a échoué : on ne conclut pas, on réessaiera.
        Ok(l) if !l.found() && failure.is_some() => Err(failure.unwrap_or_default()),
        other => other,
    }
}

/// Secours : lyrics.ovh, gratuit et sans clé, paroles non synchronisées seulement.
async fn plain_fallback(client: &reqwest::Client, q: &Query) -> Result<Lyrics, String> {
    #[derive(Deserialize)]
    struct Ovh {
        lyrics: Option<String>,
    }
    let artist = artist_variants(&q.artist).pop().unwrap_or_default();
    let title = title_variants(&q.title).pop().unwrap_or_default();
    if artist.is_empty() || title.is_empty() {
        return Ok(Lyrics::none());
    }
    let mut url = reqwest::Url::parse("https://api.lyrics.ovh/v1/").map_err(|e| e.to_string())?;
    url.path_segments_mut().map_err(|_| "adresse")?.pop_if_empty().push(&artist).push(&title);
    // Service lent ou absent : ce n'est pas une raison de garder « introuvable » en cache.
    let found = match get_json::<Ovh>(client, url).await {
        Ok(found) => found,
        Err(e) => return Err(e),
    };
    let text = found.and_then(|o| o.lyrics).map(|t| t.replace("\r\n", "\n")).filter(|t| t.trim().len() > 20);
    Ok(match text {
        Some(plain) => Lyrics {
            plain: Some(plain.trim().to_string()),
            source: "lyricsovh".into(),
            matched: format!("{artist} — {title}"),
            ..Default::default()
        },
        None => Lyrics::none(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_format_lrc_est_lu() {
        let lines = parse_lrc("[ar:Nova Lane]\n[00:01.50]Bonjour\n[00:03.00][00:10.00]Refrain\n[00:05]Ligne\n");
        assert_eq!(
            lines,
            vec![
                Line { ms: 1500, text: "Bonjour".into() },
                Line { ms: 3000, text: "Refrain".into() },
                Line { ms: 5000, text: "Ligne".into() },
                Line { ms: 10000, text: "Refrain".into() },
            ]
        );
    }

    #[test]
    fn le_decalage_lrc_est_applique() {
        let lines = parse_lrc("[offset:500]\n[00:02.00]a\n[00:04.00]b");
        assert_eq!(lines[0].ms, 1500);
        assert_eq!(lines[1].ms, 3500);
    }

    #[test]
    fn un_texte_sans_horodatage_reste_brut() {
        let l = Lyrics::from_text("Juste du texte\nsur deux lignes", "file").unwrap();
        assert!(l.synced.is_none());
        assert_eq!(l.plain.as_deref(), Some("Juste du texte\nsur deux lignes"));
        assert!(Lyrics::from_text("   ", "file").is_none());
    }

    fn remote(track: &str, artist: &str, duration: f64, synced: bool) -> Remote {
        Remote {
            track: track.into(),
            artist: artist.into(),
            plain: Some("x".into()),
            synced: synced.then(|| "[00:01.00]a\n[00:02.00]b".into()),
            instrumental: false,
            duration,
        }
    }

    fn query(artist: &str, title: &str, secs: u64, video: bool) -> Query {
        Query { artist: artist.into(), title: title.into(), album: String::new(), duration_ms: secs * 1000, video }
    }

    #[test]
    fn une_fiche_incomplete_ne_casse_pas_la_liste() {
        let json = r#"[{"trackName":"Not Like Us","artistName":"Kendrick Lamar","duration":null,"instrumental":null,"plainLyrics":"a","syncedLyrics":null},
                       {"trackName":"Not Like Us","artistName":"Kendrick Lamar","duration":274.0,"instrumental":false,"plainLyrics":"a","syncedLyrics":"[00:01.00]a"}]"#;
        let list: Vec<Remote> = serde_json::from_str(json).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].duration, 0.0);
    }

    #[test]
    fn les_noms_youtube_sont_decomposes() {
        assert_eq!(artist_variants("Tiakola x Dave"), ["Tiakola x Dave", "Tiakola"]);
        assert_eq!(artist_variants("Kendrick Lamar, SZA"), ["Kendrick Lamar, SZA", "Kendrick Lamar"]);
        assert_eq!(artist_variants("Drake"), ["Drake"]);
        assert_eq!(title_variants("Hit 'Em Up (Dirty)"), ["Hit 'Em Up (Dirty)", "Hit 'Em Up"]);
        assert_eq!(title_variants("GANG (feat. La Mano 19 & Niska)"), ["GANG"]);
        assert_eq!(fold("Yamê - Bécane"), "yame becane");
    }

    #[test]
    fn les_noms_pieges_ne_font_pas_planter() {
        assert_eq!(artist_variants("İBO x Dave"), ["İBO x Dave", "İBO"]);
        assert_eq!(search_title("Straße (FEAT. İ)"), "Straße");
    }

    #[test]
    fn la_ressemblance_trie_les_resultats() {
        let q = query("Yamê", "Bécane", 185, true);
        assert!(score(&q, &remote("Yamê - Bécane (Officiel)", "Yamê", 212.0, true)) >= 0.7);
        assert!(score(&q, &remote("Bécane", "Yame", 185.0, true)) >= 0.9);
        // Un autre morceau du même artiste ne passe pas.
        assert!(score(&q, &remote("Lumière", "Yamê", 185.0, true)) < 0.7);
        let q = query("CENTRAL CEE", "BAND4BAND (feat. LIL BABY)", 151, true);
        assert!(score(&q, &remote("CENTRAL CEE FT. LIL BABY - BAND4BAND", "Central Cee", 141.0, true)) >= 0.7);
    }

    #[test]
    fn les_variantes_d_ecriture_sont_reconnues() {
        let q = query("Tiakola", "GANG (feat. La Mano 19 & Niska)", 198, true);
        assert!(score(&q, &remote("G.A.N.G", "Tiakola", 198.0, true)) >= 0.7);
        let q = query("KOBA LA D x Binkszola91", "TEMPS EN TEMPS", 175, true);
        assert!(score(&q, &remote("TEMPS EN TEMPS", "Koba LaD", 175.0, true)) >= 0.7);
        assert!(score(&q, &remote("Temps en Temps", "Missiat", 194.0, true)) < 0.7, "même titre, autre artiste");
    }

    #[test]
    fn un_fichier_audio_exige_la_bonne_duree() {
        let q = query("SDM", "CARTIER SANTOS", 211, false);
        let picked = pick(&q, vec![remote("Cartier Santos", "SDM", 184.0, true), remote("Cartier Santos", "SDM", 211.0, true)]).unwrap();
        assert_eq!(picked.duration, 211.0);
        assert!(pick(&q, vec![remote("Cartier Santos", "SDM", 184.0, true)]).is_none());
        // Un clip accepte la version de l'album.
        let clip = query("SDM", "CARTIER SANTOS", 240, true);
        assert!(pick(&clip, vec![remote("Cartier Santos", "SDM", 211.0, true)]).is_some());
    }

    #[test]
    fn le_titre_de_recherche_perd_les_invites() {
        assert_eq!(search_title("3 singes (feat. Ninho)"), "3 singes");
        assert_eq!(search_title("God's Plan"), "God's Plan");
    }

    #[test]
    fn un_fichier_lrc_voisin_est_trouve() {
        let dir = std::env::temp_dir().join(format!("echo-lrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("morceau.mp3");
        std::fs::write(&song, b"pas vraiment un mp3").unwrap();
        std::fs::write(dir.join("morceau.lrc"), "[00:00.50]un\n[00:02.00]deux\n").unwrap();
        let l = local(&song).unwrap();
        assert_eq!(l.source, "lrc");
        assert_eq!(l.synced.unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn le_cache_garde_les_reponses() {
        let dir = std::env::temp_dir().join(format!("echo-lyr-cache-{}", std::process::id()));
        let q = Query { artist: "A".into(), title: "B".into(), album: String::new(), duration_ms: 200_000, video: false };
        let l = Lyrics { plain: Some("x".into()), source: "lrclib".into(), ..Default::default() };
        store(&dir, &q, &l);
        assert_eq!(cached(&dir, &q).unwrap().plain.as_deref(), Some("x"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Essai réel : cargo test lyrics::essai_lrclib -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore]
fn essai_lrclib() {
    let dir = std::env::temp_dir().join("echo-lrclib-essai");
    for (artist, title, ms) in [("Drake", "God's Plan", 198_000u64), ("Werenoi", "3 singes (feat. Ninho)", 204_000), ("Nova Lane", "Midnight Drive", 214_000)] {
        let q = Query { artist: artist.into(), title: title.into(), album: String::new(), duration_ms: ms, video: false };
        let l = tauri::async_runtime::block_on(online(&dir, &q));
        println!("{artist} - {title}: source={} synced={} plain={}", l.source, l.synced.as_ref().map(|s| s.len()).unwrap_or(0), l.plain.is_some());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mesure sur une vraie bibliothèque : ECHO_LIB=~/.config/com.echo.player/library.json
/// cargo test lyrics::mesure_bibliotheque -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore]
fn mesure_bibliotheque() {
    let path = std::env::var("ECHO_LIB").expect("ECHO_LIB");
    let lib: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("echo-lyr-mesure-{}", std::process::id()));
    let (mut synced, mut plain, mut total) = (0, 0, 0);
    for t in lib["tracks"].as_array().unwrap() {
        let q = Query {
            artist: t["artist"].as_str().unwrap_or("").into(),
            title: t["title"].as_str().unwrap_or("").into(),
            album: t["album"].as_str().unwrap_or("").into(),
            duration_ms: t["duration_ms"].as_u64().unwrap_or(0),
            video: t["video"].as_bool().unwrap_or(false),
        };
        let l = match tauri::async_runtime::block_on(fetch(&q)) {
            Ok(l) => l,
            Err(e) => {
                println!("ERREUR {} | {} : {e}", q.artist, q.title);
                Lyrics::none()
            }
        };
        std::thread::sleep(Duration::from_millis(std::env::var("ECHO_PAUSE").ok().and_then(|v| v.parse().ok()).unwrap_or(0)));
        total += 1;
        let kind = if l.synced.is_some() { synced += 1; "SYNC " } else if l.plain.is_some() { plain += 1; "texte" } else { "  -  " };
        println!("{kind} {} | {}  =>  {} ({:.0} s)", q.artist, q.title, l.matched, l.duration_s);
    }
    println!("BILAN synchronisées {synced}/{total}, texte seul {plain}/{total}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Détail d'une recherche : ECHO_A=artiste ECHO_T=titre ECHO_D=secondes cargo test lyrics::detail -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore]
fn detail() {
    let q = Query {
        artist: std::env::var("ECHO_A").unwrap(),
        title: std::env::var("ECHO_T").unwrap(),
        album: String::new(),
        duration_ms: std::env::var("ECHO_D").unwrap().parse::<u64>().unwrap() * 1000,
        video: true,
    };
    let client = client().unwrap();
    for t in title_variants(&q.title) {
        for a in artist_variants(&q.artist) {
            let url = reqwest::Url::parse_with_params(&format!("{LRCLIB}/search"), &[("track_name", t.as_str()), ("artist_name", a.as_str())]).unwrap();
            let r: Result<Option<Vec<Remote>>, String> = tauri::async_runtime::block_on(get_json(&client, url));
            match r {
                Ok(Some(list)) => {
                    println!("[{a} / {t}] {} résultats", list.len());
                    for x in list.iter().take(5) {
                        println!("   {:.2}  {} / {} / {} sync={}", score(&q, x), x.artist, x.track, x.duration, x.synced.is_some());
                    }
                }
                other => println!("[{a} / {t}] {:?}", other.map(|o| o.map(|l| l.len()))),
            }
        }
    }
}
