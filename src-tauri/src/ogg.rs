//! Ogg dont la première piste n'est pas du son (clip, ou pochette rangée en piste
//! vidéo Theora) : `lofty` le refuse et le morceau perdait ses tags. On y lit quand
//! même les commentaires Vorbis ou Opus, et la durée de la piste son.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Default)]
pub struct Info {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub has_audio: bool,
    pub has_video: bool,
}

/// Au-delà, on renonce : les en-têtes sont au tout début du fichier.
const MAX_HEAD: u64 = 16 * 1024 * 1024;

/// Une piste son : son numéro de flux, sa fréquence et le décalage du début (Opus).
struct Audio {
    serial: u32,
    rate: u64,
    pre_skip: u64,
}

pub fn read(path: &Path) -> Option<Info> {
    let mut file = BufReader::new(File::open(path).ok()?);
    let mut info = Info::default();
    let mut audio: Option<Audio> = None;
    // Paquet en cours de reconstitution et nombre de paquets déjà vus, par flux.
    let mut partial: HashMap<u32, (Vec<u8>, u32)> = HashMap::new();
    let mut read_bytes = 0u64;
    let mut found_comments = false;

    'pages: while read_bytes < MAX_HEAD {
        let mut header = [0u8; 27];
        if file.read_exact(&mut header).is_err() || &header[..4] != b"OggS" {
            break;
        }
        let serial = u32::from_le_bytes(header[14..18].try_into().ok()?);
        let mut lacing = vec![0u8; header[26] as usize];
        file.read_exact(&mut lacing).ok()?;
        let mut body = vec![0u8; lacing.iter().map(|l| *l as usize).sum()];
        file.read_exact(&mut body).ok()?;
        read_bytes += 27 + lacing.len() as u64 + body.len() as u64;

        let mut at = 0;
        for lace in lacing {
            let entry = partial.entry(serial).or_default();
            entry.0.extend_from_slice(&body[at..at + lace as usize]);
            at += lace as usize;
            if lace == 255 {
                continue; // le paquet continue
            }
            let packet = std::mem::take(&mut entry.0);
            let number = entry.1;
            entry.1 += 1;
            if number == 0 {
                // Premier paquet d'un flux : il dit ce qu'il contient.
                if packet.starts_with(b"\x80theora") {
                    info.has_video = true;
                } else if audio.is_none() && packet.starts_with(b"\x01vorbis") && packet.len() >= 16 {
                    let rate = u32::from_le_bytes(packet[12..16].try_into().ok()?) as u64;
                    audio = Some(Audio { serial, rate, pre_skip: 0 });
                } else if audio.is_none() && packet.starts_with(b"OpusHead") && packet.len() >= 12 {
                    let pre_skip = u16::from_le_bytes(packet[10..12].try_into().ok()?) as u64;
                    audio = Some(Audio { serial, rate: 48_000, pre_skip });
                }
            } else if number == 1 && audio.as_ref().is_some_and(|a| a.serial == serial) {
                // Deuxième paquet de la piste son : ses commentaires.
                let comments = packet
                    .strip_prefix(b"\x03vorbis")
                    .or_else(|| packet.strip_prefix(b"OpusTags"));
                if let Some(c) = comments {
                    read_comments(c, &mut info);
                }
                found_comments = true;
                break 'pages;
            }
        }
    }
    let audio = audio?;
    info.has_audio = true;
    if !found_comments {
        return Some(info);
    }
    if let Some(granule) = last_granule(file.get_mut(), audio.serial) {
        let samples = granule.saturating_sub(audio.pre_skip).saturating_mul(1000);
        info.duration_ms = samples.checked_div(audio.rate).unwrap_or(0);
    }
    Some(info)
}

