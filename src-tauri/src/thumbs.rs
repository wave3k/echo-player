//! Vignettes des pochettes. Une liste n'affiche qu'un carré de 36 px : y poser
//! l'image d'origine (500 px et plus) oblige le moteur d'affichage à la garder
//! décodée en entier, près d'un Mo par pochette. La vignette en pèse 60 Ko.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Côté de la vignette, en pixels : de quoi rester net sur un écran haute densité.
pub const SIZE: u32 = 128;

/// Une vignette faite à la fois : décoder plusieurs grandes images en parallèle
/// prendrait autant de mémoire que l'économie visée.
static LOCK: Mutex<()> = Mutex::new(());

pub fn dir(covers_dir: &Path) -> PathBuf {
    covers_dir.join("thumbs")
}

fn thumb_of(full: &Path) -> Option<PathBuf> {
    let covers = full.parent()?;
    Some(dir(covers).join(format!("{}.jpg", full.file_name()?.to_string_lossy())))
}

/// Marque « pas de vignette » (image petite ou illisible) : on ne réessaie pas.
fn marker_of(full: &Path) -> Option<PathBuf> {
    let covers = full.parent()?;
    Some(dir(covers).join(format!("{}.none", full.file_name()?.to_string_lossy())))
}

/// La vignette si elle est déjà faite (rien n'est créé ici).
pub fn existing(full: &Path) -> Option<PathBuf> {
    thumb_of(full).filter(|t| t.is_file())
}

/// Vrai si la question est déjà réglée : vignette faite, ou marquée impossible.
pub fn settled(full: &Path) -> bool {
    existing(full).is_some() || marker_of(full).is_some_and(|m| m.is_file())
}

/// Vignette d'une pochette (créée si besoin). `None` : l'image d'origine est déjà
/// petite, ou son format ne se lit pas ici (GIF…) ; on garde alors l'originale.
pub fn ensure(full: &Path) -> Option<PathBuf> {
    let out = thumb_of(full)?;
    let marker = marker_of(full)?;
    if out.is_file() {
        return Some(out);
    }
    if marker.is_file() {
        return None;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Faite pendant qu'on attendait le verrou ?
    if out.is_file() {
        return Some(out);
    }
    match make(full, &out) {
        Made::Thumb => Some(out),
        Made::Never => {
            // Petite ou illisible : noté une fois pour toutes.
            if std::fs::create_dir_all(out.parent()?).is_ok() {
                let _ = std::fs::write(&marker, b"");
            }
            None
        }
        Made::Failed => None,
    }
}

enum Made {
    Thumb,
    /// Pas de vignette pour cette image, inutile de réessayer.
    Never,
    /// Échec d'écriture : on réessaiera.
    Failed,
}

fn make(full: &Path, out: &Path) -> Made {
    // Pochette absente (cache en train d'être vidé) : rien à conclure.
    if !full.is_file() {
        return Made::Failed;
    }
    // Les dimensions se lisent dans l'en-tête : une petite image n'est pas décodée.
    // Au-delà de 8000 px de côté, ce n'est pas une pochette : pas de décodage géant.
    match image::image_dimensions(full) {
        Ok((w, h)) if (w > SIZE * 3 / 2 || h > SIZE * 3 / 2) && w <= 8000 && h <= 8000 => {}
        _ => return Made::Never,
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let Some(img) = image::ImageReader::open(full)
        .ok()
        .and_then(|r| r.with_guessed_format().ok())
        .and_then(|mut r| {
            r.limits(limits);
            r.decode().ok()
        })
    else {
        return Made::Never;
    };
    // Recadrée au centre, comme l'affichage (« cover ») le ferait.
    let small = img.resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Triangle).to_rgb8();
    drop(img);
    let Some(parent) = out.parent() else { return Made::Failed };
    if std::fs::create_dir_all(parent).is_err() {
        return Made::Failed;
    }
    let tmp = crate::store::unique_tmp(out, "part");
    let ok = std::fs::File::create(&tmp)
        .ok()
        .map(std::io::BufWriter::new)
        .and_then(|mut file| {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 82)
                .encode_image(&small)
                .ok()
        })
        .is_some();
    if ok && std::fs::rename(&tmp, out).is_ok() {
        Made::Thumb
    } else {
        let _ = std::fs::remove_file(&tmp);
        Made::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_grande_pochette_donne_une_vignette() {
        let dir = std::env::temp_dir().join(format!("echo-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("grande.png");
        image::RgbImage::from_pixel(600, 400, image::Rgb([200, 30, 30])).save(&big).unwrap();
        assert!(existing(&big).is_none());
        let thumb = ensure(&big).unwrap();
        let img = image::open(&thumb).unwrap();
        assert_eq!((img.width(), img.height()), (SIZE, SIZE));
        // Deuxième demande : la vignette existante est reprise telle quelle.
        assert_eq!(ensure(&big), Some(thumb.clone()));
        assert_eq!(existing(&big), Some(thumb));

        let small = dir.join("petite.png");
        image::RgbImage::new(100, 100).save(&small).unwrap();
        assert!(ensure(&small).is_none());
        assert!(settled(&small), "une petite image est notée une fois pour toutes");

        // Image abîmée : pas de vignette, et pas de nouvel essai.
        let broken = dir.join("abimee.jpg");
        std::fs::write(&broken, b"pas une image").unwrap();
        assert!(ensure(&broken).is_none());
        assert!(settled(&broken));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
