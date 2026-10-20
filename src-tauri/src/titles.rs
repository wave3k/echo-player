//! Devine l'artiste et un titre propre à partir d'un nom de fichier ou d'un titre
//! de vidéo : « 21 Savage - a lot (Official Video) ft. J. Cole » donne
//! l'artiste « 21 Savage » et le titre « a lot (feat. J. Cole) ».

/// Version des règles de lecture des fichiers (titres, pistes) : quand elle change,
/// les morceaux déjà connus sont relus une fois.
pub const VERSION: u32 = 3;

/// Mots qui signalent un ajout de la plateforme vidéo, pas une partie du titre.
const NOISE: &[&str] = &[
    "official", "officiel", "officielle", "video", "vidéo", "clip", "audio", "lyric", "lyrics", "paroles",
    "visualizer", "visualiser", "visual", "hd", "hq", "4k", "explicit", "closed captioned", "exclu",
    "mv",
];

/// Résultat : l'artiste deviné (vide si rien de sûr) et le titre nettoyé.
pub fn guess(raw: &str) -> (String, String) {
    let text = normalize(raw);
    let text = strip_noise(strip_track_number(&text));
    let (artist, title) = split_artist(&text);
    let (title, mut feats) = take_feat(&title);
    let (artist, artist_feat) = take_feat(&artist);
    feats.extend(artist_feat);

    let mut title = tidy(&title);
    let artist = tidy(&artist.replace('@', ""));
    if !feats.is_empty() {
        title = format!("{title} (feat. {})", feats.join(", "));
    }
    if title.is_empty() {
        return (artist, tidy(&text));
    }
    (artist, title)
}

/// Caractères « pleine largeur » et autres variantes que les sites de vidéo
/// mettent à la place de ceux interdits dans les noms de fichiers.
fn normalize(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| match c {
            '｜' => '|',
            '⧸' | '／' => '/',
            '：' => ':',
            '＂' => '"',
            '＊' => '*',
            '？' => '?',
            '–' | '—' => '-',
            _ => c,
        })
        .collect();
    // Suffixe de format de yt-dlp : « .f616 », « .f140 ».
    if let Some(pos) = s.rfind(".f") {
        if s[pos + 2..].chars().all(|c| c.is_ascii_digit()) && s.len() > pos + 2 {
            s.truncate(pos);
        }
    }
    s
}

/// « 13 - Funkorama », « 03. Titre » : le numéro de piste en tête d'un nom de fichier
/// (1 à 3 chiffres, puis « . », « - », « _ » ou « ) » suivi d'une espace) est retiré.
/// « 50 Cent », « 1-800-273-8255 » ou une année (« 2019 - … ») restent tels quels.
pub fn strip_track_number(text: &str) -> &str {
    let trimmed = text.trim_start();
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return text;
    }
    let rest = trimmed[digits..].trim_start();
    let Some(after) = rest.strip_prefix(['.', '-', '_', ')']) else { return text };
    if !after.starts_with(char::is_whitespace) || after.trim().is_empty() {
        return text;
    }
    after.trim_start()
}

/// « (Prod. X) », « [prod by X] » : le nom du producteur, ajouté au titre du clip.
fn is_credit(inner: &str) -> bool {
    let lower = inner.trim_start().to_lowercase();
    ["prod.", "prod ", "prod:", "produced by"].iter().any(|p| lower.starts_with(p))
}

fn is_noise(inner: &str) -> bool {
    let lower = inner.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return false;
    }
    // Groupe entièrement fait de mots « bruit » : (Official Music Video), [HD]…
    // Un groupe qui en contient seulement quelques-uns est gardé : (Live), (Remix)…
    let noisy = words
        .iter()
        .filter(|w| {
            NOISE.contains(w)
                || matches!(**w, "music" | "the" | "version" | "dirty" | "clean" | "uncut" | "closed" | "captioned" | "full")
        })
        .count();
    let has_strong = words.iter().any(|w| {
        matches!(
            *w,
            "official" | "officiel" | "officielle" | "video" | "vidéo" | "clip" | "audio" | "lyrics" | "lyric"
                | "visualizer" | "hd" | "hq" | "4k" | "exclu" | "mv" | "explicit"
        )
    }) || lower.contains("closed captioned");
    has_strong && noisy * 2 >= words.len()
}

