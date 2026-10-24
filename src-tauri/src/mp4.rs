//! Lecture minimale d'un MP4 que `lofty` refuse (une vidéo sans piste son, par
//! exemple) : sa durée et la présence d'une piste audio.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub struct Info {
    pub duration_ms: u64,
    pub has_audio: bool,
    pub has_video: bool,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Profondeur maximale des boîtes : un vrai fichier en a 3 (moov, trak, mdia). Un
/// fichier piégé fait de boîtes imbriquées sans fin ne fait pas déborder la pile.
const MAX_DEPTH: u32 = 8;

/// Parcourt les boîtes entre `start` et `end` ; descend dans moov, trak et mdia.
/// Chaque taille est vérifiée : une boîte ne dépasse jamais son parent, et la position
/// avance toujours (pas de boucle sans fin sur une taille fabriquée).
fn walk<R: Read + Seek>(r: &mut R, start: u64, end: u64, depth: u32, info: &mut Info, found_moov: &mut bool) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut pos = start;
    while pos.checked_add(8)? <= end {
        r.seek(SeekFrom::Start(pos)).ok()?;
        let mut head = [0u8; 8];
        r.read_exact(&mut head).ok()?;
        let mut size = u32_at(&head, 0)? as u64;
        let kind = &head[4..8];
        let mut header = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            r.read_exact(&mut big).ok()?;
            size = u64::from_be_bytes(big);
            header = 16;
        } else if size == 0 {
            size = end - pos;
        }
        let box_end = pos.checked_add(size)?;
        if size < header || box_end > end {
            return None;
        }
        let body = pos + header;
        let body_len = size - header;
        match kind {
            b"moov" | b"trak" | b"mdia" => {
                if kind == b"moov" {
                    *found_moov = true;
                }
                walk(r, body, box_end, depth + 1, info, found_moov)?;
            }
            b"mvhd" if body_len >= 32 => {
                let mut b = vec![0u8; body_len.min(40) as usize];
                r.read_exact(&mut b).ok()?;
                let (scale, duration) = if b[0] == 1 {
                    (u32_at(&b, 20)? as u64, u64_at(&b, 24)?)
                } else {
                    (u32_at(&b, 12)? as u64, u32_at(&b, 16)? as u64)
                };
                if let Some(ms) = duration.saturating_mul(1000).checked_div(scale) {
                    info.duration_ms = ms;
                }
            }
            b"hdlr" if body_len >= 12 => {
                let mut b = [0u8; 12];
                r.read_exact(&mut b).ok()?;
                match &b[8..12] {
                    b"soun" => info.has_audio = true,
                    b"vide" => info.has_video = true,
                    _ => {}
                }
            }
            _ => {}
        }
        pos = box_end;
    }
    Some(())
}

pub fn read(path: &Path) -> Option<Info> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut r = BufReader::new(file);
    let mut info = Info { duration_ms: 0, has_audio: false, has_video: false };
    let mut found_moov = false;
    walk(&mut r, 0, len, 0, &mut info, &mut found_moov);
    found_moov.then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une boîte MP4 : taille sur 4 octets, type, contenu.
    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [((body.len() + 8) as u32).to_be_bytes().to_vec(), kind.to_vec(), body.to_vec()].concat()
    }

    #[test]
    fn video_sans_son_et_sa_duree() {
        let mvhd = bx(b"mvhd", &[vec![0u8; 12], 1000u32.to_be_bytes().to_vec(), 297_338u32.to_be_bytes().to_vec(), vec![0u8; 80]].concat());
        let hdlr = |h: &[u8; 4]| bx(b"hdlr", &[vec![0u8; 8], h.to_vec(), vec![0u8; 12]].concat());
        let video = bx(b"trak", &bx(b"mdia", &hdlr(b"vide")));
        let audio = bx(b"trak", &bx(b"mdia", &hdlr(b"soun")));
        let mute = [bx(b"ftyp", b"isom"), bx(b"mdat", &[0u8; 32]), bx(b"moov", &[mvhd.clone(), video.clone()].concat())].concat();
        let both = [bx(b"ftyp", b"isom"), bx(b"moov", &[mvhd, video, audio].concat())].concat();

        let path = std::env::temp_dir().join(format!("echo-mp4-{}.mp4", std::process::id()));
        std::fs::write(&path, mute).unwrap();
        let info = read(&path).unwrap();
        assert_eq!(info.duration_ms, 297_338);
        assert!(!info.has_audio);
        assert!(info.has_video);
        std::fs::write(&path, both).unwrap();
        assert!(read(&path).unwrap().has_audio);
        std::fs::write(&path, b"pas un mp4").unwrap();
        assert!(read(&path).is_none());
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod pieges {
    use super::*;
    use std::io::Cursor;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn un_fichier_piege_ne_fait_pas_planter() {
        // 5 000 boîtes « trak » imbriquées : la pile ne déborde pas.
        let mut data = Vec::new();
        for _ in 0..5_000 {
            data = boxed(b"trak", &data);
        }
        let data = boxed(b"moov", &data);
        let len = data.len() as u64;
        let mut info = Info { duration_ms: 0, has_audio: false, has_video: false };
        let mut moov = false;
        walk(&mut Cursor::new(data), 0, len, 0, &mut info, &mut moov);
        assert!(moov);

        // Taille 64 bits énorme : pas de boucle sans fin ni de débordement.
        let mut big = 1u32.to_be_bytes().to_vec();
        big.extend_from_slice(b"free");
        big.extend_from_slice(&(u64::MAX - 4).to_be_bytes());
        big.extend_from_slice(&[0u8; 32]);
        let len = big.len() as u64;
        assert!(walk(&mut Cursor::new(big), 0, len, 0, &mut info, &mut moov).is_none());
    }
}
