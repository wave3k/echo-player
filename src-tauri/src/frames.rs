//! Pochette tirée d'une vidéo, sous Linux : GStreamer (déjà utilisé par le moteur
//! d'affichage) décode une seule image clé, hors de l'interface. Bien plus léger
//! que lire la vidéo dans la page : une 4K AV1 ne coûte qu'une image décodée.

use gstreamer as gst;
use gstreamer::prelude::*;
use std::path::Path;
use std::time::Duration;

/// Côté de la pochette produite, en pixels.
const SIDE: usize = 480;

pub enum Frame {
    /// Image JPEG carrée, prise au centre de la vidéo.
    Image(Vec<u8>),
    /// La vidéo n'a pas d'image exploitable : inutile de réessayer.
    None,
    /// Le système ne sait pas décoder cette vidéo (greffon manquant) : on réessaiera.
    Undecodable,
}

/// Points essayés, en fraction de la durée : le tiers évite le noir du début.
const POINTS: [f64; 4] = [0.33, 0.5, 0.18, 0.7];

pub fn grab(path: &Path) -> Frame {
    if gst::init().is_err() {
        return Frame::Undecodable;
    }
    let Ok(uri) = gst::glib::filename_to_uri(path, None) else {
        return Frame::None;
    };
    // Recadrage au carré et réduction avant la conversion : on ne convertit que 480×480 pixels.
    let desc = format!(
        "uridecodebin name=src ! aspectratiocrop aspect-ratio=1/1 ! videoscale ! videoconvert ! \
         video/x-raw,format=RGB,width={SIDE},height={SIDE},pixel-aspect-ratio=1/1 ! \
         appsink name=sink sync=false max-buffers=1 drop=true"
    );
    let Ok(pipeline) = gst::parse::launch(&desc).map(|e| e.downcast::<gst::Pipeline>()) else {
        return Frame::Undecodable;
    };
    let Ok(pipeline) = pipeline else { return Frame::Undecodable };
    let (Some(src), Some(sink)) = (pipeline.by_name("src"), pipeline.by_name("sink")) else {
        return Frame::Undecodable;
    };
    src.set_property("uri", uri.as_str());
    let result = run(&pipeline, &sink);
    let _ = pipeline.set_state(gst::State::Null);
    result
}

fn run(pipeline: &gst::Pipeline, sink: &gst::Element) -> Frame {
    if pipeline.set_state(gst::State::Paused).is_err() {
        return Frame::Undecodable;
    }
    let Some(bus) = pipeline.bus() else { return Frame::Undecodable };
    if let Err(frame) = settle(&bus) {
        return frame;
    }
    let duration = pipeline.query_duration::<gst::ClockTime>();
    let mut best: Option<(f64, Vec<u8>)> = None;
    for point in POINTS {
        if let Some(d) = duration {
            let at = gst::ClockTime::from_nseconds((d.nseconds() as f64 * point) as u64);
            if pipeline
                .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, at)
                .is_err()
            {
                break;
            }
            if settle(&bus).is_err() {
                break;
            }
        }
        let Some(rgb) = preroll(sink) else { break };
        let light = brightness(&rgb);
        if best.as_ref().map(|(l, _)| light > *l).unwrap_or(true) {
            best = Some((light, rgb));
        }
        // Une image ni noire ni vide : on la garde.
        if light >= 18.0 || duration.is_none() {
            break;
        }
    }
    match best.and_then(|(_, rgb)| jpeg(&rgb)) {
        Some(bytes) => Frame::Image(bytes),
        None => Frame::None,
    }
}

/// Attend que la vidéo soit prête (première image décodée), ou l'échec.
fn settle(bus: &gst::Bus) -> Result<(), Frame> {
    let types = [gst::MessageType::AsyncDone, gst::MessageType::Error];
    let timeout = gst::ClockTime::from_seconds(12);
    match bus.timed_pop_filtered(timeout, &types) {
        Some(msg) => match msg.view() {
            gst::MessageView::AsyncDone(_) => Ok(()),
            gst::MessageView::Error(err) => {
                let e = err.error();
                if e.matches(gst::CoreError::MissingPlugin) || e.matches(gst::StreamError::CodecNotFound) {
                    Err(Frame::Undecodable)
                } else {
                    Err(Frame::None)
                }
            }
            _ => Err(Frame::None),
        },
        None => Err(Frame::None),
    }
}

/// L'image prête dans le collecteur, en RGB 480×480.
fn preroll(sink: &gst::Element) -> Option<Vec<u8>> {
    let wait = Duration::from_secs(5).as_nanos() as u64;
    let sample = sink.emit_by_name::<Option<gst::Sample>>("try-pull-preroll", &[&wait])?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let data = map.as_slice();
    (data.len() >= SIDE * SIDE * 3).then(|| data[..SIDE * SIDE * 3].to_vec())
}

/// Luminosité moyenne (0 à 255), mesurée sur un pixel sur 64.
fn brightness(rgb: &[u8]) -> f64 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for px in rgb.chunks_exact(3).step_by(64) {
        sum += px[0] as u64 + px[1] as u64 + px[2] as u64;
        n += 3;
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

fn jpeg(rgb: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 86)
        .encode(rgb, SIDE as u16, SIDE as u16, jpeg_encoder::ColorType::Rgb)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_luminosite_distingue_le_noir() {
        assert!(brightness(&vec![0u8; SIDE * SIDE * 3]) < 1.0);
        assert!(brightness(&vec![200u8; SIDE * SIDE * 3]) > 199.0);
    }

    #[test]
    fn une_image_est_encodee_en_jpeg() {
        let bytes = jpeg(&vec![120u8; SIDE * SIDE * 3]).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn un_fichier_absent_ne_donne_pas_d_image() {
        assert!(!matches!(grab(Path::new("/nulle/part/video.webm")), Frame::Image(_)));
    }
}

/// Essai sur de vrais fichiers : ECHO_FRAMES=dossier cargo test frames -- --ignored
#[cfg(test)]
#[test]
#[ignore]
fn essai_sur_un_dossier() {
    let dir = std::env::var("ECHO_FRAMES").expect("ECHO_FRAMES");
    let out = std::env::temp_dir().join("echo-frames");
    let _ = std::fs::create_dir_all(&out);
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let t = std::time::Instant::now();
        let name = entry.file_name().to_string_lossy().to_string();
        let what = match grab(&entry.path()) {
            Frame::Image(b) => {
                std::fs::write(out.join(format!("{name}.jpg")), &b).unwrap();
                format!("image {} o", b.len())
            }
            Frame::None => "aucune".into(),
            Frame::Undecodable => "indécodable".into(),
        };
        println!("{:>6} ms  {what:<16} {name}", t.elapsed().as_millis());
    }
}
