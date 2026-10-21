//! Lecteur audio d'Écho sous Linux, dans le cœur de l'app (GStreamer).
//!
//! La page web n'est plus qu'un écran posé dessus : elle envoie des ordres (lire,
//! suivant, volume…) et reçoit l'état du lecteur. La musique ne dépend donc plus
//! d'elle : rangé dans la barre du système, Écho ferme sa page (≈ 200 Mo de moins)
//! et continue de jouer, de passer au morceau suivant et de répondre aux touches média.
//!
//! Les ordres passent par un canal jusqu'au fil « lecteur », qui les traite dans
//! l'ordre : ouvrir un fichier ou écrire sur le disque ne bloque jamais la fenêtre.
//!
//! Deux lecteurs servent au fondu enchaîné. Chacun passe par la même chaîne :
//! égaliseur 5 bandes → mesure du niveau → volume égal → fondu → limiteur.

use gstreamer as gst;
use gstreamer::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
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
/// Corrections de volume gardées dans gains.json (les plus anciennes sont oubliées).
const MAX_GAINS: usize = 3000;

/// Battement : rapide quand il se passe quelque chose, lent au repos.
const FAST: Duration = Duration::from_millis(50);
const SLOW: Duration = Duration::from_millis(250);
/// Après un saut, GStreamer donne encore un moment l'ancienne position.
const SEEK_HOLD: Duration = Duration::from_millis(300);
/// Un saut demandé avant l'ouverture qui n'a toujours pas pu se faire est abandonné.
const SEEK_GIVE_UP: Duration = Duration::from_secs(8);

/// Un visualiseur est à l'écran : le spectre est calculé et envoyé à la page.
static VISUAL: AtomicBool = AtomicBool::new(false);
/// Bandes envoyées à la page, de 40 Hz à 16 kHz (échelle logarithmique, comme l'oreille).
const VISUAL_BANDS: usize = 32;
/// Plancher du spectre, en dB : en dessous, une bande est éteinte.
const VISUAL_FLOOR: f32 = -75.0;

/// Numéro des statuts : la page ignore ceux qui arrivent après un plus récent.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed) + 1
}

/* ---------------- un lecteur ---------------- */

struct Deck {
    pipe: gst::Pipeline,
    eq: gst::Element,
    meter: gst::Element,
    spec: gst::Element,
    master: gst::Element,
    norm: gst::Element,
    fade: gst::Element,
    limiter: gst::Element,
    id: String,
    track: Track,
    /// Durée donnée par GStreamer (0 : pas encore connue).
    duration_ms: u64,
    /// Position à rejoindre dès que le fichier est prêt (reprise, saut demandé tôt).
    pending_seek: Option<u64>,
    /// Position demandée par le dernier saut, renvoyée le temps qu'il se fasse.
    seek_hold: Option<(u64, Instant)>,
    /// Lecture demandée pendant que le saut initial attend : elle part après lui.
    resume: bool,
    /// Le morceau a été annoncé (notification, touches média).
    announced: bool,
    /// Fichier en erreur : il ne joue plus, « lecture » le rouvre.
    broken: bool,
    opened: Instant,
}

impl Drop for Deck {
    fn drop(&mut self) {
        let _ = self.pipe.set_state(gst::State::Null);
    }
}

impl Deck {
    fn open(track: &Track) -> Result<Deck, String> {
        let uri = gst::glib::filename_to_uri(Path::new(&track.path), None).map_err(|e| e.to_string())?;
        // Le son seul : les autres pistes d'un clip (image, sous-titres) ne sont pas
        // décodées du tout. (playbin, lui, décode l'image même sans l'afficher.)
        let source = gst::ElementFactory::make("uridecodebin")
            .property("uri", uri.as_str())
            .property("caps", gst::Caps::new_empty_simple("audio/x-raw"))
            .property("expose-all-streams", false)
            .build()
            .map_err(|e| e.to_string())?;
        // Calcul en flottant jusqu'au limiteur : l'égaliseur et le volume égal peuvent
        // dépasser 0 dB sans écrêter, le limiteur ramène ensuite le tout.
        // Sortie : PulseAudio / PipeWire quand ils sont là, avec une identité propre.
        // La vidéo muette des clips ouvre aussi un flux (WebKit) sous le nom
        // « echo-player » : le serveur de son retient « coupé » par nom d'application et
        // l'appliquait à la musique. Avec sa propre clé de mémorisation, la musique n'en
        // dépend plus ; et elle part toujours non coupée.
        let sink = if gst::ElementFactory::find("pulsesink").is_some() { "pulsesink name=out" } else { "autoaudiosink name=out" };
        let chain = gst::parse::bin_from_description(
            &format!(
                "audioconvert ! audioresample ! audio/x-raw,format=F32LE \
                 ! equalizer-nbands name=eq num-bands=5 \
                 ! level name=meter interval=250000000 post-messages=false \
                 ! spectrum name=spec bands=512 threshold=-75 interval=50000000 post-messages=false \
                 ! volume name=norm ! volume name=fade ! volume name=master \
                 ! audiodynamic name=limiter characteristics=hard-knee mode=compressor threshold=0.84 \
                 ! audioconvert ! {sink}"
            ),
            true,
        )
        .map_err(|e| e.to_string())?;
        if let Some(out) = chain.by_name("out").filter(|o| o.find_property("stream-properties").is_some()) {
            let props = gst::Structure::builder("props")
                .field("application.name", "Écho")
                .field("application.id", "com.echo.player")
                .field("application.icon_name", "echo-player")
                .field("module-stream-restore.id", "echo-player-musique")
                .build();
            out.set_property("stream-properties", props);
            out.set_property("client-name", "Écho");
            out.set_property("mute", false);
        }
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
            meter: get("meter")?,
            spec: get("spec")?,
            master: get("master")?,
            norm: get("norm")?,
            fade: get("fade")?,
            limiter: get("limiter")?,
            pipe,
            id: track.id.clone(),
            track: track.clone(),
            duration_ms: 0,
            pending_seek: None,
            seek_hold: None,
            resume: false,
            announced: false,
            broken: false,
            opened: Instant::now(),
        };
        deck.spec.set_property("post-messages", VISUAL.load(Ordering::Relaxed));
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

    /// Durée apprise par GStreamer, sinon celle des tags.
    fn duration(&self) -> u64 {
        if self.duration_ms > 0 {
            self.duration_ms
        } else {
            self.track.duration_ms
        }
    }

