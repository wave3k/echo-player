use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::time::Duration;

/// Petite enveloppe autour des controles media du systeme (MPRIS sur Linux).
/// Si le systeme ne repond pas, l'app continue normalement sans eux.
/// On peut les eteindre et les rallumer depuis les reglages.
pub struct Mpris {
    controls: Option<MediaControls>,
    handle: tauri::AppHandle,
}

fn platform_config(handle: &tauri::AppHandle) -> PlatformConfig<'static> {
    #[cfg(not(target_os = "windows"))]
    let hwnd = {
        let _ = handle;
        None
    };
    // Sous Windows, les contrôles média s'attachent à la fenêtre principale.
    #[cfg(target_os = "windows")]
    let hwnd = handle
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as *mut std::ffi::c_void);

    PlatformConfig {
        dbus_name: "echo",
        display_name: "Écho",
        hwnd,
    }
}

impl Mpris {
    pub fn new(handle: &tauri::AppHandle) -> Self {
        let mut mpris = Self {
            controls: None,
            handle: handle.clone(),
        };
        let _ = mpris.set_enabled(true);
        mpris
    }

    /// Allume ou eteint les touches media du clavier.
    ///
    /// Éteintes, les commandes sont rendues à l'appelant, qui doit les détruire une fois
    /// le verrou `Media` rendu : leur destruction attend la fin du fil D-Bus, qui peut
    /// justement attendre ce verrou (touche pressée au même moment).
    #[must_use = "à détruire hors du verrou Media"]
    pub fn set_enabled(&mut self, enabled: bool) -> Option<MediaControls> {
        if !enabled {
            return self.controls.take();
        }
        if self.controls.is_some() {
            return None;
        }
        if let Ok(mut controls) = MediaControls::new(platform_config(&self.handle)) {
            let handle = self.handle.clone();
            let _ = controls.attach(move |event| {
                if let Some(label) = describe(&event) {
                    // Un clic sur le widget du bureau ramene la fenetre au premier plan.
                    if label == "raise" {
                        // Sur le fil principal : la fenêtre a pu être fermée, il faut la recréer.
                        let app = handle.clone();
                        let _ = handle.run_on_main_thread(move || crate::show_window(&app));
                    }
                    crate::media_control(&handle, &label);
                }
            });
            self.controls = Some(controls);
        }
        None
    }

    pub fn set_metadata(
        &mut self,
        title: &str,
        artist: &str,
        album: &str,
        cover_url: Option<String>,
        duration: Option<Duration>,
    ) {
        let Some(controls) = self.controls.as_mut() else {
            return;
        };
        let metadata = MediaMetadata {
            title: Some(title),
            artist: if artist.is_empty() { None } else { Some(artist) },
            album: if album.is_empty() { None } else { Some(album) },
            cover_url: cover_url.as_deref(),
            duration,
        };
        let _ = controls.set_metadata(metadata);
    }

    pub fn set_playback(&mut self, playing: bool, position: Option<Duration>) {
        let Some(controls) = self.controls.as_mut() else {
            return;
        };
        let status = if playing {
            MediaPlayback::Playing {
                progress: position.map(MediaPosition),
            }
        } else {
            MediaPlayback::Paused {
                progress: position.map(MediaPosition),
            }
        };
        let _ = controls.set_playback(status);
    }
}

/// Ce que l'app doit faire quand une touche media est pressee.
/// Les actions qui portent une valeur la donnent après « : », en millisecondes
/// (« set-position:12000 », « seek-by:-5000 »).
pub fn describe(event: &MediaControlEvent) -> Option<String> {
    match event {
        MediaControlEvent::SetPosition(MediaPosition(at)) => {
            return Some(format!("set-position:{}", at.as_millis()));
        }
        MediaControlEvent::SeekBy(direction, by) => {
            let sign = if matches!(direction, SeekDirection::Forward) { "" } else { "-" };
            return Some(format!("seek-by:{sign}{}", by.as_millis()));
        }
        _ => {}
    }
    let label = match event {
        MediaControlEvent::Play => "play",
        MediaControlEvent::Pause => "pause",
        MediaControlEvent::Toggle => "toggle",
        MediaControlEvent::Stop => "stop",
        MediaControlEvent::Next => "next",
        MediaControlEvent::Previous => "previous",
        MediaControlEvent::Seek(SeekDirection::Forward) => "seek-forward",
        MediaControlEvent::Seek(SeekDirection::Backward) => "seek-backward",
        MediaControlEvent::Raise => "raise",
        MediaControlEvent::Quit => "quit",
        _ => return None,
    };
    Some(label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_position_demandee_par_le_bureau_est_transmise() {
        let event = MediaControlEvent::SetPosition(MediaPosition(Duration::from_millis(83_500)));
        assert_eq!(describe(&event).as_deref(), Some("set-position:83500"));
    }

    #[test]
    fn un_saut_relatif_garde_son_sens() {
        let back = MediaControlEvent::SeekBy(SeekDirection::Backward, Duration::from_secs(5));
        let fwd = MediaControlEvent::SeekBy(SeekDirection::Forward, Duration::from_secs(5));
        assert_eq!(describe(&back).as_deref(), Some("seek-by:-5000"));
        assert_eq!(describe(&fwd).as_deref(), Some("seek-by:5000"));
    }

    #[test]
    fn quitter_est_transmis() {
        assert_eq!(describe(&MediaControlEvent::Quit).as_deref(), Some("quit"));
    }
}
