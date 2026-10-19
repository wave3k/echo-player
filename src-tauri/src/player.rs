//! Lecteur audio d'Écho sous Linux, dans le cœur de l'app (GStreamer).
//!
//! La page web n'est plus qu'un écran posé dessus : elle envoie des ordres (lire,
//! suivant, volume…) et reçoit l'état du lecteur. La musique ne dépend donc plus
//! d'elle : rangé dans la barre du système, Écho ferme sa page (≈ 200 Mo de moins)
//! et continue de jouer, de passer au morceau suivant et de répondre aux touches média.
//!
//! Deux lecteurs servent au fondu enchaîné. Chacun passe par la même chaîne :
//! égaliseur 5 bandes → mesure du niveau → volume égal → fondu → limiteur.

use gstreamer as gst;
use gstreamer::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::library::Track;

/// Fréquences de l'égaliseur, et préréglages en dB (les mêmes que l'interface).
const BANDS: [f64; 5] = [60.0, 250.0, 1000.0, 4000.0, 12000.0];

fn eq_preset(name: &str) -> [f64; 5] {
    match name {
        "bass" => [6.0, 3.5, 0.0, 0.0, 1.0],
        "vocal" => [-2.0, -1.0, 2.5, 3.5, 1.0],
        "treble" => [-1.0, 0.0, 0.0, 2.5, 5.0],
        "rap" => [6.5, 3.0, -1.5, 1.5, 3.0],
        "acoustic" => [2.5, 1.0, 1.0, 2.0, 3.0],
        _ => [0.0; 5],
    }
}

/// Volume égal : niveau visé (proche des −14 LUFS des plateformes) et bornes de la
/// correction, pour ne pas faire ressortir le souffle d'un passage très calme.
const TARGET_RMS: f64 = 0.18;
const MIN_GAIN: f64 = 0.2;
const MAX_GAIN: f64 = 5.6;

/* ---------------- un lecteur ---------------- */

struct Deck {
    pipe: gst::Pipeline,
    eq: gst::Element,
    master: gst::Element,
    norm: gst::Element,
    fade: gst::Element,
    limiter: gst::Element,
    id: String,
    duration_ms: u64,
    /// Position à rejoindre dès que le fichier est prêt (reprise, saut demandé tôt).
    pending_seek: Option<u64>,
}

impl Drop for Deck {
    fn drop(&mut self) {
        let _ = self.pipe.set_state(gst::State::Null);
    }
}

impl Deck {
    fn open(path: &Path, id: &str) -> Result<Deck, String> {
        let uri = gst::glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
        // Le son seul : les autres pistes d'un clip (image, sous-titres) ne sont pas
        // décodées du tout. (playbin, lui, décode l'image même sans l'afficher.)
        let source = gst::ElementFactory::make("uridecodebin")
            .property("uri", uri.as_str())
            .property("caps", gst::Caps::new_empty_simple("audio/x-raw"))
            .property("expose-all-streams", false)
            .build()
            .map_err(|e| e.to_string())?;
        let chain = gst::parse::bin_from_description(
            "audioconvert ! audioresample ! equalizer-nbands name=eq num-bands=5 \
             ! level name=meter interval=250000000 post-messages=true \
             ! volume name=norm ! volume name=fade ! volume name=master \
             ! audiodynamic name=limiter characteristics=hard-knee mode=compressor threshold=0.84 \
             ! audioconvert ! autoaudiosink",
            true,
        )
        .map_err(|e| e.to_string())?;
        let pipe = gst::Pipeline::new();
        pipe.add_many([&source, chain.upcast_ref()]).map_err(|e| e.to_string())?;
        let input = chain.static_pad("sink").ok_or("entrée absente")?;
        source.connect_pad_added(move |_, pad| {
            if !input.is_linked() {
                let _ = pad.link(&input);
            }
        });
        let get = |name: &str| chain.by_name(name).ok_or_else(|| format!("élément {name} absent"));
        let deck = Deck {
            eq: get("eq")?,
            master: get("master")?,
            norm: get("norm")?,
            fade: get("fade")?,
            limiter: get("limiter")?,
            pipe,
            id: id.to_string(),
            duration_ms: 0,
            pending_seek: None,
        };
        if let Some(proxy) = deck.eq.dynamic_cast_ref::<gst::ChildProxy>() {
            for (i, freq) in BANDS.iter().enumerate() {
                if let Some(band) = proxy.child_by_index(i as u32) {
                    band.set_property("freq", *freq);
                    band.set_property("bandwidth", freq / 0.9);
                    let kind = if i == 0 { "low-shelf" } else if i == BANDS.len() - 1 { "high-shelf" } else { "peak" };
                    band.set_property_from_str("type", kind);
                }
            }
        }
        Ok(deck)
    }