/// Commentaires Vorbis : « CLÉ=valeur », la clé sans tenir compte de la casse.
fn read_comments(data: &[u8], info: &mut Info) {
    let mut at = 0usize;
    let mut next = |len: usize| -> Option<&[u8]> {
        let out = data.get(at..at.checked_add(len)?)?;
        at += len;
        Some(out)
    };
    let Some(vendor_len) = next(4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else { return };
    if next(vendor_len as usize).is_none() {
        return;
    }
    let Some(count) = next(4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else { return };
    let mut album_artist = String::new();
    for _ in 0..count.min(10_000) {
        let Some(len) = next(4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else { break };
        let Some(entry) = next(len as usize) else { break };
        let entry = String::from_utf8_lossy(entry);
        let Some((key, value)) = entry.split_once('=') else { continue };
        let value = value.trim();
        let slot = match key.to_ascii_uppercase().as_str() {
            "TITLE" => &mut info.title,
            "ARTIST" => &mut info.artist,
            "ALBUM" => &mut info.album,
            "ALBUMARTIST" | "ALBUM ARTIST" => &mut album_artist,
            _ => continue,
        };
        // Plusieurs valeurs : la première compte.
        if slot.is_empty() {
            *slot = value.to_string();
        }
    }
    if info.artist.is_empty() {
        info.artist = album_artist;
    }
}

/// Position (en échantillons) de la dernière page de ce flux, lue à la fin du fichier.
fn last_granule(file: &mut File, serial: u32) -> Option<u64> {
    let len = file.seek(SeekFrom::End(0)).ok()?;
    let start = len.saturating_sub(256 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let mut best = None;
    let mut i = 0;
    while i + 27 <= tail.len() {
        if &tail[i..i + 4] == b"OggS" {
            let granule = i64::from_le_bytes(tail[i + 6..i + 14].try_into().ok()?);
            let page_serial = u32::from_le_bytes(tail[i + 14..i + 18].try_into().ok()?);
            if page_serial == serial && granule > 0 {
                best = Some(granule as u64);
            }
        }
        i += 1;
    }
    best
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Une page Ogg (sans somme de contrôle : elle n'est pas vérifiée ici).
    fn page(serial: u32, granule: i64, flags: u8, packets: &[&[u8]]) -> Vec<u8> {
        let mut lacing = Vec::new();
        let mut body = Vec::new();
        for p in packets {
            let mut n = p.len();
            while n >= 255 {
                lacing.push(255u8);
                n -= 255;
            }
            lacing.push(n as u8);
            body.extend_from_slice(p);
        }
        let mut out = b"OggS".to_vec();
        out.push(0);
        out.push(flags);
        out.extend(granule.to_le_bytes());
        out.extend(serial.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.push(lacing.len() as u8);
        out.extend(lacing);
        out.extend(body);
        out
    }

    fn comments(entries: &[&str]) -> Vec<u8> {
        let mut c = b"\x03vorbis".to_vec();
        c.extend(4u32.to_le_bytes());
        c.extend(b"test");
        c.extend((entries.len() as u32).to_le_bytes());
        for e in entries {
            c.extend((e.len() as u32).to_le_bytes());
            c.extend(e.as_bytes());
        }
        c
    }

    /// Un Ogg dont la première piste est une image Theora, puis le son Vorbis.
    pub fn theora_then_vorbis(path: &Path, entries: &[&str]) {
        let mut id = b"\x01vorbis".to_vec();
        id.extend(0u32.to_le_bytes()); // version
        id.push(2); // canaux
        id.extend(44_100u32.to_le_bytes());
        id.extend([0u8; 13]);
        let mut theora = b"\x80theora".to_vec();
        theora.extend([0u8; 34]);
        let mut file = Vec::new();
        file.extend(page(7, 0, 2, &[&theora]));
        file.extend(page(9, 0, 2, &[&id]));
        // Un long commentaire, coupé en plusieurs segments.
        let long = format!("DESCRIPTION={}", "x".repeat(600));
        let mut all: Vec<&str> = entries.to_vec();
        all.push(&long);
        file.extend(page(9, 0, 0, &[&comments(&all)]));
        file.extend(page(9, 44_100 * 3, 4, &[b"son"]));
        std::fs::write(path, file).unwrap();
    }

    #[test]
    fn les_tags_sont_lus_malgre_la_piste_video() {
        let tmp = crate::library::tests::TempDir::new("ogg");
        let path = tmp.0.join("clip.ogg");
        theora_then_vorbis(&path, &["title=Funkorama", "ARTIST=Quelqu'un", "Album=Disque"]);
        let info = read(&path).unwrap();
        assert_eq!(info.title, "Funkorama");
        assert_eq!(info.artist, "Quelqu'un");
        assert_eq!(info.album, "Disque");
        assert!(info.has_audio && info.has_video);
        assert_eq!(info.duration_ms, 3000);
    }

    #[test]
    fn un_fichier_qui_n_est_pas_un_ogg_est_ignore() {
        let tmp = crate::library::tests::TempDir::new("ogg-faux");
        let path = tmp.0.join("faux.ogg");
        std::fs::write(&path, b"pas du tout un ogg").unwrap();
        assert!(read(&path).is_none());
    }
}