/// Retire les groupes (…) et […] de bruit, les hashtags et la partie « | chaîne ».
fn strip_noise(text: &str) -> String {
    // « Titre | GRM Daily » : ce qui suit la barre est le nom de la chaîne.
    let text = match text.find(" | ") {
        Some(pos) if pos > 0 => &text[..pos],
        _ => text,
    };
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '(' || c == '[' {
            let close = if c == '(' { ')' } else { ']' };
            if let Some(len) = chars[i + 1..].iter().position(|x| *x == close) {
                let inner: String = chars[i + 1..i + 1 + len].iter().collect();
                // « (Explicit Version/Closed Captioned) » : chaque partie est du bruit.
                let all_noise = inner.split('/').all(|part| is_noise(part) || part.trim().is_empty());
                if is_noise(&inner) || all_noise || is_credit(&inner) {
                    i += len + 2;
                    continue;
                }
            }
        }
        if c == '#' {
            // Hashtag de chaîne (#PlanèteRap) : retiré. Un numéro (#5) reste.
            let word_len = chars[i + 1..].iter().take_while(|x| x.is_alphanumeric()).count();
            let word: String = chars[i + 1..i + 1 + word_len].iter().collect();
            if word_len > 0 && !word.chars().all(|x| x.is_ascii_digit()) {
                i += word_len + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    // « … (Music Video) HD » : mention de qualité laissée seule en fin de titre.
    let mut out = tidy(&out);
    for tail in [" HD", " HQ", " 4K"] {
        let cut = out.len().saturating_sub(tail.len());
        if out.len() > tail.len() && out.is_char_boundary(cut) && out[cut..].eq_ignore_ascii_case(tail) {
            out.truncate(cut);
        }
    }
    tidy(&out)
}

/// « Artiste - Titre » : coupe au premier tiret entouré d'espaces.
/// À défaut, « Artiste "Titre" (…) » : le titre est entre guillemets.
fn split_artist(text: &str) -> (String, String) {
    if let Some(pos) = text.find(" - ").filter(|p| *p > 0) {
        return (text[..pos].to_string(), text[pos + 3..].to_string());
    }
    let quotes = ['"', '“', '”', '«', '»'];
    if let Some(open) = text.find(quotes).filter(|p| *p > 0) {
        // « et “ font 2 à 3 octets : on avance de la taille réelle du guillemet.
        let open_len = text[open..].chars().next().map(char::len_utf8).unwrap_or(1);
        let after = &text[open + open_len..];
        if let Some(len) = after.find(quotes) {
            let close_len = after[len..].chars().next().map(char::len_utf8).unwrap_or(1);
            let artist = text[..open].trim().to_string();
            let title = format!("{} {}", &after[..len], &after[len + close_len..]);
            if !artist.is_empty() && len > 0 {
                return (artist, title);
            }
        }
    }
    (String::new(), text.to_string())
}

/// Cherche `needle` (en minuscules ASCII) sans tenir compte de la casse, et renvoie
/// début et fin **dans le texte d'origine**. Chercher dans `text.to_lowercase()` puis
/// couper `text` à la même position plante : « İ » ou « ẞ » changent de longueur.
pub fn find_ci(text: &str, needle: &str) -> Option<(usize, usize)> {
    let lower = |c: char| {
        let mut l = c.to_lowercase();
        let first = l.next();
        if l.next().is_some() {
            None
        } else {
            first
        }
    };
    for (start, _) in text.char_indices() {
        let mut rest = text[start..].char_indices();
        let mut end = start;
        let mut ok = true;
        for n in needle.chars() {
            match rest.next() {
                Some((at, c)) if lower(c) == Some(n) => end = start + at + c.len_utf8(),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && !needle.is_empty() {
            return Some((start, end));
        }
    }
    None
}

/// Sépare les artistes invités : « ft. », « feat. », « featuring », y compris entre parenthèses.
fn take_feat(text: &str) -> (String, Vec<String>) {
    let markers = ["(feat. ", "(feat ", "(ft. ", "(ft ", "[feat. ", " feat. ", " feat ", " ft. ", " ft ", " featuring "];
    let mut best: Option<(usize, usize, &str)> = None;
    for m in markers {
        if let Some((start, end)) = find_ci(text, m) {
            if best.map(|(p, _, _)| start < p).unwrap_or(true) {
                best = Some((start, end, m));
            }
        }
    }
    let Some((pos, end, marker)) = best else {
        return (text.to_string(), Vec::new());
    };
    let before = text[..pos].to_string();
    let after = &text[end..];
    let (guests, rest) = if marker.starts_with('(') || marker.starts_with('[') {
        let close = if marker.starts_with('(') { ')' } else { ']' };
        match after.find(close) {
            Some(end) => (after[..end].to_string(), after[end + 1..].to_string()),
            None => (after.to_string(), String::new()),
        }
    } else {
        // Sans parenthèses, les invités vont jusqu'au prochain groupe ou à la fin.
        match after.find(['(', '[']) {
            Some(end) => (after[..end].to_string(), after[end..].to_string()),
            None => (after.to_string(), String::new()),
        }
    };
    let guests = tidy(&guests);
    let rest_clean = format!("{} {}", before.trim_end(), rest.trim_start());
    if guests.is_empty() {
        return (tidy(&rest_clean), Vec::new());
    }
    (tidy(&rest_clean), vec![guests])
}

/// Espaces en trop, séparateurs orphelins en bout de chaîne.
fn tidy(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .trim_matches(|c: char| c == '-' || c == '|' || c == ',' || c == ':' || c.is_whitespace())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::guess;

    /// Les vrais noms de fichiers de la bibliothèque de l'utilisateur.
    #[test]
    fn noms_de_videos_reels() {
        let cases = [
            ("21 Savage - a lot (Official Video) ft. J. Cole", "21 Savage", "a lot (feat. J. Cole)"),
            ("2Pac - Hit 'Em Up (Dirty) (Music Video) HD", "2Pac", "Hit 'Em Up (Dirty)"),
            ("50 Cent - Many Men (Wish Death) (Dirty Version).f616", "50 Cent", "Many Men (Wish Death) (Dirty Version)"),
            ("Central Cee - Doja (Official Music Video)", "Central Cee", "Doja"),
            ("CENTRAL CEE FT. LIL BABY - BAND4BAND (MUSIC VIDEO)", "CENTRAL CEE", "BAND4BAND (feat. LIL BABY)"),
            ("Central Cee - Loading [Music Video] ｜ GRM Daily", "Central Cee", "Loading"),
            ("Central Cee x Dave - Sprinter [Music Video]", "Central Cee x Dave", "Sprinter"),
            ("Chief Keef - Love Sosa", "Chief Keef", "Love Sosa"),
            ("[EXCLU] Genezio - Big boss #PlanèteRap", "Genezio", "Big boss"),
            ("Kendrick Lamar, SZA - All The Stars", "Kendrick Lamar, SZA", "All The Stars"),
            ("KOBA LA D x @Binkszola91  - TEMPS EN TEMPS", "KOBA LA D x Binkszola91", "TEMPS EN TEMPS"),
            ("La Rvfleuze - Argent Sale ｜ A COLORS SHOW", "La Rvfleuze", "Argent Sale"),
            ("La Rvfleuze - Serrure #5", "La Rvfleuze", "Serrure #5"),
            ("Leto (feat. iDS) - MATHÉMATIQUE (Clip officiel)", "Leto", "MATHÉMATIQUE (feat. iDS)"),
            ("Lil Nas X - Old Town Road (Official Video) ft. Billy Ray Cyrus", "Lil Nas X", "Old Town Road (feat. Billy Ray Cyrus)"),
            ("Lil Wayne - Love Me (Explicit Version⧸Closed Captioned) ft. Drake, Future", "Lil Wayne", "Love Me (feat. Drake, Future)"),
            ("Migos - Walk It Talk It ft. Drake (Official Video)", "Migos", "Walk It Talk It (feat. Drake)"),
            ("OrelSan - Encore une fois (feat. Yamê) [CLIP OFFICIEL]", "OrelSan", "Encore une fois (feat. Yamê)"),
            ("Post Malone, Swae Lee - Sunflower (Spider-Man： Into the Spider-Verse)", "Post Malone, Swae Lee", "Sunflower (Spider-Man: Into the Spider-Verse)"),
            ("Roddy Ricch - The Box [Official Audio]", "Roddy Ricch", "The Box"),
            ("Tiakola - Coucher de soleil (Live) ｜ Vevo Rounds", "Tiakola", "Coucher de soleil (Live)"),
            ("Tiakola feat. La Mano 19 & Niska - GANG #PlanèteRap", "Tiakola", "GANG (feat. La Mano 19 & Niska)"),
            ("Tiakola x SDM x Liim's - PLAISIR NOCIF (Visual Mixtape)", "Tiakola x SDM x Liim's", "PLAISIR NOCIF (Visual Mixtape)"),
            ("YNW Melly \"Murder On My Mind\" (Live Performance)", "YNW Melly", "Murder On My Mind (Live Performance)"),
            ("Werenoi - 3 singes (feat. Ninho)", "Werenoi", "3 singes (feat. Ninho)"),
        ];
        for (raw, artist, title) in cases {
            assert_eq!(guess(raw), (artist.to_string(), title.to_string()), "pour « {raw} »");
        }
    }

    /// Bug de l'audit : « 13 - Funkorama » donnait l'artiste « 13 ».
    #[test]
    fn le_numero_de_piste_est_retire() {
        let none = String::new();
        assert_eq!(guess("13 - Funkorama"), (none.clone(), "Funkorama".to_string()));
        assert_eq!(guess("03. Funkorama"), (none.clone(), "Funkorama".to_string()));
        assert_eq!(guess("7 - Daft Punk - One More Time"), ("Daft Punk".to_string(), "One More Time".to_string()));
        assert_eq!(guess("02 – Nekfeu - Égérie"), ("Nekfeu".to_string(), "Égérie".to_string()));
        // Pas des numéros de piste.
        assert_eq!(guess("50 Cent - Many Men").0, "50 Cent");
        assert_eq!(guess("1-800-273-8255"), (none.clone(), "1-800-273-8255".to_string()));
        assert_eq!(guess("3 singes"), (none, "3 singes".to_string()));
        assert_eq!(super::strip_track_number("13 - "), "13 - ");
        assert_eq!(super::strip_track_number("2019 - Titre"), "2019 - Titre");
    }

    #[test]
    fn le_producteur_est_retire_du_titre() {
        assert_eq!(guess("Ninho - Jefe (Prod. Dawson)"), ("Ninho".to_string(), "Jefe".to_string()));
        assert_eq!(guess("Gims - Bella [prod by Renaud Rebillaud] (Clip Officiel)"), ("Gims".to_string(), "Bella".to_string()));
        assert_eq!(guess("Freeze Corleone - Projet Blue Beam [PROD. KOSEI]").1, "Projet Blue Beam");
        // Un mot qui commence seulement pareil reste.
        assert_eq!(guess("Artiste - Titre (Prodigy Remix)").1, "Titre (Prodigy Remix)");
    }

    #[test]
    fn sans_tiret_pas_d_artiste_invente() {
        assert_eq!(guess("Ma chanson (Live)"), (String::new(), "Ma chanson (Live)".to_string()));
        assert_eq!(guess("Piste 03"), (String::new(), "Piste 03".to_string()));
    }

    #[test]
    fn les_noms_pieges_ne_font_pas_planter() {
        // Caractères dont la minuscule ou la majuscule change de longueur, guillemets sur 2-3 octets.
        let (a, t) = guess("İSTANBUL ft. Dave - Gece");
        assert!(!a.is_empty() && !t.is_empty());
        let (a, t) = guess("Nekfeu «Énergie sombre» (Clip officiel)");
        assert_eq!(a, "Nekfeu");
        assert!(t.contains("Énergie sombre"));
        let (_, t) = guess("Artiste “Titre” HD");
        assert!(t.contains("Titre"));
        let (_, t) = guess("Straße HD");
        assert_eq!(t, "Straße");
        let (_, t) = guess("ẞ FEAT. İ");
        assert!(!t.is_empty());
        assert_eq!(super::find_ci("A FEAT. B", " feat. "), Some((1, 8)));
        assert_eq!(super::find_ci("İ ft. X", " ft. "), Some((2, 7)));
    }
}