    fn position_ms(&self) -> u64 {
        if let Some(ms) = self.pending_seek {
            return ms;
        }
        if let Some((ms, at)) = self.seek_hold {
            if at.elapsed() < SEEK_HOLD {
                return ms;
            }
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
        self.seek_hold = ok.then(|| (ms, Instant::now()));
    }

    /// Lecture ou pause. Tant qu'un saut initial attend, le lecteur reste en pause :
    /// le début du morceau ne s'entend pas avant d'arriver à la bonne position.
    fn play(&mut self, on: bool) {
        self.resume = on && self.pending_seek.is_some();
        let state = if on && !self.resume { gst::State::Playing } else { gst::State::Paused };
        let _ = self.pipe.set_state(state);
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

    /// Jette les messages en attente (lecteur qui s'éteint, fichier en erreur).
    fn drain(&self) {
        if let Some(bus) = self.pipe.bus() {
            while bus.pop().is_some() {}
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

    /// Le morceau qui remplace un morceau illisible : les règles de fin de morceau,
    /// mais « répéter un titre » vaut « tout » (rejouer le même ne servirait à rien).
    fn next_after_failure(&mut self, rand: &mut impl FnMut(usize) -> usize) -> Option<usize> {
        let repeat = std::mem::take(&mut self.repeat);
        self.repeat = if repeat == "one" { "all".into() } else { repeat.clone() };
        let next = self.next(true, rand);
        self.repeat = repeat;
        next
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

    /// Nouvelle file demandée par la page. La même file avec le même aléatoire garde
    /// sa mémoire (morceaux déjà joués, historique) : seul le morceau change.
    fn adopt(&mut self, ids: Vec<String>, index: usize, shuffle: bool, repeat: String) {
        if !ids.is_empty() && self.ids == ids && self.shuffle == shuffle {
            let old = self.index;
            if shuffle && old != index {
                self.history.push(old);
                self.played.insert(old);
            }
            self.index = index;
            self.repeat = repeat;
        } else {
            *self = Plan { ids, index, shuffle, repeat, ..Default::default() };
        }
    }
}

/// Place du morceau qui joue dans une file modifiée : l'index donné s'il désigne
/// bien ce morceau (il peut y être deux fois), sinon sa première position.
fn place_in(ids: &[String], index: usize, playing: Option<&str>) -> usize {
    let fallback = index.min(ids.len().saturating_sub(1));
    match playing {
        Some(id) if ids.get(index).map(String::as_str) == Some(id) => index,
        Some(id) => ids.iter().position(|x| x == id).unwrap_or(fallback),
        None => fallback,
    }
}

/* ---------------- ce que la page reçoit ---------------- */

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    /// Croît à chaque statut : la page ignore un statut plus vieux que le dernier vu.
    pub seq: u64,
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
    volume: f64,
    muted: bool,
}

/// Ce qu'un changement d'état entraîne hors du lecteur (fait sans tenir son verrou).
enum Effect {
    Started(Track),
    Playback(bool, u64),
    /// Progression à garder ; `force` : écrite tout de suite (pause, sortie).
    Save { id: String, position_ms: u64, playing: bool, force: bool },
    Gains(HashMap<String, f64>),
    Emit(Status),
    /// Spectre du son (visualiseur), de 0 à 255 par bande.
    Spectrum(Vec<u8>),
    Error(String),
}

/// Ordres de la page, des touches média et de la barre du système.
enum Order {
    Play(Request),
    Queue { ids: Vec<String>, index: usize, shuffle: bool, repeat: String },
    SetPlaying(bool),
    Toggle,
    /// Arrêt : pause et retour au début.
    Stop,
    Step(i32),
    Seek(u64),
    SeekBy(i64),
    /// Saut d'un pas réglé (« avancer de 10 s »), vers l'avant ou l'arrière.
    Skip(i64),
    Volume(f64, bool),
    Configure(Box<crate::store::Settings>),
    /// Visualiseur affiché ou non.
    Visual(bool),
}

struct Request {
    ids: Vec<String>,
    index: usize,
    autoplay: bool,
    start_ms: Option<u64>,
    shuffle: bool,
    repeat: String,
}

/// D'où vient le changement de morceau.
#[derive(Clone, Copy, PartialEq)]
enum Way {
    /// Demandé (lecture, suivant) : un fondu en cours est coupé.
    User,
    /// Fin de morceau ou erreur : un fondu en cours se termine normalement.
    Auto,
    /// Précédent : un morceau illisible fait reculer encore.
    Back,
}

/* ---------------- le moteur ---------------- */

pub struct Engine {
    /// Le lecteur qui joue, et celui qui s'éteint pendant un fondu enchaîné.
    deck: Option<Deck>,
    outgoing: Option<Deck>,
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
    /// Le lecteur actif monte pendant le fondu (faux s'il a été remplacé entre-temps).
    fade_in: bool,
    /// Fondu déjà tenté pour ce morceau (le suivant ne s'ouvrait pas).
    crossfade_tried: bool,
    norm_target: f64,
    norm_now: f64,
    norm_tau: f64,
    measured: f64,
    samples: u32,
    gains: HashMap<String, f64>,
    gains_order: VecDeque<String>,
    gains_dirty: bool,

    last_tick: Instant,
    last_second: u64,
    last_push: Instant,
    last_save: Instant,
    errors: u32,
}

pub struct Player {
    engine: Mutex<Option<Engine>>,
    /// Vers le fil « lecteur » (absent : la page lit elle-même la musique).
    orders: Option<Sender<Order>>,
    /// Pris par le fil « lecteur » à son démarrage.
    inbox: Mutex<Option<Receiver<Order>>>,
}

impl Player {
    /// Le lecteur, si GStreamer et ses greffons de base sont là ; sinon la page lit
    /// elle-même la musique, comme avant.
    pub fn new(app: &AppHandle) -> Self {
        let ready = gst::init().is_ok()
            // Exactement les éléments de la chaîne d'un lecteur (Deck::open) : s'il en
            // manque un, la page lit elle-même la musique au lieu d'échouer à chaque morceau.
            && ["uridecodebin", "audioconvert", "audioresample", "equalizer-nbands", "level", "spectrum", "volume", "audiodynamic", "autoaudiosink"]
                .iter()
                .all(|f| gst::ElementFactory::find(f).is_some());
        if !ready || std::env::var_os("ECHO_WEB_AUDIO").is_some() {
            return Self { engine: Mutex::new(None), orders: None, inbox: Mutex::new(None) };
        }
        let mut gains: HashMap<String, f64> = std::fs::read(gains_file(app))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut gains_order: VecDeque<String> = gains.keys().cloned().collect();
        while gains.len() > MAX_GAINS {
            match gains_order.pop_front() {
                Some(old) => {
                    gains.remove(&old);
                }
                None => break,
            }
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(7)
            | 1;
        let now = Instant::now();
        let (tx, rx) = mpsc::channel();
        Self {
            engine: Mutex::new(Some(Engine {
                deck: None,
                outgoing: None,
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
                fade_in: false,
                crossfade_tried: false,
                norm_target: 1.0,
                norm_now: 1.0,
                norm_tau: 0.4,
                measured: 0.0,
                samples: 0,
                gains,
                gains_order,
                gains_dirty: false,
                last_tick: now,
                last_second: u64::MAX,
                last_push: now,
                last_save: now,
                errors: 0,
            })),
            orders: Some(tx),
            inbox: Mutex::new(Some(rx)),
        }
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

    fn pick_after_failure(&mut self) -> Option<usize> {
        let seed = &mut self.seed;
        self.plan.next_after_failure(&mut |n| lcg(seed, n))
    }

    fn handle(&mut self, app: &AppHandle, order: Order, fx: &mut Vec<Effect>) {
        match order {
            Order::Play(r) => {
                self.plan.adopt(r.ids, r.index, r.shuffle, r.repeat);
                let index = self.plan.index;
                self.load(app, index, r.autoplay, r.start_ms, Way::User, fx);
            }
            Order::Queue { ids, index, shuffle, repeat } => {
                let index = place_in(&ids, index, self.deck.as_ref().map(|d| d.id.as_str()));
                if self.plan.ids != ids || self.plan.shuffle != shuffle {
                    self.plan.played.clear();
                    self.plan.history.clear();
                }
                self.plan.ids = ids;
                self.plan.index = index;
                self.plan.shuffle = shuffle;
                self.plan.repeat = repeat;
            }
            Order::SetPlaying(on) => self.set_playing(app, on, fx),
            Order::Toggle => {
                let on = !self.playing;
                self.set_playing(app, on, fx);
            }
            Order::Stop => self.rewind(fx),
            Order::Step(delta) => self.step(app, delta, fx),
            Order::Seek(ms) => self.seek(ms, fx),
            Order::SeekBy(by) => {
                let to = (self.position() as i64 + by).max(0) as u64;
                self.seek(to, fx);
            }
            Order::Skip(steps) => {
                let to = (self.position() as i64 + steps * self.skip_s as i64 * 1000).max(0) as u64;
                self.seek(to, fx);
            }
            Order::Volume(volume, muted) => {
                self.volume = volume.clamp(0.0, 1.0);
                self.muted = muted;
                self.apply_sound();
            }
            Order::Configure(settings) => self.configure(&settings),
            Order::Visual(on) => {
                VISUAL.store(on, Ordering::Relaxed);
                for d in self.deck.iter().chain(self.outgoing.iter()) {
                    d.spec.set_property("post-messages", on);
                }
            }
        }
    }

    fn configure(&mut self, s: &crate::store::Settings) {
        self.volume = (s.volume as f64 / 100.0).clamp(0.0, 1.0);
        self.eq = eq_preset(&s.eq);
        let was = self.loudness;
        self.loudness = s.loudness;
        self.crossfade_s = s.crossfade;
        self.next_on_error = s.next_on_error;
        self.skip_s = s.skip_seconds.max(1);
        if was != self.loudness {
            let id = self.deck.as_ref().map(|d| d.id.clone());
            self.begin_level(id.as_deref());
            // Appliqué tout de suite : le battement ne rattrape que les petits écarts.
            if let Some(d) = &self.deck {
                set_volume(&d.norm, self.norm_now);
            }
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
        for deck in [&self.deck, &self.outgoing].into_iter().flatten() {
            deck.set_eq(&self.eq, limit);
            set_volume(&deck.master, self.volume);
            deck.master.set_property("mute", self.muted);
            // La mesure du niveau ne sert qu'au volume égal.
            deck.meter.set_property("post-messages", self.loudness);
        }
    }

    /// Nouveau morceau : on reprend la correction de volume déjà apprise pour lui.
    fn begin_level(&mut self, id: Option<&str>) {
        self.measured = 0.0;
        self.samples = 0;
        let known = id.and_then(|id| self.gains.get(id).copied()).filter(|g| g.is_finite() && (MIN_GAIN..=MAX_GAIN).contains(g));
        self.norm_target = if self.loudness { known.unwrap_or(1.0) } else { self.eq_compensation() };
        self.norm_now = self.norm_target;
        self.norm_tau = 0.4;
        if let (true, Some(g)) = (self.loudness, known) {
            self.measured = TARGET_RMS / g;
            self.samples = 12;
        }
    }

    /// `pos` : position déjà demandée à GStreamer pendant ce battement.
    fn status_at(&self, changed: bool, pos: Option<u64>) -> Status {
        let deck = self.deck.as_ref();
        Status {
            seq: next_seq(),
            id: deck.map(|d| d.id.clone()),
            index: self.plan.index,
            playing: self.playing && deck.is_some(),
            position_ms: pos.or_else(|| deck.map(|d| d.position_ms())).unwrap_or(0),
            duration_ms: deck.map(|d| d.duration()).unwrap_or(0),
            changed,
        }
    }

    fn status(&self, changed: bool) -> Status {
        self.status_at(changed, None)
    }

    fn report(&mut self, fx: &mut Vec<Effect>, changed: bool) {
        self.report_with(fx, changed, false, None);
    }

    /// Statut pour la page, le système et la reprise. `force` : la progression est
    /// écrite tout de suite (pause), sans attendre le regroupement des écritures.
    fn report_with(&mut self, fx: &mut Vec<Effect>, changed: bool, force: bool, pos: Option<u64>) {
        let s = self.status_at(changed, pos);
        fx.push(Effect::Playback(s.playing, s.position_ms));
        // Sans morceau, rien à garder : le dernier morceau connu reste celui de la reprise.
        if let Some(id) = &s.id {
            fx.push(Effect::Save { id: id.clone(), position_ms: s.position_ms, playing: s.playing, force });
        }
        fx.push(Effect::Emit(s));
        self.last_push = Instant::now();
        self.last_save = Instant::now();
    }

    fn cancel_fade(&mut self) {
        self.fade = None;
        self.fade_in = false;
        self.outgoing = None;
        if let Some(d) = &self.deck {
            set_volume(&d.fade, 1.0f64);
        }
    }

    fn remember_gain(&mut self, id: String, gain: f64) {
        if !gain.is_finite() {
            return;
        }
        // Correction inchangée (à 0,5 % près) : rien à réécrire sur le disque.
        if self.gains.get(&id).is_some_and(|g| (g - gain).abs() <= g.abs() * 0.005) {
            return;
        }
        if self.gains.insert(id.clone(), gain).is_none() {
            self.gains_order.push_back(id);
            while self.gains.len() > MAX_GAINS {
                match self.gains_order.pop_front() {
                    Some(old) => {
                        self.gains.remove(&old);
                    }
                    None => break,
                }
            }
        }
        self.gains_dirty = true;
    }

    /// Les corrections apprises partent sur le disque, une fois le verrou rendu.
    fn flush_gains(&mut self, fx: &mut Vec<Effect>) {
        if std::mem::take(&mut self.gains_dirty) {
            fx.push(Effect::Gains(self.gains.clone()));
        }
    }

    /// Lance le morceau `index` de la file. Un morceau muet ou illisible est passé
    /// (vers l'arrière pour « précédent ») ; `start_ms` ne vaut que pour le premier.
    fn load(&mut self, app: &AppHandle, index: usize, autoplay: bool, start_ms: Option<u64>, way: Way, fx: &mut Vec<Effect>) {
        // Fin de morceau pendant un fondu : celui qui s'éteint finit son fondu.
        if way == Way::Auto && self.fade.is_some() {
            self.fade_in = false;
        } else {
            self.cancel_fade();
        }
        self.flush_gains(fx);
        self.crossfade_tried = false;
        self.last_second = u64::MAX;
        let (mut index, mut start_ms) = (index, start_ms);
        for _ in 0..self.plan.ids.len() {
            let Some(id) = self.plan.ids.get(index).cloned() else { break };
            self.plan.index = index;
            let opened = match track_of(app, &id) {
                Some(t) if t.has_audio => Deck::open(&t),
                Some(t) if is_partial_download(&t.path) => Err(format!(
                    "« {} » est sauté : téléchargement incomplet, ce fichier n'est que l'image du clip. Retélécharge-le.",
                    t.title
                )),
                Some(t) => Err(format!("« {} » n'a pas de piste son (vidéo seule).", t.title)),
                None => Err("Ce morceau n'est plus dans la bibliothèque.".into()),
            };
            match opened {
                Ok(deck) => return self.start_deck(deck, autoplay, start_ms, fx),
                Err(msg) => {
                    fx.push(Effect::Error(msg));
                    start_ms = None;
                    // Réglage « passer au suivant en cas d'erreur » coupé : un morceau choisi
                    // qui ne s'ouvre pas arrête la lecture (une vidéo sans son est toujours sautée).
                    let silent = track_of(app, &id).is_some_and(|t| !t.has_audio);
                    if !self.next_on_error && way != Way::Auto && !silent {
                        break;
                    }
                    let next = if way == Way::Back { self.plan.prev() } else { self.pick_after_failure() };
                    match next {
                        Some(i) if i != index => index = i,
                        _ => break,
                    }
                }
            }
        }
        // Rien de lisible : plus aucun lecteur, la page ne doit pas croire qu'un morceau est prêt.
        self.deck = None;
        self.cancel_fade();
        self.playing = false;
        self.report_with(fx, false, true, None);
    }

    fn start_deck(&mut self, mut deck: Deck, autoplay: bool, start_ms: Option<u64>, fx: &mut Vec<Effect>) {
        if let Some(ms) = start_ms.filter(|ms| *ms > 0) {
            deck.pending_seek = Some(ms);
        }
        let id = deck.id.clone();
        self.deck = Some(deck);
        if self.fade.is_none() {
            self.outgoing = None;
        }
        self.begin_level(Some(&id));
        if let Some(d) = &self.deck {
            set_volume(&d.norm, self.norm_now);
        }
        self.apply_sound();
        self.playing = autoplay;
        let mut started = None;
        if let Some(d) = self.deck.as_mut() {
            d.play(autoplay);
            // Chargé en pause (reprise au lancement) : annoncé quand il partira.
            if autoplay {
                d.announced = true;
                started = Some(d.track.clone());
            }
        }
        if let Some(t) = started {
            fx.push(Effect::Started(t));
        }
        self.report(fx, true);
    }

    /// Fichier en erreur : il reste affiché mais ne joue plus.
    fn halt(&mut self, fx: &mut Vec<Effect>) {
        self.cancel_fade();
        self.playing = false;
        if let Some(d) = self.deck.as_mut() {
            d.broken = true;
            d.drain();
            let _ = d.pipe.set_state(gst::State::Paused);
        }
        self.report_with(fx, false, true, None);
    }

    fn set_playing(&mut self, app: &AppHandle, playing: bool, fx: &mut Vec<Effect>) {
        let usable = self.deck.as_ref().is_some_and(|d| !d.broken);
        if !usable {
            if playing && !self.plan.ids.is_empty() {
                let i = self.plan.index.min(self.plan.ids.len() - 1);
                return self.load(app, i, true, None, Way::User, fx);
            }
            // Rien à lire : la page a peut-être déjà affiché « lecture », on la dément.
            self.playing = false;
            return self.report(fx, false);
        }
        if !playing {
            self.cancel_fade();
        }
        self.playing = playing;
        let mut started = None;
        if let Some(d) = self.deck.as_mut() {
            d.play(playing);
            if playing && !d.announced {
                d.announced = true;
                started = Some(d.track.clone());
            }
        }
        if let Some(t) = started {
            fx.push(Effect::Started(t));
        }
        self.report_with(fx, false, !playing, None);
    }

    fn seek(&mut self, ms: u64, fx: &mut Vec<Effect>) {
        self.cancel_fade();
        self.crossfade_tried = false;
        if let Some(d) = self.deck.as_mut() {
            let dur = d.duration();
            let ms = if dur > 0 { ms.min(dur.saturating_sub(500)) } else { ms };
            d.seek(ms);
        }
        self.last_second = u64::MAX;
        self.report(fx, false);
    }

    /// Arrêt : pause et retour au début, un seul statut.
    fn rewind(&mut self, fx: &mut Vec<Effect>) {
        self.cancel_fade();
        self.crossfade_tried = false;
        self.playing = false;
        if let Some(d) = self.deck.as_mut() {
            d.play(false);
            d.seek(0);
        }
        self.last_second = u64::MAX;
        self.report_with(fx, false, true, None);
    }

    /// Le même morceau reprend du début (répéter un titre, file d'un seul morceau).
    fn restart(&mut self, app: &AppHandle, fx: &mut Vec<Effect>) {
        self.cancel_fade();
        self.crossfade_tried = false;
        self.last_second = u64::MAX;
        if let Some(d) = self.deck.as_mut() {
            d.seek(0);
        }
        self.set_playing(app, true, fx);
    }

    fn position(&self) -> u64 {
        self.deck.as_ref().map(|d| d.position_ms()).unwrap_or(0)
    }

    /// Suivant (`delta` > 0) ou précédent, à la demande.
    fn step(&mut self, app: &AppHandle, delta: i32, fx: &mut Vec<Effect>) {
        if delta > 0 {
            let current = self.plan.index;
            match self.pick_next(false) {
                None => self.set_playing(app, false, fx),
                Some(i) if i == current && self.deck.is_some() => self.restart(app, fx),
                Some(i) => {
                    if self.plan.shuffle && i != current {
                        self.plan.history.push(current);
                    }
                    self.load(app, i, true, None, Way::User, fx);
                }
            }
        } else {
            // Au milieu d'un titre, « précédent » revient d'abord au début.
            if self.position() > 3000 {
                return self.seek(0, fx);
            }
            if let Some(i) = self.plan.prev() {
                self.load(app, i, true, None, Way::Back, fx);
            }
        }
    }

    /// Fin du morceau : le suivant selon l'aléatoire et la répétition.
    fn ended(&mut self, app: &AppHandle, fx: &mut Vec<Effect>) {
        let current = self.plan.index;
        match self.pick_next(true) {
            // Fin de la file : en pause, au début du dernier morceau.
            None => self.rewind(fx),
            Some(i) if i == current => self.restart(app, fx),
            Some(i) => {
                if self.plan.shuffle {
                    self.plan.history.push(current);
                }
                self.load(app, i, true, None, Way::Auto, fx);
            }
        }
    }

    /// Le fichier ne se lit pas : un autre morceau prend sa place, dans le même état
    /// (lecture ou pause). Rejouer le même ne servirait à rien.
    fn failed(&mut self, app: &AppHandle, err: String, fx: &mut Vec<Effect>) {
        if std::env::var_os("ECHO_DEBUG").is_some() {
            eprintln!("lecture : {err}");
        }
        self.errors += 1;
        if self.errors >= 3 {
            self.errors = 0;
            fx.push(Effect::Error("Plusieurs fichiers de suite ne se lisent pas : lecture arrêtée.".into()));
            return self.halt(fx);
        }
        fx.push(Effect::Error("Ce fichier n'a pas pu être lu.".into()));
        if self.next_on_error {
            let current = self.plan.index;
            if let Some(i) = self.pick_after_failure().filter(|i| *i != current) {
                let autoplay = self.playing;
                return self.load(app, i, autoplay, None, Way::Auto, fx);
            }
        }
        self.halt(fx);
    }

    /// Fondu enchaîné : quelques secondes avant la fin, le suivant démarre sur
    /// l'autre lecteur pendant que celui-ci s'éteint.
    fn maybe_crossfade(&mut self, app: &AppHandle, pos: u64, fx: &mut Vec<Effect>) {
        let seconds = self.crossfade_s as u64;
        if seconds == 0 || self.fade.is_some() || self.crossfade_tried || !self.playing || self.plan.repeat == "one" {
            return;
        }
        let Some(d) = &self.deck else { return };
        let dur = d.duration();
        if dur < seconds * 3000 || dur.saturating_sub(pos) > seconds * 1000 {
            return;
        }
        // Une seule tentative : si le suivant ne s'ouvre pas, la fin du morceau s'en charge.
        self.crossfade_tried = true;
        let current = self.plan.index;
        let Some(i) = self.pick_next(true).filter(|i| *i != current) else { return };
        let Some(id) = self.plan.ids.get(i).cloned() else { return };
        let Some(track) = track_of(app, &id).filter(|t| t.has_audio) else { return };
        let Ok(mut deck) = Deck::open(&track) else { return };
        if self.plan.shuffle {
            self.plan.history.push(current);
        }
        self.flush_gains(fx);
        self.plan.index = i;
        set_volume(&deck.fade, 0.0f64);
        deck.announced = true;
        self.outgoing = self.deck.take();
        self.deck = Some(deck);
        self.begin_level(Some(&id));
        if let Some(d) = &self.deck {
            set_volume(&d.norm, self.norm_now);
        }
        self.apply_sound();
        if let Some(d) = self.deck.as_mut() {
            d.play(true);
        }
        self.fade = Some((Instant::now(), (seconds * 1000).max(500)));
        self.fade_in = true;
        self.last_second = u64::MAX;
        fx.push(Effect::Started(track));
        self.report(fx, true);
    }

    /// Volume égal : la mesure du niveau ajuste doucement la correction.
    fn on_level(&mut self, s: &gst::StructureRef) {
        if !self.loudness || !self.playing || self.fade.is_some() {
            return;
        }
        let Ok(rms) = s.get::<gst::glib::ValueArray>("rms") else { return };
        // GStreamer donne parfois « NaN » (tampon vide après un saut) : ignoré, sinon il
        // empoisonnerait toute la correction (et la valeur gardée pour ce morceau).
        let values: Vec<f64> = rms
            .iter()
            .filter_map(|v| v.get::<f64>().ok())
            .map(|db| 10f64.powf(db / 20.0))
            .filter(|v| v.is_finite())
            .collect();
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
        if !wanted.is_finite() {
            self.measured = 0.0;
            self.samples = 0;
            return;
        }
        self.norm_target = wanted;
        self.norm_tau = if self.samples < 12 { 0.4 } else { 1.2 };
        if let Some(id) = self.deck.as_ref().map(|d| d.id.clone()) {
            self.remember_gain(id, wanted);
        }
    }

    /// Rythme du battement : rapide en lecture, pendant un fondu, un saut ou tant que
    /// la durée n'est pas connue ; lent sinon.
    fn beat(&self) -> Duration {
        let unsettled = self.deck.as_ref().is_some_and(|d| {
            // Durée encore inconnue : battement rapide quelques secondes seulement (un fichier
            // qui ne la donne jamais ne réveille pas le lecteur 20 fois par seconde en pause).
            !d.broken
                && ((d.duration_ms == 0 && d.opened.elapsed() < Duration::from_secs(5))
                    || d.pending_seek.is_some()
                    || d.seek_hold.is_some()
                    || d.resume)
        });
        if (self.playing && self.deck.is_some()) || self.fade.is_some() || unsettled {
            FAST
        } else {
            SLOW
        }
    }

    /// Un battement : messages des lecteurs, fondu, volume égal, fondu enchaîné, et
    /// nouvelles de la position pour l'écran et le système.
    fn tick(&mut self, app: &AppHandle, fx: &mut Vec<Effect>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;

        // Messages du lecteur actif.
        let mut ended = false;
        let mut failed: Option<String> = None;
        let mut async_done = false;
        let mut duration_changed = false;
        let mut levels = Vec::new();
        let mut spectrum: Option<gst::Structure> = None;
        if let Some(d) = &self.deck {
            if d.broken {
                d.drain();
            } else if let Some(bus) = d.pipe.bus() {
                while let Some(msg) = bus.pop() {
                    match msg.view() {
                        gst::MessageView::Eos(_) => ended = true,
                        gst::MessageView::Error(e) => failed = Some(e.error().to_string()),
                        gst::MessageView::AsyncDone(_) => async_done = true,
                        gst::MessageView::DurationChanged(_) => duration_changed = true,
                        gst::MessageView::Element(e) => match e.structure() {
                            Some(s) if s.name() == "level" => levels.push(s.to_owned()),
                            Some(s) if s.name() == "spectrum" => spectrum = Some(s.to_owned()),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }
        // Celui qui s'éteint dans un fondu : ses messages sont jetés.
        if let Some(d) = &self.outgoing {
            d.drain();
        }
        for s in &levels {
            self.on_level(s);
        }
        // Seul le dernier spectre compte : la page n'affiche que le plus récent.
        if let Some(s) = spectrum.filter(|_| VISUAL.load(Ordering::Relaxed)) {
            // Fréquence d'échantillonnage du fichier (44,1 ou 48 kHz) : elle place les bandes.
            let rate = self
                .deck
                .as_ref()
                .and_then(|d| d.spec.static_pad("sink"))
                .and_then(|p| p.current_caps())
                .and_then(|c| c.structure(0).and_then(|st| st.get::<i32>("rate").ok()))
                .unwrap_or(48_000);
            if let Some(bands) = visual_bands(&s, rate as f32) {
                fx.push(Effect::Spectrum(bands));
            }
        }
        // Un fichier qui s'ouvre bien remet le compte des erreurs à zéro.
        if async_done {
            self.errors = 0;
        }
        let mut learned = false;
        if let Some(d) = self.deck.as_mut() {
            if async_done || d.seek_hold.is_some_and(|(_, at)| at.elapsed() >= SEEK_HOLD) {
                d.seek_hold = None;
            }
            if let Some(ms) = d.pending_seek {
                if async_done || duration_changed {
                    d.seek(ms);
                }
                // Fichier prêt mais qui ne se laisse pas déplacer (ou trop long à
                // s'ouvrir) : il part du début plutôt que d'attendre pour toujours.
                if d.pending_seek.is_some() && (async_done || d.opened.elapsed() > SEEK_GIVE_UP) {
                    d.pending_seek = None;
                }
            }
            if d.resume && d.pending_seek.is_none() {
                d.resume = false;
                let _ = d.pipe.set_state(gst::State::Playing);
            }
            if async_done || duration_changed {
                learned = d.learn_duration();
            }
        }
        if let Some(err) = failed {
            return self.failed(app, err, fx);
        }
        if ended {
            self.errors = 0;
            return self.ended(app, fx);
        }

        // Fondu en cours.
        if let Some((start, ms)) = self.fade {
            let k = (now.duration_since(start).as_millis() as f64 / ms as f64).min(1.0);
            if let (true, Some(d)) = (self.fade_in, &self.deck) {
                set_volume(&d.fade, k);
            }
            if let Some(d) = &self.outgoing {
                set_volume(&d.fade, 1.0 - k);
            }
            if k >= 1.0 {
                self.fade = None;
                self.fade_in = false;
                self.outgoing = None;
            }
        }

        // Volume égal : la correction rejoint doucement sa cible (une valeur devenue
        // invalide repart d'une cible saine plutôt que de rester bloquée).
        if !self.norm_target.is_finite() {
            self.norm_target = 1.0;
        }
        if !self.norm_now.is_finite() {
            self.norm_now = self.norm_target;
        }
        if (self.norm_now - self.norm_target).abs() > 1e-3 {
            self.norm_now += (self.norm_target - self.norm_now) * (1.0 - (-dt / self.norm_tau).exp());
            if let Some(d) = &self.deck {
                set_volume(&d.norm, self.norm_now);
            }
        }

        let playing = self.playing && self.deck.is_some();
        if !playing && !learned {
            return;
        }
        // Une seule demande de position par battement.
        let pos = self.position();
        if learned {
            self.report_with(fx, false, false, Some(pos));
        }
        if !playing {
            return;
        }
        let before = self.plan.index;
        self.maybe_crossfade(app, pos, fx);
        if self.plan.index != before {
            return;
        }

        // L'écran ne change qu'à chaque seconde ; le système et la reprise, plus rarement.
        let second = pos / 1000;
        if second != self.last_second {
            self.last_second = second;
            fx.push(Effect::Emit(self.status_at(false, Some(pos))));
        }
        if self.last_push.elapsed() > Duration::from_secs(5) {
            self.last_push = Instant::now();
            fx.push(Effect::Playback(true, pos));
        }
        if self.last_save.elapsed() > Duration::from_secs(15) {
            self.last_save = Instant::now();
            if let Some(id) = self.deck.as_ref().map(|d| d.id.clone()) {
                fx.push(Effect::Save { id, position_ms: pos, playing: true, force: false });
            }
            self.flush_gains(fx);
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
    app.try_state::<Player>().is_some_and(|p| p.orders.is_some())
}

/// Confie un ordre au fil « lecteur ». Faux : pas de lecteur du cœur.
fn send(app: &AppHandle, order: Order) -> bool {
    app.try_state::<Player>()
        .and_then(|p| p.orders.as_ref().map(|tx| tx.send(order).is_ok()))
        .unwrap_or(false)
}

/// Exécute une action sur le moteur, puis ses effets une fois le verrou rendu.
fn with_engine(app: &AppHandle, f: impl FnOnce(&mut Engine, &mut Vec<Effect>)) -> bool {
    let Some(player) = app.try_state::<Player>() else { return false };
    let mut fx = Vec::new();
    {
        let mut guard = player.engine.lock().unwrap();
        let Some(engine) = guard.as_mut() else { return false };
        f(engine, &mut fx);
    }
    run_effects(app, fx);
    true
}

fn run_effects(app: &AppHandle, fx: Vec<Effect>) {
    // La page d'abord : le nouveau morceau s'affiche sans attendre la notification,
    // les touches média ni Discord.
    let (first, rest): (Vec<Effect>, Vec<Effect>) = fx.into_iter().partition(|e| matches!(e, Effect::Emit(_)));
    for effect in first.into_iter().chain(rest) {
        match effect {
            Effect::Started(track) => crate::announce_track(app, &track),
            Effect::Playback(playing, pos) => crate::playback_changed(app, playing, pos),
            Effect::Save { id, position_ms, playing, force } => {
                crate::save_progress_impl(app, Some(id), position_ms, playing, force)
            }
            Effect::Gains(gains) => {
                if let Ok(bytes) = serde_json::to_vec(&gains) {
                    let _ = crate::store::write_atomic(&gains_file(app), &bytes);
                }
            }
            Effect::Emit(status) => {
                let _ = app.emit("player", status);
            }
            Effect::Error(msg) => {
                let _ = app.emit("player-error", msg);
            }
            Effect::Spectrum(bands) => {
                let _ = app.emit("spectrum", bands);
            }
        }
    }
}

/// Démarre le fil « lecteur » (si le lecteur est en service).
pub fn start(app: &AppHandle) {
    let Some(player) = app.try_state::<Player>() else { return };
    let Some(inbox) = player.inbox.lock().unwrap().take() else { return };
    let settings = app.state::<crate::AppSettings>().0.lock().unwrap().clone();
    with_engine(app, |e, _| e.configure(&settings));
    let app = app.clone();
    let _ = std::thread::Builder::new().name("lecteur".into()).spawn(move || run(app, inbox));
}

/// Le fil « lecteur » : les ordres dans l'ordre d'arrivée, et le battement entre eux.
fn run(app: AppHandle, inbox: Receiver<Order>) {
    let mut next_tick = Instant::now();
    loop {
        match inbox.recv_timeout(next_tick.saturating_duration_since(Instant::now())) {
            Ok(order) => {
                let mut beat = SLOW;
                with_engine(&app, |e, fx| {
                    e.handle(&app, order, fx);
                    beat = e.beat();
                });
                // Un morceau qui démarre veut le battement rapide sans attendre.
                next_tick = next_tick.min(Instant::now() + beat);
                if Instant::now() < next_tick {
                    continue;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let mut beat = SLOW;
        with_engine(&app, |e, fx| {
            if e.deck.is_some() || e.outgoing.is_some() {
                e.tick(&app, fx);
            }
            beat = e.beat();
        });
        next_tick = Instant::now() + beat;
    }
}

/// Réglages du son modifiés.
pub fn configure(app: &AppHandle, settings: &crate::store::Settings) {
    send(app, Order::Configure(Box::new(settings.clone())));
}

/// Touches média, icône de la barre et widget du bureau. Faux : action inconnue ici.
pub fn media_control(app: &AppHandle, action: &str) -> bool {
    if !available(app) {
        return false;
    }
    let (name, value) = action.split_once(':').unwrap_or((action, ""));
    let ms: i64 = value.parse().unwrap_or(0);
    let order = match name {
        "play" => Order::SetPlaying(true),
        "pause" => Order::SetPlaying(false),
        "toggle" => Order::Toggle,
        "stop" => Order::Stop,
        "next" => Order::Step(1),
        "previous" => Order::Step(-1),
        "seek-forward" => Order::Skip(1),
        "seek-backward" => Order::Skip(-1),
        "seek-by" => Order::SeekBy(ms),
        "set-position" => Order::Seek(ms.max(0) as u64),
        "quit" => {
            save_now(app);
            app.exit(0);
            return true;
        }
        _ => return false,
    };
    send(app, order)
}

/// Position du moment enregistrée (à la fermeture d'Écho).
pub fn save_now(app: &AppHandle) {
    with_engine(app, |e, fx| {
        e.flush_gains(fx);
        let s = e.status(false);
        if let Some(id) = s.id {
            fx.push(Effect::Save { id, position_ms: s.position_ms, playing: s.playing, force: true });
        }
    });
}

/* ---------------- commandes de la page ---------------- */

/// Nouvelle file, et lecture du morceau `index` (à `start_ms` si donné).
#[tauri::command]
pub fn player_play(app: AppHandle, ids: Vec<String>, index: usize, autoplay: bool, start_ms: Option<u64>, shuffle: bool, repeat: String) {
    send(&app, Order::Play(Request { ids, index, autoplay, start_ms, shuffle, repeat }));
}

/// La file a changé (ajout, retrait, ordre, aléatoire, répétition) sans changer de
/// morceau : le morceau qui joue garde sa place, quelle que soit sa nouvelle position.
#[tauri::command]
pub fn player_queue(app: AppHandle, ids: Vec<String>, index: usize, shuffle: bool, repeat: String) {
    send(&app, Order::Queue { ids, index, shuffle, repeat });
}

#[tauri::command]
pub fn player_set_playing(app: AppHandle, playing: bool) {
    send(&app, Order::SetPlaying(playing));
}

#[tauri::command]
pub fn player_step(app: AppHandle, delta: i32) {
    send(&app, Order::Step(delta));
}

#[tauri::command]
pub fn player_seek(app: AppHandle, ms: u64) {
    send(&app, Order::Seek(ms));
}

/// Règle un élément « volume » de GStreamer. Une valeur hors de ses bornes (0 à 10)
/// ou invalide ferait échouer `set_property`, et le fil lecteur avec : elle est bornée,
/// et une valeur invalide laisse le volume tel quel.
fn set_volume(element: &gst::Element, value: f64) {
    if value.is_finite() {
        element.set_property("volume", value.clamp(0.0, 10.0));
    }
}

/// Morceau intermédiaire de yt-dlp (« Titre.f616.webm ») : l'image sans le son.
fn is_partial_download(path: &str) -> bool {
    let name = Path::new(path).file_stem().and_then(|n| n.to_str()).unwrap_or("");
    name.rsplit_once(".f").is_some_and(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// La page va disparaître (fenêtre rangée dans la barre) : plus de spectre à calculer.
pub fn visual_off(app: &AppHandle) {
    send(app, Order::Visual(false));
}

/// Le visualiseur s'affiche (ou disparaît) : le spectre n'est calculé que s'il est vu.
#[tauri::command]
pub fn player_visual(app: AppHandle, on: bool) {
    send(&app, Order::Visual(on));
}

/// Regroupe les 512 bandes linéaires de GStreamer en bandes logarithmiques (de 0 à 255).
fn visual_bands(s: &gst::StructureRef, rate: f32) -> Option<Vec<u8>> {
    let list = s.get::<gst::List>("magnitude").ok()?;
    let mags: Vec<f32> = list.iter().filter_map(|v| v.get::<f32>().ok()).collect();
    if mags.is_empty() {
        return None;
    }
    Some(group_bands(&mags, rate))
}

fn group_bands(mags: &[f32], rate: f32) -> Vec<u8> {
    let hz_per_bin = rate / 2.0 / mags.len() as f32;
    let (lo, hi) = (40.0f32, 16_000.0f32);
    let ratio = (hi / lo).powf(1.0 / VISUAL_BANDS as f32);
    (0..VISUAL_BANDS)
        .map(|i| {
            let f0 = lo * ratio.powi(i as i32);
            let f1 = f0 * ratio;
            let a = ((f0 / hz_per_bin) as usize).min(mags.len() - 1);
            let b = ((f1 / hz_per_bin).ceil() as usize).clamp(a + 1, mags.len());
            let db = mags[a..b].iter().copied().fold(VISUAL_FLOOR, f32::max);
            // Les aigus portent moins d'énergie : un léger relevé équilibre l'affichage.
            let tilt = 6.0 * i as f32 / VISUAL_BANDS as f32;
            let v = ((db + tilt - VISUAL_FLOOR) / -VISUAL_FLOOR).clamp(0.0, 1.0);
            (v * 255.0).round() as u8
        })
        .collect()
}

#[tauri::command]
pub fn player_volume(app: AppHandle, volume: f64, muted: bool) {
    send(&app, Order::Volume(volume, muted));
}

/// État complet, pour une page qui (re)vient : la file, le morceau, la position.
#[tauri::command]
pub fn player_snapshot(app: AppHandle) -> Option<Snapshot> {
    let player = app.try_state::<Player>()?;
    let guard = player.engine.lock().unwrap();
    let e = guard.as_ref()?;
    Some(Snapshot {
        status: e.status(false),
        ids: e.plan.ids.clone(),
        shuffle: e.plan.shuffle,
        repeat: e.plan.repeat.clone(),
        volume: e.volume,
        muted: e.muted,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn le_spectre_est_regroupe_en_bandes() {
        // 512 bandes linéaires : du silence, sauf une basse vers 100 Hz.
        let mut mags = vec![-80.0f32; 512];
        mags[2] = -10.0;
        let bands = super::group_bands(&mags, 48_000.0);
        assert_eq!(bands.len(), super::VISUAL_BANDS);
        let loud = bands.iter().enumerate().max_by_key(|(_, v)| **v).map(|(i, _)| i).unwrap();
        assert!(loud < 8, "la basse est à gauche : {bands:?}");
        assert!(bands[super::VISUAL_BANDS - 1] < 30, "les aigus restent éteints");
    }

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

    /// Bug de l'audit : « répéter un titre » rejouait sans fin un fichier illisible.
    #[test]
    fn un_morceau_illisible_laisse_la_place_a_un_autre() {
        let mut first = |_: usize| 0;
        let mut p = plan(3, 1, false, "one");
        assert_eq!(p.next_after_failure(&mut first), Some(2));
        assert_eq!(p.repeat, "one", "le réglage de répétition est gardé");
        assert_eq!(plan(3, 2, false, "one").next_after_failure(&mut first), Some(0));
        // Fin de file sans répétition : on s'arrête.
        assert_eq!(plan(3, 2, false, "off").next_after_failure(&mut first), None);
    }

    /// Bug de l'audit : un morceau présent deux fois dans la file sautait à sa première place.
    #[test]
    fn un_doublon_garde_sa_place_dans_la_file() {
        let ids: Vec<String> = ["a", "b", "a", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(place_in(&ids, 2, Some("a")), 2);
        assert_eq!(place_in(&ids, 3, Some("b")), 1);
        assert_eq!(place_in(&ids, 9, Some("z")), 3);
        assert_eq!(place_in(&ids, 1, None), 1);
        assert_eq!(place_in(&[], 4, Some("a")), 0);
    }

    /// Bug de l'audit : relancer la même file en aléatoire oubliait les morceaux déjà joués.
    #[test]
    fn la_meme_file_garde_la_memoire_de_l_aleatoire() {
        let mut p = plan(5, 0, true, "off");
        p.played.insert(3);
        p.history.push(3);
        let ids = p.ids.clone();
        p.adopt(ids.clone(), 2, true, "all".into());
        assert_eq!(p.index, 2);
        assert_eq!(p.history, vec![3, 0]);
        assert!(p.played.contains(&3) && p.played.contains(&0));
        assert_eq!(p.repeat, "all");
        // Autre file : tout repart de zéro.
        p.adopt(ids[..3].to_vec(), 1, true, "off".into());
        assert!(p.history.is_empty() && p.played.is_empty());
    }

    #[test]
    fn les_statuts_sont_numerotes_dans_l_ordre() {
        let a = next_seq();
        let b = next_seq();
        assert!(b > a);
    }
}

/// Essai réel (silencieux) : cargo test player::essai_sortie -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore]
fn essai_sortie() {
    gst::init().unwrap();
    let pipe = gst::parse::launch("audiotestsrc volume=0 ! pulsesink name=out").unwrap().downcast::<gst::Pipeline>().unwrap();
    let out = pipe.by_name("out").unwrap();
    let props = gst::Structure::builder("props")
        .field("application.name", "Écho")
        .field("module-stream-restore.id", "echo-player-musique")
        .build();
    out.set_property("stream-properties", props);
    out.set_property("client-name", "Écho");
    out.set_property("mute", false);
    pipe.set_state(gst::State::Playing).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let list = std::process::Command::new("pactl").args(["list", "sink-inputs"]).output().unwrap();
    for line in String::from_utf8_lossy(&list.stdout).lines() {
        if line.contains("Sink Input") || line.contains("Mute") || line.contains("application.name") || line.contains("stream-restore") {
            println!("{line}");
        }
    }
    pipe.set_state(gst::State::Null).unwrap();
}
