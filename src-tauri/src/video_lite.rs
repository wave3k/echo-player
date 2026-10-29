//! Copie légère d'un clip trop lourd pour la machine, sous Linux. Un clip AV1 en 4K
//! se décode en logiciel sur beaucoup de processeurs : l'image prend du retard sur le
//! son, et chaque recalage fait clignoter la vidéo. On en fait donc une fois, en
//! arrière-plan, une copie vidéo seule en 720p (VP8, images clés rapprochées pour que
//! les recalages soient légers), gardée en cache. Le fichier d'origine n'est pas touché.

use gstreamer as gst;
use gstreamer::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Place maximale du cache vidéo : au-delà, les copies les plus anciennes partent.
const CACHE_LIMIT: u64 = 6 * 1024 * 1024 * 1024;
/// Taille maximale de la copie : tient dans 1280 × 720.
const MAX_W: i32 = 1280;
const MAX_H: i32 = 720;
/// Au-delà de 30 images par seconde, des images sont sautées.
const MAX_FPS: i32 = 30;
/// Débit visé, en bits par seconde.
const BITRATE: i32 = 1_800_000;

/// Demande d'arrêt de la copie en cours (réglage désactivé, fermeture).
pub static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cache_dir(cache: &Path) -> PathBuf {
    cache.join("video")
}

fn out_path(source: &Path, mtime: u64, cache: &Path) -> PathBuf {
    let name = format!(
        "{:016x}-{mtime}.webm",
        crate::library::stable_hash(source.to_string_lossy().as_bytes())
    );
    cache_dir(cache).join(name)
}

/// Copie déjà faite pour ce fichier (même chemin, même date de modification).
pub fn existing(source: &Path, mtime: u64, cache: &Path) -> Option<PathBuf> {
    let out = out_path(source, mtime, cache);
    out.metadata().map(|m| m.len() > 1024).unwrap_or(false).then_some(out)
}

/// Fait la copie (bloquant, plusieurs minutes pour un long clip en 4K). `progress`
/// reçoit l'avancement, de 0 à 1. `None` : échec ou arrêt demandé.
pub fn make(source: &Path, mtime: u64, cache: &Path, progress: &dyn Fn(f32)) -> Option<PathBuf> {
    if let Some(done) = existing(source, mtime, cache) {
        return Some(done);
    }
    let out = out_path(source, mtime, cache);
    std::fs::create_dir_all(cache_dir(cache)).ok()?;
    let tmp = out.with_extension("webm.part");
    let ok = transcode(source, &tmp, progress);
    if ok && tmp.metadata().map(|m| m.len() > 1024).unwrap_or(false) && std::fs::rename(&tmp, &out).is_ok() {
        prune(cache);
        Some(out)
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
    }
}

/// Taille de la copie : l'image tient dans MAX_W × MAX_H, proportions gardées,
/// dimensions paires (exigées par l'encodeur).
fn fit(width: i32, height: i32) -> (i32, i32) {
    if width <= 0 || height <= 0 {
        return (MAX_W, MAX_H);
    }
    let scale = (MAX_W as f64 / width as f64).min(MAX_H as f64 / height as f64).min(1.0);
    let even = |v: f64| ((v / 2.0).round() as i32 * 2).max(2);
    (even(width as f64 * scale), even(height as f64 * scale))
}

