//! Vignettes des pochettes. Une liste n'affiche qu'un carré de 36 px : y poser
//! l'image d'origine (500 px et plus) oblige le moteur d'affichage à la garder
//! décodée en entier, près d'un Mo par pochette. La vignette en pèse 60 Ko.

use std::path::{Path, PathBuf};

/// Côté de la vignette, en pixels : de quoi rester net sur un écran haute densité.
pub const SIZE: u32 = 128;

pub fn dir(covers_dir: &Path) -> PathBuf {
    covers_dir.join("thumbs")
}

/// Vignette d'une pochette (créée si besoin). `None` : l'image d'origine est déjà
/// petite, ou son format ne se lit pas ici (GIF…) ; on garde alors l'originale.
pub fn ensure(full: &Path) -> Option<PathBuf> {
    let covers = full.parent()?;
    let out = dir(covers).join(format!("{}.jpg", full.file_name()?.to_string_lossy()));
    if out.is_file() {
        return Some(out);
    }
    let img = image::ImageReader::open(full).ok()?.with_guessed_format().ok()?.decode().ok()?;
    if img.width() <= SIZE * 3 / 2 && img.height() <= SIZE * 3 / 2 {
        return None;
    }
    // Recadrée au centre, comme l'affichage (« cover ») le ferait.
    let small = img.resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Triangle).to_rgb8();
    std::fs::create_dir_all(out.parent()?).ok()?;
    let tmp = out.with_extension("part");
    let ok = std::fs::File::create(&tmp)
        .ok()
        .map(std::io::BufWriter::new)
        .and_then(|mut file| {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 82)
                .encode_image(&small)
                .ok()
        })
        .is_some();
    if ok && std::fs::rename(&tmp, &out).is_ok() {
        Some(out)
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
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
        let thumb = ensure(&big).unwrap();
        let img = image::open(&thumb).unwrap();
        assert_eq!((img.width(), img.height()), (SIZE, SIZE));
        // Deuxième demande : la vignette existante est reprise telle quelle.
        assert_eq!(ensure(&big), Some(thumb));

        let small = dir.join("petite.png");
        image::RgbImage::new(100, 100).save(&small).unwrap();
        assert!(ensure(&small).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
