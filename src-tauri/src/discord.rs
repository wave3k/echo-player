//! Statut Discord (Rich Presence) : « Écoute <titre> », avec l'artiste et la
//! progression. Tourne dans son propre fil et se reconnecte si Discord est lancé
//! après Écho ; désactivé, il ne fait rien et ne garde aucune connexion ouverte.

use discord_rich_presence::activity::{Activity, ActivityType, Assets, StatusDisplayType, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Identifiant de l'application Discord « Écho », créée sur le portail développeur
/// de Discord (discord.com/developers/applications). Le nom de l'application est
/// celui que Discord affiche. Vide : le statut Discord reste inactif.
/// La variable d'environnement ECHO_DISCORD_APP_ID le remplace (pour essayer).
pub const APP_ID: &str = "1560984989225586780";

pub fn app_id() -> Option<String> {
    let id = std::env::var("ECHO_DISCORD_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let id = id.trim().to_string();
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then_some(id)
}

/// Image affichée à côté du statut (Discord accepte une adresse https).
const ICON_URL: &str = "https://raw.githubusercontent.com/wave3k/echo-player/main/src-tauri/icons/icon.png";

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
}

enum Command {
    Enable(bool),
    Update(Option<NowPlaying>),
}

pub struct Discord {
    tx: Sender<Command>,
}

impl Discord {
    pub fn start(enabled: bool) -> Self {
        let (tx, rx) = channel::<Command>();
        std::thread::Builder::new()
            .name("discord".into())
            .spawn(move || {
                let mut enabled = enabled;
                let mut client: Option<DiscordIpcClient> = None;
                let mut current: Option<NowPlaying> = None;
                let mut dirty = true;
                loop {
                    // Attente d'un ordre ; toutes les 15 s, on retente la connexion si besoin.
                    match rx.recv_timeout(Duration::from_secs(15)) {
                        Ok(Command::Enable(on)) => {
                            enabled = on;
                            dirty = true;
                        }
                        Ok(Command::Update(now)) => {
                            if now != current {
                                current = now;
                                dirty = true;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if !enabled {
                        if let Some(mut c) = client.take() {
                            let _ = c.clear_activity();
                            let _ = c.close();
                        }
                        continue;
                    }
                    let Some(id) = app_id() else { continue };
                    if client.is_none() {
                        let mut c = DiscordIpcClient::new(id);
                        if c.connect().is_ok() {
                            client = Some(c);
                            dirty = true;
                        } else {
                            continue;
                        }
                    }
                    if !dirty {
                        continue;
                    }
                    let ok = match (&mut client, &current) {
                        (Some(c), Some(now)) if now.playing => c.set_activity(activity(now)).is_ok(),
                        (Some(c), _) => c.clear_activity().is_ok(),
                        _ => true,
                    };
                    if ok {
                        dirty = false;
                    } else {
                        // Discord fermé entre-temps : on se reconnectera au prochain tour.
                        client = None;
                    }
                }
            })
            .expect("fil Discord");
        Self { tx }
    }

    pub fn set_enabled(&self, on: bool) {
        let _ = self.tx.send(Command::Enable(on));
    }

    pub fn update(&self, now: Option<NowPlaying>) {
        let _ = self.tx.send(Command::Update(now));
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Le statut tel que Discord l'affiche : titre, artiste, barre de progression.
fn activity(now: &NowPlaying) -> Activity<'_> {
    let start = now_ms() - now.position_ms as i64;
    let mut times = Timestamps::new().start(start);
    if now.duration_ms > 0 {
        times = times.end(start + now.duration_ms as i64);
    }
    let artist = if now.artist.is_empty() { "Artiste inconnu" } else { now.artist.as_str() };
    let mut assets = Assets::new().large_image(ICON_URL);
    if !now.album.is_empty() {
        assets = assets.large_text(now.album.as_str());
    }
    Activity::new()
        .activity_type(ActivityType::Listening)
        .status_display_type(StatusDisplayType::Details)
        .details(clip(&now.title))
        .state(clip(artist))
        .timestamps(times)
        .assets(assets)
}

/// Discord refuse les textes de moins de 2 ou plus de 128 caractères.
fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(127).collect();
    while out.chars().count() < 2 {
        out.push(' ');
    }
    out
}