fn transcode(source: &Path, out: &Path, progress: &dyn Fn(f32)) -> bool {
    if gst::init().is_err() {
        return false;
    }
    let make = |name: &str| gst::ElementFactory::make(name).build().ok();
    let (Some(src), Some(decode), Some(queue), Some(convert), Some(scale), Some(rate), Some(filter), Some(encode), Some(mux), Some(sink)) = (
        make("filesrc"),
        make("decodebin"),
        make("queue"),
        make("videoconvert"),
        make("videoscale"),
        make("videorate"),
        make("capsfilter"),
        make("vp8enc"),
        make("webmmux"),
        make("filesink"),
    ) else {
        return false;
    };
    src.set_property("location", source.to_string_lossy().as_ref());
    sink.set_property("location", out.to_string_lossy().as_ref());
    // Encodage rapide (temps réel), une image clé par seconde au plus : sauter dans la
    // copie ne demande jamais de redécoder plus d'une seconde d'images.
    encode.set_property("deadline", 1i64);
    encode.set_property("cpu-used", 8i32);
    encode.set_property("target-bitrate", BITRATE);
    encode.set_property("keyframe-max-dist", MAX_FPS);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
    encode.set_property("threads", threads as i32);

    let pipeline = gst::Pipeline::new();
    let chain = [&queue, &convert, &scale, &rate, &filter, &encode, &mux, &sink];
    if pipeline.add_many([&src, &decode]).is_err()
        || pipeline.add_many(chain).is_err()
        || src.link(&decode).is_err()
        || gst::Element::link_many(chain).is_err()
    {
        return false;
    }

    // La première piste vidéo décodée va vers la copie ; le reste (son, sous-titres)
    // est jeté. La taille et la cadence de la copie se décident sur celles de la source.
    let target = queue.static_pad("sink");
    let bin = pipeline.downgrade();
    let filter_ref = filter.clone();
    decode.connect_pad_added(move |_, pad| {
        let caps = pad.current_caps().or_else(|| Some(pad.query_caps(None)));
        let video = caps
            .as_ref()
            .and_then(|c| c.structure(0))
            .filter(|s| s.name().starts_with("video/"))
            .map(|s| {
                let w = s.get::<i32>("width").unwrap_or(0);
                let h = s.get::<i32>("height").unwrap_or(0);
                let fps = s.get::<gst::Fraction>("framerate").ok();
                (w, h, fps)
            });
        if let (Some((w, h, fps)), Some(sinkpad)) = (video, target.as_ref().filter(|p| !p.is_linked())) {
            let (tw, th) = fit(w, h);
            let mut wanted = gst::Caps::builder("video/x-raw")
                .field("width", tw)
                .field("height", th)
                .field("pixel-aspect-ratio", gst::Fraction::new(1, 1));
            let too_fast = fps.map(|f| f.denom() > 0 && f.numer() > MAX_FPS * f.denom()).unwrap_or(false);
            if too_fast {
                wanted = wanted.field("framerate", gst::Fraction::new(MAX_FPS, 1));
            }
            filter_ref.set_property("caps", wanted.build());
            let _ = pad.link(sinkpad);
            return;
        }
        if let (Some(bin), Ok(trash)) = (
            bin.upgrade(),
            gst::ElementFactory::make("fakesink").property("sync", false).property("async", false).build(),
        ) {
            if bin.add(&trash).is_ok() {
                let _ = trash.sync_state_with_parent();
                if let Some(p) = trash.static_pad("sink") {
                    let _ = pad.link(&p);
                }
            }
        }
    });

    if pipeline.set_state(gst::State::Playing).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        return false;
    }
    let Some(bus) = pipeline.bus() else {
        let _ = pipeline.set_state(gst::State::Null);
        return false;
    };
    let mut last = -1.0f32;
    let ok = loop {
        if CANCEL.load(Ordering::Relaxed) {
            break false;
        }
        if let Some(msg) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(500),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) {
            match msg.view() {
                gst::MessageView::Eos(_) => break true,
                gst::MessageView::Error(e) => {
                    if std::env::var_os("ECHO_DEBUG").is_some() {
                        eprintln!("copie légère : {} ({:?})", e.error(), e.debug());
                    }
                    break false;
                }
                _ => {}
            }
        }
        let pos = pipeline.query_position::<gst::ClockTime>();
        let dur = pipeline.query_duration::<gst::ClockTime>();
        if let (Some(pos), Some(dur)) = (pos, dur) {
            if dur.nseconds() > 0 {
                let done = (pos.nseconds() as f64 / dur.nseconds() as f64).clamp(0.0, 1.0) as f32;
                if done - last >= 0.01 {
                    last = done;
                    progress(done);
                }
            }
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    ok
}

/// Ce que dit l'en-tête de la piste vidéo, sans rien décoder.
#[derive(serde::Serialize, Clone, Copy, Default)]
pub struct Probe {
    pub width: i32,
    pub height: i32,
    /// Images par seconde (0 si inconnu).
    pub fps: f64,
}

