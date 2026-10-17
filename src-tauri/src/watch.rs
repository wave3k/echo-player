//! Surveillance des dossiers de musique : un fichier ajouté, supprimé ou renommé
//! déclenche une relecture de la bibliothèque. Les rafales d'événements (copie de
//! tout un album) sont regroupées en une seule relecture.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

/// Délai de calme après le dernier événement avant de relire.
const SETTLE: Duration = Duration::from_millis(1500);

pub struct FolderWatch {
    // Gardé en vie : le relâcher arrête la surveillance (et son fil).
    _watcher: RecommendedWatcher,
}

/// Vrai si l'événement concerne un fichier qui peut entrer dans la bibliothèque.
fn relevant(event: &Event) -> bool {
    let kind_ok = matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_));
    kind_ok
        && event.paths.iter().any(|p| {
            // Un dossier renommé ou supprimé n'a plus d'extension : il compte aussi.
            p.extension().is_none() || crate::library::is_media_path(p)
        })
}

impl FolderWatch {
    /// Surveille `folders` et appelle `on_change` après chaque série de changements.
    pub fn start(folders: &[String], on_change: impl Fn() + Send + 'static) -> Option<Self> {
        let (tx, rx) = channel::<Event>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if relevant(&event) {
                    let _ = tx.send(event);
                }
            }
        })
        .ok()?;
        let mut watching = 0;
        for folder in folders {
            if Path::new(folder).is_dir() && watcher.watch(&PathBuf::from(folder), RecursiveMode::Recursive).is_ok() {
                watching += 1;
            }
        }
        if watching == 0 {
            return None;
        }
        std::thread::Builder::new()
            .name("surveillance".into())
            .spawn(move || {
                // Attend un premier événement, puis le calme, puis relit une fois.
                while rx.recv().is_ok() {
                    loop {
                        match rx.recv_timeout(SETTLE) {
                            Ok(_) => continue,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    on_change();
                }
            })
            .ok()?;
        Some(Self { _watcher: watcher })
    }
}