    fn set_eq(&self, gains: &[f64; 5], limit: bool) {
        if let Some(proxy) = self.eq.dynamic_cast_ref::<gst::ChildProxy>() {
            for (i, g) in gains.iter().enumerate() {
                if let Some(band) = proxy.child_by_index(i as u32) {
                    band.set_property("gain", *g);
                }
            }
        }
        // Limiteur : seulement quand l'égaliseur ou le volume égal peuvent faire saturer.
        self.limiter.set_property("ratio", if limit { 0.05f32 } else { 1.0f32 });
    }

    fn position_ms(&self) -> u64 {
        if let Some(ms) = self.pending_seek {
            return ms;
        }
        self.pipe.query_position::<gst::ClockTime>().map(|t| t.mseconds()).unwrap_or(0)
    }

    fn seek(&mut self, ms: u64) {
        let ok = self
            .pipe
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, gst::ClockTime::from_mseconds(ms))
            .is_ok();
        // Pas encore prêt : le saut sera fait dès que le fichier sera ouvert.
        self.pending_seek = if ok { None } else { Some(ms) };
    }

    fn learn_duration(&mut self) -> bool {
        if self.duration_ms > 0 {
            return false;
        }
        match self.pipe.query_duration::<gst::ClockTime>() {
            Some(d) if d.mseconds() > 0 => {
                self.duration_ms = d.mseconds();
                true
            }
            _ => false,
        }
    }
}

/* ---------------- la file ---------------- */

/// File de lecture et mémoire de l'aléatoire (mêmes règles que `logic.js`).
#[derive(Default)]
struct Plan {
    ids: Vec<String>,
    index: usize,
    shuffle: bool,
    repeat: String,
    played: HashSet<usize>,
    history: Vec<usize>,
}

impl Plan {
    /// Le morceau suivant. `auto` : la fin du morceau, pas un clic sur Suivant.
    fn next(&mut self, auto: bool, rand: &mut impl FnMut(usize) -> usize) -> Option<usize> {
        let n = self.ids.len();
        if n == 0 {
            return None;
        }
        if auto && self.repeat == "one" {
            return Some(self.index);
        }
        if self.shuffle && n > 1 {
            self.played.insert(self.index);
            let mut pool: Vec<usize> = (0..n).filter(|i| !self.played.contains(i)).collect();
            if pool.is_empty() {
                if auto && self.repeat == "off" {
                    return None;
                }
                self.played = HashSet::from([self.index]);
                pool = (0..n).filter(|&i| i != self.index).collect();
            }
            return Some(pool[rand(pool.len())]);
        }
        if self.index + 1 < n {
            Some(self.index + 1)
        } else if auto && self.repeat != "all" {
            None
        } else {
            Some(0)
        }
    }

    /// Le morceau précédent : en aléatoire, celui qui a vraiment joué avant.
    fn prev(&mut self) -> Option<usize> {
        let n = self.ids.len();
        if n == 0 {
            return None;
        }
        if self.shuffle {
            if let Some(back) = self.history.pop() {
                self.played.remove(&self.index);
                if back < n {
                    return Some(back);
                }
            }
        }
        Some(if self.index == 0 { n - 1 } else { self.index - 1 })
    }

}

/* ---------------- ce que la page reçoit ---------------- */

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub id: Option<String>,
    pub index: usize,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// Vrai quand le morceau vient de changer (fin, fondu, touche média…).
    pub changed: bool,
}

#[derive(Serialize)]
pub struct Snapshot {
    status: Status,
    ids: Vec<String>,
    shuffle: bool,
    repeat: String,
}

/// Ce qu'un changement d'état entraîne hors du lecteur (fait sans tenir son verrou).
enum Effect {
    Started(Track),
    Playback(bool, u64),
    Save(Option<String>, u64, bool),
    Emit(Status),
    Error(String),
}

/* ---------------- le moteur ---------------- */

pub struct Engine {
    decks: [Option<Deck>; 2],
    active: usize,
    plan: Plan,
    seed: u64,
    playing: bool,