/// Taille et cadence d'un clip, lues par le démultiplexeur (quelques millisecondes).
pub fn probe(source: &Path) -> Option<Probe> {
    if gst::init().is_err() {
        return None;
    }
    let src = gst::ElementFactory::make("filesrc").build().ok()?;
    let parse = gst::ElementFactory::make("parsebin").build().ok()?;
    src.set_property("location", source.to_string_lossy().as_ref());
    let pipeline = gst::Pipeline::new();
    pipeline.add_many([&src, &parse]).ok()?;
    src.link(&parse).ok()?;
    // Chaque piste finit dans un puits qui ne bloque jamais : les données passent, et
    // l'en-tête de la piste vidéo apparaît sur sa sortie dès les premiers paquets.
    let bin = pipeline.downgrade();
    parse.connect_pad_added(move |_, pad| {
        if let (Some(bin), Ok(trash)) = (
            bin.upgrade(),
            gst::ElementFactory::make("fakesink").property("sync", false).property("async", false).build(),
        ) {
            if bin.add(&trash).is_ok() {
                let _ = trash.sync_state_with_parent();
                if let Some(p) = trash.static_pad("sink") {
                    let _ = pad.link(&p);
                }
            }
        }
    });
    let read = |parse: &gst::Element| {
        parse.src_pads().iter().find_map(|pad| {
            let caps = pad.current_caps()?;
            let s = caps.structure(0)?;
            if !s.name().starts_with("video/") {
                return None;
            }
            let fps = s
                .get::<gst::Fraction>("framerate")
                .ok()
                .filter(|f| f.denom() > 0)
                .map(|f| f.numer() as f64 / f.denom() as f64)
                .unwrap_or(0.0);
            Some(Probe { width: s.get("width").unwrap_or(0), height: s.get("height").unwrap_or(0), fps })
        })
    };
    let mut out = None;
    // En lecture et sans horloge (sync=false) : rien ne bloque ; on s'arrête dès que
    // l'en-tête est connu, bien avant d'avoir lu le fichier.
    if pipeline.set_state(gst::State::Playing).is_ok() {
        let bus = pipeline.bus();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while out.is_none() && std::time::Instant::now() < until {
            let failed = bus
                .as_ref()
                .and_then(|b| b.timed_pop_filtered(gst::ClockTime::from_mseconds(20), &[gst::MessageType::Error]))
                .is_some();
            if failed {
                break;
            }
            out = read(&parse);
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    out
}

/// Garde le cache sous sa limite : les copies les plus anciennes partent d'abord.
/// Les copies interrompues (« .part ») partent toujours.
pub fn prune(cache: &Path) {
    let Ok(read) = std::fs::read_dir(cache_dir(cache)) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "part") {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if let Ok(m) = entry.metadata() {
            if let Ok(t) = m.modified() {
                files.push((t, m.len(), path));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= CACHE_LIMIT {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Place prise par les copies, en octets.
pub fn size(cache: &Path) -> u64 {
    std::fs::read_dir(cache_dir(cache))
        .map(|r| r.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

/// Le fil qui fait les copies passe après le reste de la machine (et ses fils, ceux
/// de GStreamer compris, héritent de cette priorité).
pub fn lower_priority() {
    // SAFETY : appels système sans pointeur ; un échec laisse simplement la priorité.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 15);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_copie_tient_dans_720p_sans_deformer() {
        assert_eq!(fit(3840, 2160), (1280, 720));
        assert_eq!(fit(3840, 1600), (1280, 534));
        assert_eq!(fit(1080, 1920), (406, 720));
        assert_eq!(fit(640, 480), (640, 480));
        assert_eq!(fit(0, 0), (1280, 720));
    }

    /// Lecture de l'en-tête d'un vrai clip (ignoré par défaut, même variable).
    #[test]
    #[ignore]
    fn essai_probe() {
        let Some(sample) = std::env::var_os("ECHO_LITE_SAMPLE") else { return };
        let p = probe(Path::new(&sample)).expect("en-tête lu");
        eprintln!("{} × {} à {} images/s", p.width, p.height, p.fps);
        assert!(p.width > 0 && p.fps > 0.0);
    }

    /// Essai réel (ignoré par défaut) : une vidéo de test 4K en AV1 ou VP9 devient une
    /// copie 720p lisible. `ECHO_LITE_SAMPLE=/chemin/clip.webm cargo test -- --ignored`
    #[test]
    #[ignore]
    fn essai_copie_legere() {
        let Some(sample) = std::env::var_os("ECHO_LITE_SAMPLE") else { return };
        let dir = std::env::temp_dir().join(format!("echo-lite-{}", std::process::id()));
        let started = std::time::Instant::now();
        let out = make(Path::new(&sample), 1, &dir, &|p| eprint!("\r{:.0} %", p * 100.0)).expect("copie");
        eprintln!("\ncopie : {:?} en {:?}", out.metadata().map(|m| m.len()), started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
