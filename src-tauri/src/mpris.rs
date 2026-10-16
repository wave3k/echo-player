use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::time::Duration;

/// Petite enveloppe autour des controles media du systeme (MPRIS sur Linux).
/// Si le systeme ne repond pas, l'app continue normalement sans eux.
pub struct Mpris {
    controls: Option<MediaControls>,
}

impl Mpris {
    pub fn new() -> Self {
        #[cfg(not(target_os = "windows"))]
        let hwnd = None;

        let config = PlatformConfig {
            dbus_name: "musicapp",
            display_name: "MusicApp",
            hwnd,
        };

        let controls = MediaControls::new(config).ok();
        Self { controls }
    }

    pub fn attach<F>(&mut self, handler: F)
    where
        F: Fn(MediaControlEvent) + Send + 'static,
    {
        if let Some(controls) = self.controls.as_mut() {
            let _ = controls.attach(handler);
        }
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
pub fn describe(event: &MediaControlEvent) -> Option<String> {
    let label = match event {
        MediaControlEvent::Play => "play",
        MediaControlEvent::Pause => "pause",
        MediaControlEvent::Toggle => "toggle",
        MediaControlEvent::Stop => "stop",
        MediaControlEvent::Next => "next",
        MediaControlEvent::Previous => "previous",
        MediaControlEvent::Seek(SeekDirection::Forward) => "seek-forward",
        MediaControlEvent::Seek(SeekDirection::Backward) => "seek-backward",
        MediaControlEvent::SetPosition(MediaPosition(_)) => "set-position",
        MediaControlEvent::Raise => "raise",
        MediaControlEvent::Quit => "quit",
        _ => return None,
    };
    Some(label.to_string())
}