    volume: f64,
    muted: bool,
    eq: [f64; 5],
    loudness: bool,
    crossfade_s: u32,
    next_on_error: bool,
    skip_s: u32,

    fade: Option<(Instant, u64)>,
    norm_target: f64,
    norm_now: f64,
    norm_tau: f64,
    measured: f64,
    samples: u32,
    gains: HashMap<String, f64>,
    gains_dirty: bool,

    last_tick: Instant,
    last_second: u64,
    last_push: Instant,
    last_save: Instant,
    errors: u32,
}

pub struct Player(pub Mutex<Option<Engine>>);

impl Player {
    /// Le lecteur, si GStreamer et ses greffons de base sont là ; sinon la page lit
    /// elle-même la musique, comme avant.
    pub fn new(app: &AppHandle) -> Self {
        let ready = gst::init().is_ok()
            && ["playbin", "equalizer-nbands", "level", "audiodynamic", "volume", "audioconvert"]
                .iter()
                .all(|f| gst::ElementFactory::find(f).is_some());
        if !ready || std::env::var_os("ECHO_WEB_AUDIO").is_some() {
            return Self(Mutex::new(None));
        }
        let gains = std::fs::read(gains_file(app))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(7)
            | 1;
        let now = Instant::now();
        Self(Mutex::new(Some(Engine {
            decks: [None, None],
            active: 0,
            plan: Plan { repeat: "off".into(), ..Default::default() },
            seed,
            playing: false,
            volume: 0.9,
            muted: false,
            eq: [0.0; 5],
            loudness: false,
            crossfade_s: 0,
            next_on_error: true,
            skip_s: 10,
            fade: None,
            norm_target: 1.0,
            norm_now: 1.0,
            norm_tau: 0.4,
            measured: 0.0,
            samples: 0,
            gains,
            gains_dirty: false,
            last_tick: now,
            last_second: u64::MAX,
            last_push: now,
            last_save: now,
            errors: 0,
        })))
    }
}

fn gains_file(app: &AppHandle) -> std::path::PathBuf {
    crate::config_dir(app).join("gains.json")
}

fn track_of(app: &AppHandle, id: &str) -> Option<Track> {
    let state = app.state::<crate::AppData>();
    let data = state.0.lock().unwrap();
    data.tracks.iter().find(|t| t.id == id).cloned()
}

impl Engine {
    /// Le suivant dans la file (`auto` : fin du morceau), tirage aléatoire compris.
    fn pick_next(&mut self, auto: bool) -> Option<usize> {
        let seed = &mut self.seed;
        self.plan.next(auto, &mut |n| lcg(seed, n))
    }

    fn deck(&self) -> Option<&Deck> {
        self.decks[self.active].as_ref()
    }

    fn deck_mut(&mut self) -> Option<&mut Deck> {
        self.decks[self.active].as_mut()
    }

    pub fn configure(&mut self, s: &crate::store::Settings) {
        self.volume = (s.volume as f64 / 100.0).clamp(0.0, 1.0);
        self.eq = eq_preset(&s.eq);
        let was = self.loudness;
        self.loudness = s.loudness;
        self.crossfade_s = s.crossfade;
        self.next_on_error = s.next_on_error;
        self.skip_s = s.skip_seconds.max(1);
        if was != self.loudness {
            let id = self.deck().map(|d| d.id.clone());
            self.begin_level(id.as_deref());
        }
        if !self.loudness {
            self.norm_target = self.eq_compensation();
        }
        self.apply_sound();
    }

    /// L'égaliseur ajoute du volume : on le compense pour ne pas saturer.
    fn eq_compensation(&self) -> f64 {
        let boost = self.eq.iter().cloned().fold(0.0, f64::max);
        10f64.powf(-boost / 20.0 / 1.5)
    }

    fn apply_sound(&self) {
        let limit = self.loudness || self.eq.iter().any(|g| *g != 0.0);
        for deck in self.decks.iter().flatten() {
            deck.set_eq(&self.eq, limit);
            deck.master.set_property("volume", self.volume);
            deck.master.set_property("mute", self.muted);
        }
    }

    /// Nouveau morceau : on reprend la correction de volume déjà apprise pour lui.
    fn begin_level(&mut self, id: Option<&str>) {
        self.measured = 0.0;
        self.samples = 0;
        let known = id.and_then(|id| self.gains.get(id).copied());
        self.norm_target = if self.loudness { known.unwrap_or(1.0) } else { self.eq_compensation() };
        self.norm_now = self.norm_target;
        self.norm_tau = 0.4;
        if let (true, Some(g)) = (self.loudness, known) {
            self.measured = TARGET_RMS / g;
            self.samples = 12;
        }
    }

