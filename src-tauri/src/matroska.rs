//! Lecture minimale des fichiers Matroska (webm, mkv) : durée, titre, tags,
//! présence d'une piste audio et pochette jointe. `lofty` ne lit pas ce format.
//!
//! Un fichier Matroska est une suite d'éléments EBML (identifiant, taille, données).
//! On ne lit que les en-têtes utiles, sans jamais charger les images ni le son.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMECODE_SCALE: u32 = 0x2A_D7B1;
const DURATION: u32 = 0x4489;
const TITLE: u32 = 0x7BA9;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_TYPE: u32 = 0x83;
const TAGS: u32 = 0x1254_C367;
const TAG: u32 = 0x7373;
const SIMPLE_TAG: u32 = 0x67C8;
const TAG_NAME: u32 = 0x45A3;
const TAG_STRING: u32 = 0x4487;
const ATTACHMENTS: u32 = 0x1941_A469;
const ATTACHED_FILE: u32 = 0x61A7;
const FILE_MIME: u32 = 0x4660;
const FILE_DATA: u32 = 0x465C;
const CLUSTER: u32 = 0x1F43_B675;

/// Taille au-delà de laquelle une pochette jointe est ignorée.
const MAX_COVER: u64 = 16 * 1024 * 1024;

#[derive(Default, Debug)]
pub struct Info {
    pub duration_ms: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `None` si le fichier ne déclare aucune piste (on ne sait pas).
    pub has_audio: Option<bool>,
    /// Vrai si le fichier a une piste image (une vidéo, pas seulement du son).
    pub has_video: bool,
    /// Pochette jointe : (type MIME, octets).
    pub cover: Option<(String, Vec<u8>)>,
}

struct Reader<R> {
    inner: R,
    len: u64,
}

impl<R: Read + Seek> Reader<R> {
    fn pos(&mut self) -> Option<u64> {
        self.inner.stream_position().ok()
    }

    fn byte(&mut self) -> Option<u8> {
        let mut b = [0u8; 1];
        self.inner.read_exact(&mut b).ok()?;
        Some(b[0])
    }

    /// Identifiant d'élément : entier de longueur variable, marqueur compris.
    fn id(&mut self) -> Option<u32> {
        let first = self.byte()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 4 {
            return None;
        }
        let mut v = first as u32;
        for _ in 1..len {
            v = (v << 8) | self.byte()? as u32;
        }
        Some(v)
    }

    /// Taille d'élément : entier de longueur variable, marqueur retiré.
    /// `None` dans le résultat pour une taille inconnue (flux en direct).
    fn size(&mut self) -> Option<Option<u64>> {
        let first = self.byte()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 8 {
            return None;
        }
        let mask = if len == 8 { 0 } else { 0xFFu8 >> len };
        let mut v = (first & mask) as u64;
        let mut all_ones = (first & mask) == mask;
        for _ in 1..len {
            let b = self.byte()?;
            all_ones &= b == 0xFF;
            v = (v << 8) | b as u64;
        }
        Some(if all_ones { None } else { Some(v) })
    }

