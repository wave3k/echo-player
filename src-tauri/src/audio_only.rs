//! Piste audio seule d'un clip, sous Linux. Le lecteur <audio> de WebKit décode aussi
//! la vidéo d'un fichier webm/mp4 (des images 4K pour rien : mémoire et processeur).
//! On lui donne donc une copie audio seule, faite sans réencodage (les paquets sont
//! recopiés tels quels, en une fraction de seconde), gardée en cache.

use gstreamer as gst;
use gstreamer::prelude::*;
use std::path::{Path, PathBuf};

/// Place maximale du cache audio : au-delà, les copies les plus anciennes partent.
const CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// Conteneur de sortie selon celui du clip (Matroska/WebM ou MP4).
fn plan(path: &Path) -> Option<(&'static str, &'static str, &'static str)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "webm" | "mkv" => Some(("matroskademux", "matroskamux", "mka")),
        "mp4" | "m4v" | "mov" => Some(("qtdemux", "mp4mux", "m4a")),
        _ => None,
    }
}

pub fn cache_dir(cache: &Path) -> PathBuf {
    cache.join("audio")
}

/// Chemin de la copie audio (créée si besoin). `None` : on lit le fichier d'origine.
/// Une copie à la fois : deux demandes pour le même clip (morceau suivant préparé,
/// puis lancé tout de suite) n'écrivent jamais le même fichier ensemble.
static BUSY: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn ensure(source: &Path, mtime: u64, cache: &Path) -> Option<PathBuf> {
    let (demux, mux, ext) = plan(source)?;
    let _one_at_a_time = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = cache_dir(cache);
    let name = format!(
        "{:016x}-{mtime}.{ext}",
        crate::library::stable_hash(source.to_string_lossy().as_bytes())
    );
    let out = dir.join(name);
    if out.metadata().map(|m| m.len() > 1024).unwrap_or(false) {
        return Some(out);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = out.with_extension(format!("{ext}.part"));
    let ok = remux(source, &tmp, demux, mux);
    if ok && tmp.metadata().map(|m| m.len() > 1024).unwrap_or(false) && std::fs::rename(&tmp, &out).is_ok() {
        Some(out)
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
    }
}

fn remux(source: &Path, out: &Path, demux: &str, mux: &str) -> bool {
    if gst::init().is_err() {
        return false;
    }
    let make = |name: &str| gst::ElementFactory::make(name).build().ok();
    let (Some(src), Some(demuxer), Some(queue), Some(muxer), Some(sink)) =
        (make("filesrc"), make(demux), make("queue"), make(mux), make("filesink"))
    else {
        return false;
    };
    src.set_property("location", source.to_string_lossy().as_ref());
    sink.set_property("location", out.to_string_lossy().as_ref());
    let pipeline = gst::Pipeline::new();
    if pipeline.add_many([&src, &demuxer, &queue, &muxer, &sink]).is_err()
        || src.link(&demuxer).is_err()
        || muxer.link(&sink).is_err()
    {
        return false;
    }
    // Une entrée « audio » du mux, explicitement (sinon il peut en choisir une vidéo).
    let linked = muxer
        .request_pad_simple("audio_%u")
        .zip(queue.static_pad("src"))
        .map(|(mux_in, queue_out)| queue_out.link(&mux_in).is_ok())
        .unwrap_or(false);
    if !linked {
        return false;
    }
    // La première piste audio, quel que soit son numéro (audio_0, audio_1…), va au mux ;
    // la vidéo reste débranchée : elle n'est ni décodée ni recopiée.
    let target = queue.static_pad("sink");
    let bin = pipeline.downgrade();
    demuxer.connect_pad_added(move |_, pad| {
        if let Some(sinkpad) = target.as_ref().filter(|p| pad.name().starts_with("audio") && !p.is_linked()) {
            let _ = pad.link(sinkpad);
            return;
        }
        // Autre piste (vidéo, sous-titres) : jetée telle quelle, sans décodage.
        if let (Some(bin), Ok(trash)) = (bin.upgrade(), gst::ElementFactory::make("fakesink").property("sync", false).property("async", false).build()) {
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
    let ok = pipeline
        .bus()
        .and_then(|bus| {
            bus.timed_pop_filtered(
                gst::ClockTime::from_seconds(90),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
        })
        .map(|msg| match msg.view() {
            gst::MessageView::Eos(_) => true,
            gst::MessageView::Error(e) => {
                if std::env::var_os("ECHO_DEBUG").is_some() {
                    eprintln!("copie audio : {} ({:?})", e.error(), e.debug());
                }
                false
            }
            _ => false,
        })
        .unwrap_or(false);
    let _ = pipeline.set_state(gst::State::Null);
    ok
}

/// Garde le cache sous sa limite : les copies les moins récemment créées partent d'abord.
pub fn prune(cache: &Path) {
    let Ok(read) = std::fs::read_dir(cache_dir(cache)) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seuls_les_clips_sont_recopies() {
        assert!(plan(Path::new("a.webm")).is_some());
        assert!(plan(Path::new("a.MP4")).is_some());
        assert!(plan(Path::new("a.mp3")).is_none());
        assert!(plan(Path::new("a.flac")).is_none());
    }

    /// Essai réel : ECHO_CLIP=fichier.webm cargo test audio_only::essai -- --ignored --nocapture
    #[test]
    #[ignore]
    fn essai() {
        let clip = PathBuf::from(std::env::var("ECHO_CLIP").unwrap());
        let dir = std::env::temp_dir().join("echo-audio-essai");
        let t = std::time::Instant::now();
        let out = ensure(&clip, 1, &dir);
        println!("{:?} en {} ms", out.as_ref().map(|p| p.metadata().map(|m| m.len()).unwrap_or(0)), t.elapsed().as_millis());
        assert!(out.is_some());
    }
}