    fn status(&self, changed: bool) -> Status {
        let deck = self.deck();
        Status {
            id: deck.map(|d| d.id.clone()),
            index: self.plan.index,
            playing: self.playing && deck.is_some(),
            position_ms: deck.map(|d| d.position_ms()).unwrap_or(0),
            duration_ms: deck.map(|d| d.duration_ms).unwrap_or(0),
            changed,
        }
    }

    fn report(&mut self, fx: &mut Vec<Effect>, changed: bool) {
        let s = self.status(changed);
        fx.push(Effect::Playback(s.playing, s.position_ms));
        fx.push(Effect::Save(s.id.clone(), s.position_ms, s.playing));
        fx.push(Effect::Emit(s));
        self.last_push = Instant::now();
        self.last_save = Instant::now();
    }

    fn cancel_fade(&mut self) {
        if self.fade.take().is_some() {
            self.decks[1 - self.active] = None;
        }
        if let Some(d) = self.deck() {
            d.fade.set_property("volume", 1.0f64);
        }
    }

    fn save_gains(&mut self, app: &AppHandle) {
        if !self.gains_dirty {
            return;
        }
        self.gains_dirty = false;
        if let Ok(bytes) = serde_json::to_vec(&self.gains) {
            let _ = std::fs::write(gains_file(app), bytes);
        }
    }

    /// Lance le morceau `index` de la file. Les vidéos muettes sont sautées.
    fn load(&mut self, app: &AppHandle, mut index: usize, autoplay: bool, start_ms: Option<u64>, fx: &mut Vec<Effect>) {
        self.cancel_fade();
        self.save_gains(app);
        let mut skipped = 0;
        loop {
            let Some(id) = self.plan.ids.get(index).cloned() else {
                return self.stop(fx);
            };
            self.plan.index = index;
            let track = track_of(app, &id);
            let opened = match &track {
                Some(t) if t.has_audio => Deck::open(Path::new(&t.path), &id),
                Some(t) => Err(format!("« {} » n'a pas de piste son (vidéo seule).", t.title)),
                None => Err("Ce morceau n'est plus dans la bibliothèque.".into()),
            };
            match opened {
                Ok(mut deck) => {
                    self.decks = [None, None];
                    self.active = 0;
                    if let Some(ms) = start_ms.filter(|ms| *ms > 0) {
                        deck.pending_seek = Some(ms);
                    }
                    self.decks[0] = Some(deck);
                    self.begin_level(Some(&id));
                    if let Some(d) = self.deck() {
                        d.norm.set_property("volume", self.norm_now);
                    }
                    self.apply_sound();
                    self.playing = autoplay;
                    let state = if autoplay { gst::State::Playing } else { gst::State::Paused };
                    if let Some(d) = self.deck() {
                        let _ = d.pipe.set_state(state);
                    }
                    if let Some(t) = track {
                        fx.push(Effect::Started(t));
                    }
                    self.report(fx, true);
                    return;
                }
                Err(msg) => {
                    fx.push(Effect::Error(msg));
                    skipped += 1;
                    let n = self.plan.ids.len();
                    let next = if skipped < n { self.pick_next(false) } else { None };
                    match next {
                        Some(i) if i != index => index = i,
                        _ => return self.stop(fx),
                    }
                }
            }
        }
    }

    fn stop(&mut self, fx: &mut Vec<Effect>) {
        self.playing = false;
        if let Some(d) = self.deck() {
            let _ = d.pipe.set_state(gst::State::Paused);
        }
        self.report(fx, false);
    }

    fn set_playing(&mut self, app: &AppHandle, playing: bool, fx: &mut Vec<Effect>) {
        if self.deck().is_none() {
            if playing && !self.plan.ids.is_empty() {
                let i = self.plan.index;
                self.load(app, i, true, None, fx);
            }
            return;
        }
        if !playing {
            self.cancel_fade();
        }
        self.playing = playing;
        let state = if playing { gst::State::Playing } else { gst::State::Paused };
        if let Some(d) = self.deck() {
            let _ = d.pipe.set_state(state);
        }
        self.report(fx, false);
    }