    fn bytes(&mut self, n: u64) -> Option<Vec<u8>> {
        if n > MAX_COVER {
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        self.inner.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    fn skip(&mut self, n: u64) -> Option<()> {
        self.inner.seek(SeekFrom::Current(n as i64)).ok()?;
        Some(())
    }

    fn uint(&mut self, n: u64) -> Option<u64> {
        if n > 8 {
            self.skip(n)?;
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 8) | self.byte()? as u64;
        }
        Some(v)
    }

    fn float(&mut self, n: u64) -> Option<f64> {
        match n {
            4 => Some(f32::from_bits(self.uint(4)? as u32) as f64),
            8 => Some(f64::from_bits(self.uint(8)?)),
            _ => {
                self.skip(n)?;
                None
            }
        }
    }

    fn text(&mut self, n: u64) -> Option<String> {
        if n > 64 * 1024 {
            self.skip(n)?;
            return None;
        }
        let raw = self.bytes(n)?;
        Some(String::from_utf8_lossy(&raw).trim_end_matches('\0').trim().to_string())
    }

    /// Parcourt les enfants d'un élément de `size` octets.
    fn children(&mut self, size: Option<u64>, mut each: impl FnMut(&mut Self, u32, Option<u64>) -> Option<()>) {
        let start = match self.pos() {
            Some(p) => p,
            None => return,
        };
        let end = size.map(|s| start + s).unwrap_or(self.len).min(self.len);
        while let Some(pos) = self.pos() {
            if pos >= end {
                break;
            }
            let Some(id) = self.id() else { break };
            let Some(child_size) = self.size() else { break };
            let child_start = match self.pos() {
                Some(p) => p,
                None => break,
            };
            if id == CLUSTER {
                // Les données audio/vidéo commencent : plus rien d'utile au fil du fichier.
                break;
            }
            if each(self, id, child_size).is_none() {
                break;
            }
            // L'élément a pu être lu en partie : on se replace juste après lui.
            match child_size {
                Some(s) => {
                    if self.inner.seek(SeekFrom::Start(child_start + s)).is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
    }
}

/// Lit les informations d'un fichier webm / mkv. `None` si ce n'en est pas un.
pub fn read(path: &Path) -> Option<Info> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut r = Reader { inner: BufReader::new(file), len };

    // En-tête EBML, puis le segment qui contient tout le reste.
    if r.id()? != 0x1A45_DFA3 {
        return None;
    }
    let header = r.size()??;
    r.skip(header)?;
    if r.id()? != SEGMENT {
        return None;
    }
    let segment_size = r.size()?;
    let segment_start = r.pos()?;

    let mut info = Info::default();
    let mut scale: u64 = 1_000_000;
    let mut duration: Option<f64> = None;
    let mut tag_duration: Option<u64> = None;
    let mut seen = Vec::new();
    let mut targets: Vec<(u32, u64)> = Vec::new();

    // Premier passage dans l'ordre du fichier, jusqu'aux données (Cluster).
    r.children(segment_size, |r, id, size| {
        read_top(r, id, size, &mut info, &mut scale, &mut duration, &mut tag_duration, &mut targets);
        seen.push(id);
        Some(())
    });

    // Les tags et pièces jointes sont souvent rangés après les données :
    // le sommaire (SeekHead) donne leur position.
    for (id, offset) in targets.clone() {
        if seen.contains(&id) || !matches!(id, INFO | TRACKS | TAGS | ATTACHMENTS) {
            continue;
        }
        if r.inner.seek(SeekFrom::Start(segment_start + offset)).is_err() {
            continue;
        }
        let (Some(found), Some(size)) = (r.id(), r.size()) else { continue };
        if found != id {
            continue;
        }
        let mut ignored = Vec::new();
        read_top(&mut r, id, size, &mut info, &mut scale, &mut duration, &mut tag_duration, &mut ignored);
        seen.push(id);
    }

    info.duration_ms = match duration {
        Some(d) if d > 0.0 => (d * scale as f64 / 1_000_000.0) as u64,
        _ => tag_duration.unwrap_or(0),
    };
    Some(info)
}

#[allow(clippy::too_many_arguments)]
fn read_top<R: Read + Seek>(
    r: &mut Reader<R>,
    id: u32,
    size: Option<u64>,
    info: &mut Info,
    scale: &mut u64,
    duration: &mut Option<f64>,
    tag_duration: &mut Option<u64>,
    targets: &mut Vec<(u32, u64)>,
) {
    match id {
        SEEK_HEAD => r.children(size, |r, id, size| {
            if id == SEEK {
                let mut seek_id = 0u32;
                let mut position = None;
                r.children(size, |r, id, size| {
                    let n = size?;
                    match id {
                        SEEK_ID => seek_id = r.uint(n)? as u32,
                        SEEK_POSITION => position = r.uint(n),
                        _ => {}
                    }
                    Some(())
                });
                if let Some(p) = position {
                    targets.push((seek_id, p));
                }
            }
            Some(())
        }),
        INFO => r.children(size, |r, id, size| {
            let n = size?;
            match id {
                TIMECODE_SCALE => *scale = r.uint(n).filter(|s| *s > 0).unwrap_or(1_000_000),
                DURATION => *duration = r.float(n),
                TITLE => info.title = r.text(n).unwrap_or_default(),
                _ => {}
            }
            Some(())
        }),
        TRACKS => r.children(size, |r, id, size| {
            if id == TRACK_ENTRY {
                r.children(size, |r, id, size| {
                    if id == TRACK_TYPE {
                        let kind = r.uint(size?)?;
                        if kind == 1 {
                            info.has_video = true;
                        }
                        if kind == 2 {
                            info.has_audio = Some(true);
                        } else if info.has_audio.is_none() {
                            info.has_audio = Some(false);
                        }
                    }
                    Some(())
                });
            }
            Some(())
        }),
        TAGS => r.children(size, |r, id, size| {
            if id == TAG {
                r.children(size, |r, id, size| {
                    if id == SIMPLE_TAG {
                        let mut name = String::new();
                        let mut value = String::new();
                        r.children(size, |r, id, size| {
                            let n = size?;
                            match id {
                                TAG_NAME => name = r.text(n)?.to_ascii_uppercase(),
                                TAG_STRING => value = r.text(n)?,
                                _ => {}
                            }
                            Some(())
                        });
                        apply_tag(info, tag_duration, &name, value);
                    }
                    Some(())
                });
            }
            Some(())
        }),
        ATTACHMENTS => r.children(size, |r, id, size| {
            if id == ATTACHED_FILE && info.cover.is_none() {
                let mut mime = String::new();
                let mut data = None;
                r.children(size, |r, id, size| {
                    let n = size?;
                    match id {
                        FILE_MIME => mime = r.text(n)?,
                        FILE_DATA if mime.starts_with("image/") || mime.is_empty() => data = r.bytes(n),
                        _ => {}
                    }
                    Some(())
                });
                if let Some(bytes) = data {
                    if mime.starts_with("image/") && !bytes.is_empty() {
                        info.cover = Some((mime, bytes));
                    }
                }
            }
            Some(())
        }),
        _ => {}
    }
}

fn apply_tag(info: &mut Info, tag_duration: &mut Option<u64>, name: &str, value: String) {
    if value.is_empty() {
        return;
    }
    match name {
        "TITLE" if info.title.is_empty() => info.title = value,
        "ARTIST" | "ALBUM_ARTIST" | "PERFORMER" if info.artist.is_empty() => info.artist = value,
        "ALBUM" if info.album.is_empty() => info.album = value,
        // ffmpeg écrit la durée de chaque piste en tag (« 00:05:56.881000000 »).
        "DURATION" if tag_duration.is_none() => *tag_duration = parse_clock(&value),
        _ => {}
    }
}

/// « HH:MM:SS.fraction » → millisecondes.
fn parse_clock(text: &str) -> Option<u64> {
    let mut parts = text.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(((h * 3600.0 + m * 60.0 + s) * 1000.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode un élément EBML (identifiant déjà avec marqueur, taille sur 8 octets).
    fn el(id: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let id_bytes = id.to_be_bytes();
        let skip = id_bytes.iter().position(|b| *b != 0).unwrap_or(3);
        out.extend(&id_bytes[skip..]);
        out.push(0x01);
        out.extend(&(data.len() as u64).to_be_bytes()[1..]);
        out.extend(data);
        out
    }

    fn build(with_seek: bool) -> Vec<u8> {
        let info = el(
            INFO,
            &[el(TIMECODE_SCALE, &1_000_000u32.to_be_bytes()), el(DURATION, &356_881.0f64.to_be_bytes())].concat(),
        );
        let tracks = el(
            TRACKS,
            &[el(TRACK_ENTRY, &el(TRACK_TYPE, &[1])), el(TRACK_ENTRY, &el(TRACK_TYPE, &[2]))].concat(),
        );
        let simple = |name: &str, value: &str| el(SIMPLE_TAG, &[el(TAG_NAME, name.as_bytes()), el(TAG_STRING, value.as_bytes())].concat());
        let tags = el(TAGS, &el(TAG, &[simple("ARTIST", "Drake"), simple("title", "God's Plan")].concat()));
        let attach = el(
            ATTACHMENTS,
            &el(ATTACHED_FILE, &[el(FILE_MIME, b"image/jpeg"), el(FILE_DATA, &[0xFF, 0xD8, 0xFF])].concat()),
        );
        let cluster = el(CLUSTER, &[0u8; 64]);

        let body = if with_seek {
            // Tags et pochette après les données, retrouvés grâce au sommaire.
            let head_len = 0x100usize;
            let after = [info.clone(), tracks.clone(), cluster.clone()].concat();
            let tags_at = (head_len + after.len()) as u64;
            let attach_at = tags_at + tags.len() as u64;
            let seek = |id: u32, at: u64| el(SEEK, &[el(SEEK_ID, &id.to_be_bytes()), el(SEEK_POSITION, &at.to_be_bytes())].concat());
            let mut head = el(SEEK_HEAD, &[seek(TAGS, tags_at), seek(ATTACHMENTS, attach_at)].concat());
            // Remplissage (élément Void) pour que le sommaire fasse exactement head_len octets.
            let pad = head_len - head.len() - 9;
            head.extend(el(0xEC, &vec![0u8; pad]));
            assert_eq!(head.len(), head_len);
            [head, after, tags, attach].concat()
        } else {
            [info, tracks, tags, attach, cluster].concat()
        };
        let mut file = el(0x1A45_DFA3, &el(0x4282, b"webm"));
        file.extend(el(SEGMENT, &body));
        file
    }

    fn read_bytes(bytes: Vec<u8>) -> Info {
        let path = std::env::temp_dir().join(format!("echo-mkv-{}-{}.webm", std::process::id(), bytes.len()));
        std::fs::write(&path, bytes).unwrap();
        let info = read(&path).unwrap();
        let _ = std::fs::remove_file(path);
        info
    }

    #[test]
    fn lit_duree_tags_audio_et_pochette() {
        let info = read_bytes(build(false));
        assert_eq!(info.duration_ms, 356_881);
        assert_eq!(info.artist, "Drake");
        assert_eq!(info.title, "God's Plan");
        assert_eq!(info.has_audio, Some(true));
        assert!(info.has_video);
        assert_eq!(info.cover.unwrap().0, "image/jpeg");
    }

    #[test]
    fn retrouve_les_tags_ranges_apres_les_donnees() {
        let info = read_bytes(build(true));
        assert_eq!(info.duration_ms, 356_881);
        assert_eq!(info.artist, "Drake");
        assert!(info.cover.is_some());
    }

    #[test]
    fn duree_en_tag_quand_l_en_tete_n_en_a_pas() {
        assert_eq!(parse_clock("00:05:56.881000000"), Some(356_881));
    }

    #[test]
    fn un_fichier_qui_n_est_pas_du_matroska_est_ignore() {
        let path = std::env::temp_dir().join(format!("echo-mkv-{}-faux.webm", std::process::id()));
        std::fs::write(&path, b"pas du tout").unwrap();
        assert!(read(&path).is_none());
        let _ = std::fs::remove_file(path);
    }
}