    fn seek(&mut self, ms: u64, fx: &mut Vec<Effect>) {
        self.cancel_fade();
        if let Some(d) = self.deck_mut() {
            let ms = if d.duration_ms > 0 { ms.min(d.duration_ms.saturating_sub(500)) } else { ms };
            d.seek(ms);
        }
        self.last_second = u64::MAX;
        self.report(fx, false);
    }

    fn position(&self) -> u64 {
        self.deck().map(|d| d.position_ms()).unwrap_or(0)
    }

    /// Suivant (`delta` > 0) ou précédent, à la demande.
    fn step(&mut self, app: &AppHandle, delta: i32, fx: &mut Vec<Effect>) {
        if delta > 0 {
            let next = self.pick_next(false);
            match next {
                None => self.set_playing(app, false, fx),
                Some(i) if i == self.plan.index => {
                    self.seek(0, fx);
                    self.set_playing(app, true, fx);
                }
                Some(i) => {
                    if self.plan.shuffle {
                        self.plan.history.push(self.plan.index);
                    }
                    self.load(app, i, true, None, fx);
                }
            }
        } else {
            // Au milieu d'un titre, « précédent » revient d'abord au début.
            if self.position() > 3000 {
                return self.seek(0, fx);
            }
            if let Some(i) = self.plan.prev() {
                self.load(app, i, true, None, fx);
            }
        }
    }

    /// Fin du morceau : le suivant selon l'aléatoire et la répétition.
    fn ended(&mut self, app: &AppHandle, fx: &mut Vec<Effect>) {
        let next = self.pick_next(true);
        match next {
            None => {
                self.stop(fx);
                self.seek(0, fx);
            }
            Some(i) if i == self.plan.index => {
                self.seek(0, fx);
                self.set_playing(app, true, fx);
            }
            Some(i) => {
                if self.plan.shuffle {
                    self.plan.history.push(self.plan.index);
                }
                self.load(app, i, true, None, fx);
            }
        }
    }

    /// Fondu enchaîné : quelques secondes avant la fin, le suivant démarre sur
    /// l'autre lecteur pendant que celui-ci s'éteint.
    fn maybe_crossfade(&mut self, app: &AppHandle, fx: &mut Vec<Effect>) {
        let seconds = self.crossfade_s as u64;
        if seconds == 0 || self.fade.is_some() || !self.playing || self.plan.repeat == "one" {
            return;
        }
        let Some(d) = self.deck() else { return };
        let (pos, dur) = (d.position_ms(), d.duration_ms);
        if dur < seconds * 3000 || dur.saturating_sub(pos) > seconds * 1000 {
            return;
        }
        let next = self.pick_next(true);
        let Some(i) = next.filter(|i| *i != self.plan.index) else { return };
        let Some(id) = self.plan.ids.get(i).cloned() else { return };
        let Some(track) = track_of(app, &id).filter(|t| t.has_audio) else { return };
        let Ok(deck) = Deck::open(Path::new(&track.path), &id) else { return };
        if self.plan.shuffle {
            self.plan.history.push(self.plan.index);
        }
        self.save_gains(app);
        self.plan.index = i;
        let incoming = 1 - self.active;
        deck.fade.set_property("volume", 0.0f64);
        self.decks[incoming] = Some(deck);
        self.active = incoming;
        self.begin_level(Some(&id));
        if let Some(d) = self.deck() {
            d.norm.set_property("volume", self.norm_now);
        }
        self.apply_sound();
        if let Some(d) = self.deck() {
            let _ = d.pipe.set_state(gst::State::Playing);
        }
        self.fade = Some((Instant::now(), (seconds * 1000).max(500)));
        fx.push(Effect::Started(track));
        self.report(fx, true);
    }

    /// Volume égal : la mesure du niveau ajuste doucement la correction.
    fn on_level(&mut self, s: &gst::StructureRef) {
        if !self.loudness || !self.playing || self.fade.is_some() {
            return;
        }
        let Ok(rms) = s.get::<gst::glib::ValueArray>("rms") else { return };
        let values: Vec<f64> = rms.iter().filter_map(|v| v.get::<f64>().ok()).map(|db| 10f64.powf(db / 20.0)).collect();
        if values.is_empty() {
            return;
        }
        let rms = values.iter().sum::<f64>() / values.len() as f64;
        if rms < 0.004 {
            return; // silence : on ne corrige pas sur du vide
        }
        self.samples += 1;
        let weight = (1.0 / self.samples as f64).max(0.03);
        self.measured = if self.measured > 0.0 { self.measured * (1.0 - weight) + rms * weight } else { rms };
        // La mesure est prise avant la correction : on vise directement le bon gain.
        let wanted = (TARGET_RMS / self.measured).clamp(MIN_GAIN, MAX_GAIN);
        self.norm_target = wanted;
        self.norm_tau = if self.samples < 12 { 0.4 } else { 1.2 };
        if let Some(id) = self.deck().map(|d| d.id.clone()) {
            self.gains.insert(id, wanted);
            self.gains_dirty = true;
        }
    }

    /// Un battement (toutes les 50 ms) : messages des lecteurs, fondu, volume égal,
    /// fondu enchaîné, et nouvelles de la position pour l'écran et le système.
    fn tick(&mut self, app: &AppHandle, fx: &mut Vec<Effect>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;

        // Messages du lecteur actif.
        let mut ended = false;
        let mut failed: Option<String> = None;
        let mut ready = false;
        let mut levels = Vec::new();
        if let Some(d) = self.deck() {
            if let Some(bus) = d.pipe.bus() {
                while let Some(msg) = bus.pop() {
                    match msg.view() {
                        gst::MessageView::Eos(_) => ended = true,
                        gst::MessageView::Error(e) => failed = Some(e.error().to_string()),
                        gst::MessageView::AsyncDone(_) | gst::MessageView::DurationChanged(_) => ready = true,
                        gst::MessageView::Element(e) => {
                            if let Some(s) = e.structure().filter(|s| s.name() == "level") {
                                levels.push(s.to_owned());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Celui qui s'éteint dans un fondu : ses messages sont jetés.
        if let Some(d) = self.decks[1 - self.active].as_ref() {
            if let Some(bus) = d.pipe.bus() {
                while bus.pop().is_some() {}
            }
        }
        for s in &levels {
            self.on_level(s);
        }
        if ready {
            let mut learned = false;
            if let Some(d) = self.deck_mut() {
                if let Some(ms) = d.pending_seek {
                    d.seek(ms);
                }
                learned = d.learn_duration();
            }
            if learned {
                self.report(fx, false);
            }
        }
        if let Some(err) = failed {
            if std::env::var_os("ECHO_DEBUG").is_some() {
                eprintln!("lecture : {err}");
            }
            self.errors += 1;
            if self.errors >= 3 {
                self.errors = 0;
                fx.push(Effect::Error("Plusieurs fichiers de suite ne se lisent pas : lecture arrêtée.".into()));
                return self.stop(fx);
            }
            fx.push(Effect::Error("Ce fichier n'a pas pu être lu.".into()));
            if self.next_on_error {
                return self.ended(app, fx);
            }
            return self.stop(fx);
        }
        if ended {
            self.errors = 0;
            return self.ended(app, fx);
        }

        // Fondu en cours.
        if let Some((start, ms)) = self.fade {
            let k = (now.duration_since(start).as_millis() as f64 / ms as f64).min(1.0);
            if let Some(d) = self.deck() {
                d.fade.set_property("volume", k);
            }
            if let Some(d) = self.decks[1 - self.active].as_ref() {
                d.fade.set_property("volume", 1.0 - k);
            }
            if k >= 1.0 {
                self.fade = None;
                self.decks[1 - self.active] = None;
            }
        }

        // Volume égal : la correction rejoint doucement sa cible.
        if (self.norm_now - self.norm_target).abs() > 1e-3 {
            self.norm_now += (self.norm_target - self.norm_now) * (1.0 - (-dt / self.norm_tau).exp());
            if let Some(d) = self.deck() {
                d.norm.set_property("volume", self.norm_now);
            }
        }

        if !self.playing || self.deck().is_none() {
            return;
        }
        self.maybe_crossfade(app, fx);

        // L'écran ne change qu'à chaque seconde ; le système et la reprise, plus rarement.
        let pos = self.position();
        let second = pos / 1000;
        if second != self.last_second {
            self.last_second = second;
            fx.push(Effect::Emit(self.status(false)));
        }
        if self.last_push.elapsed() > Duration::from_secs(5) {
            self.last_push = Instant::now();
            fx.push(Effect::Playback(true, pos));
        }
        if self.last_save.elapsed() > Duration::from_secs(15) {
            self.last_save = Instant::now();
            fx.push(Effect::Save(self.deck().map(|d| d.id.clone()), pos, true));
            self.save_gains(app);
        }
    }
}

/// Tirage pseudo-aléatoire (générateur congruentiel) : assez pour choisir un morceau.
fn lcg(seed: &mut u64, n: usize) -> usize {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) % n.max(1) as u64) as usize
}

/* ---------------- autour du moteur ---------------- */

/// Vrai si le lecteur du cœur est en service (sinon, la page lit elle-même).
pub fn available(app: &AppHandle) -> bool {
    app.try_state::<Player>().map(|p| p.0.lock().unwrap().is_some()).unwrap_or(false)
}

/// Exécute une action sur le moteur, puis ses effets une fois le verrou rendu.
fn with_engine(app: &AppHandle, f: impl FnOnce(&mut Engine, &mut Vec<Effect>)) -> bool {
    let Some(player) = app.try_state::<Player>() else { return false };
    let mut fx = Vec::new();
    {
        let mut guard = player.0.lock().unwrap();
        let Some(engine) = guard.as_mut() else { return false };
        f(engine, &mut fx);
    }
    run_effects(app, fx);
    true
}

fn run_effects(app: &AppHandle, fx: Vec<Effect>) {
    for effect in fx {
        match effect {
            Effect::Started(track) => crate::announce_track(app, &track),
            Effect::Playback(playing, pos) => crate::playback_changed(app, playing, pos),
            Effect::Save(id, pos, playing) => crate::save_progress_impl(app, id, pos, playing),
            Effect::Emit(status) => {
                let _ = app.emit("player", status);
            }
            Effect::Error(msg) => {
                let _ = app.emit("player-error", msg);
            }
        }
    }
}

/// Démarre le battement du lecteur (s'il est en service).
pub fn start(app: &AppHandle) {
    if !available(app) {
        return;
    }
    let settings = app.state::<crate::AppSettings>().0.lock().unwrap().clone();
    with_engine(app, |e, _| e.configure(&settings));
    let app = app.clone();
    let _ = std::thread::Builder::new().name("lecteur".into()).spawn(move || loop {
        let mut busy = false;
        with_engine(&app, |e, fx| {
            busy = e.decks.iter().any(|d| d.is_some());
            if busy {
                e.tick(&app, fx);
            }
        });
        std::thread::sleep(Duration::from_millis(if busy { 50 } else { 250 }));
    });
}

/// Réglages du son modifiés.
pub fn configure(app: &AppHandle, settings: &crate::store::Settings) {
    with_engine(app, |e, _| e.configure(settings));
}

/// Touches média, icône de la barre et widget du bureau. Faux : action inconnue ici.
pub fn media_control(app: &AppHandle, action: &str) -> bool {
    if !available(app) {
        return false;
    }
    let (name, value) = action.split_once(':').unwrap_or((action, ""));
    let ms: i64 = value.parse().unwrap_or(0);
    let app2 = app.clone();
    match name {
        "play" => with_engine(app, |e, fx| e.set_playing(&app2, true, fx)),
        "pause" => with_engine(app, |e, fx| e.set_playing(&app2, false, fx)),
        "toggle" => with_engine(app, |e, fx| {
            let on = !e.playing;
            e.set_playing(&app2, on, fx)
        }),
        "stop" => with_engine(app, |e, fx| {
            e.set_playing(&app2, false, fx);
            e.seek(0, fx);
        }),
        "next" => with_engine(app, |e, fx| e.step(&app2, 1, fx)),
        "previous" => with_engine(app, |e, fx| e.step(&app2, -1, fx)),
        "seek-forward" | "seek-backward" | "seek-by" => with_engine(app, |e, fx| {
            let by = match name {
                "seek-forward" => e.skip_s as i64 * 1000,
                "seek-backward" => -(e.skip_s as i64) * 1000,
                _ => ms,
            };
            let to = (e.position() as i64 + by).max(0) as u64;
            e.seek(to, fx);
        }),
        "set-position" => with_engine(app, |e, fx| e.seek(ms.max(0) as u64, fx)),
        "quit" => {
            save_now(app);
            app.exit(0);
            true
        }
        _ => false,
    }
}

/// Position du moment enregistrée (à la fermeture d'Écho).
pub fn save_now(app: &AppHandle) {
    with_engine(app, |e, fx| {
        e.save_gains(app);
        let s = e.status(false);
        if s.id.is_some() {
            fx.push(Effect::Save(s.id, s.position_ms, s.playing));
        }
    });
}

/* ---------------- commandes de la page ---------------- */

/// Nouvelle file, et lecture du morceau `index` (à `start_ms` si donné).
#[tauri::command]
pub fn player_play(app: AppHandle, ids: Vec<String>, index: usize, autoplay: bool, start_ms: Option<u64>, shuffle: bool, repeat: String) {
    let app2 = app.clone();
    with_engine(&app, |e, fx| {
        e.plan = Plan { ids, index, shuffle, repeat, ..Default::default() };
        e.load(&app2, index, autoplay, start_ms, fx);
    });
}

/// La file a changé (ajout, retrait, ordre, aléatoire, répétition) sans changer de
/// morceau : le morceau qui joue garde sa place, quelle que soit sa nouvelle position.
#[tauri::command]
pub fn player_queue(app: AppHandle, ids: Vec<String>, index: usize, shuffle: bool, repeat: String) {
    with_engine(&app, |e, _| {
        let playing_id = e.deck().map(|d| d.id.clone());
        let same = e.plan.ids == ids;
        let index = playing_id
            .and_then(|id| ids.iter().position(|x| *x == id))
            .unwrap_or_else(|| index.min(ids.len().saturating_sub(1)));
        if !same || e.plan.shuffle != shuffle {
            e.plan.played.clear();
            e.plan.history.clear();
        }
        e.plan.ids = ids;
        e.plan.index = index;
        e.plan.shuffle = shuffle;
        e.plan.repeat = repeat;
    });
}

#[tauri::command]
pub fn player_set_playing(app: AppHandle, playing: bool) {
    let app2 = app.clone();
    with_engine(&app, |e, fx| e.set_playing(&app2, playing, fx));
}

#[tauri::command]
pub fn player_step(app: AppHandle, delta: i32) {
    let app2 = app.clone();
    with_engine(&app, |e, fx| e.step(&app2, delta, fx));
}

#[tauri::command]
pub fn player_seek(app: AppHandle, ms: u64) {
    with_engine(&app, |e, fx| e.seek(ms, fx));
}

#[tauri::command]
pub fn player_volume(app: AppHandle, volume: f64, muted: bool) {
    with_engine(&app, |e, _| {
        e.volume = volume.clamp(0.0, 1.0);
        e.muted = muted;
        e.apply_sound();
    });
}

/// État complet, pour une page qui (re)vient : la file, le morceau, la position.
#[tauri::command]
pub fn player_snapshot(app: AppHandle) -> Option<Snapshot> {
    let player = app.try_state::<Player>()?;
    let guard = player.0.lock().unwrap();
    let e = guard.as_ref()?;
    Some(Snapshot {
        status: e.status(false),
        ids: e.plan.ids.clone(),
        shuffle: e.plan.shuffle,
        repeat: e.plan.repeat.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(n: usize, index: usize, shuffle: bool, repeat: &str) -> Plan {
        Plan {
            ids: (0..n).map(|i| i.to_string()).collect(),
            index,
            shuffle,
            repeat: repeat.into(),
            ..Default::default()
        }
    }

    #[test]
    fn la_file_suit_les_regles_de_l_interface() {
        let mut first = |_: usize| 0;
        assert_eq!(plan(3, 1, false, "off").next(true, &mut first), Some(2));
        assert_eq!(plan(3, 2, false, "off").next(true, &mut first), None);
        assert_eq!(plan(3, 2, false, "off").next(false, &mut first), Some(0));
        assert_eq!(plan(3, 2, false, "all").next(true, &mut first), Some(0));
        assert_eq!(plan(3, 1, false, "one").next(true, &mut first), Some(1));
        assert_eq!(plan(3, 0, false, "off").prev(), Some(2));
    }

    #[test]
    fn l_aleatoire_passe_chaque_morceau_une_fois() {
        let mut p = plan(4, 0, true, "off");
        let mut seen = vec![0];
        let mut k = 0;
        let mut rand = |n: usize| {
            k += 1;
            k % n
        };
        while let Some(i) = p.next(true, &mut rand) {
            p.history.push(p.index);
            p.index = i;
            seen.push(i);
        }
        seen.sort();
        assert_eq!(seen, vec![0, 1, 2, 3]);
        // Précédent : retour au morceau joué juste avant.
        let before = *p.history.last().unwrap();
        assert_eq!(p.prev(), Some(before));
    }
}
