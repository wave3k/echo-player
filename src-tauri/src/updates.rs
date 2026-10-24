//! Mises à jour d'Écho, depuis les releases GitHub. Chaque fichier est signé : une
//! version dont la signature ne correspond pas à la clé publique d'Écho est refusée.
//!
//! Selon l'installation :
//! - Windows et AppImage : le module officiel de Tauri remplace l'application ;
//! - binaire installé par install.sh (dossier personnel) : l'archive signée
//!   « linux-x86_64-bin » remplace le binaire ;
//! - paquet .deb (dans /usr) : c'est le gestionnaire de paquets qui met à jour,
//!   Écho prévient seulement.

use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Page des versions, ouverte quand Écho ne peut pas se mettre à jour lui-même.
pub const RELEASES_URL: &str = "https://github.com/wave3k/echo-player/releases/latest";

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Install {
    Windows,
    AppImage,
    /// Binaire dans un dossier où l'on peut écrire (install.sh, ~/.local/bin).
    Binary,
    /// Paquet système (.deb, .rpm…) : mise à jour par le gestionnaire de paquets.
    Package,
}

pub fn install_kind() -> Install {
    if cfg!(windows) {
        return Install::Windows;
    }
    if std::env::var_os("APPIMAGE").is_some() {
        return Install::AppImage;
    }
    match std::env::current_exe() {
        Ok(exe) if !is_system_path(&exe) && exe.parent().is_some_and(writable) => Install::Binary,
        _ => Install::Package,
    }
}

fn is_system_path(path: &Path) -> bool {
    ["/usr/", "/opt/", "/bin/", "/sbin/"].iter().any(|p| path.starts_with(p))
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".echo-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub current: String,
    /// Version disponible, s'il y en a une plus récente.
    pub available: Option<String>,
    pub notes: Option<String>,
    pub install: Option<Install>,
    /// Vrai si Écho peut s'installer la mise à jour lui-même.
    pub can_install: bool,
    /// Mise à jour installée : elle s'applique au prochain lancement.
    pub ready: bool,
    pub error: Option<String>,
}

/// Mise à jour trouvée, et (Windows) l'installeur téléchargé en attente du redémarrage.
#[derive(Default)]
pub struct Pending {
    update: Option<Update>,
    windows_installer: Option<Vec<u8>>,
    status: Status,
}

pub struct Updates(pub Mutex<Pending>);

impl Default for Updates {
    fn default() -> Self {
        Self(Mutex::new(Pending::default()))
    }
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Cherche une version plus récente. Ne télécharge rien.
pub async fn check(app: &AppHandle) -> Status {
    let kind = install_kind();
    let mut status = Status {
        current: current_version(app),
        install: Some(kind),
        ..Default::default()
    };
    let mut builder = app.updater_builder().timeout(std::time::Duration::from_secs(20));
    if kind == Install::Binary {
        builder = builder.target("linux-x86_64-bin");
    }
    // Autre source (essais) : https obligatoire, et la signature d'Écho reste exigée.
    if let Some(url) = std::env::var("ECHO_UPDATE_ENDPOINT")
        .ok()
        .and_then(|u| u.parse::<reqwest::Url>().ok())
        .filter(|u| u.scheme() == "https")
    {
        match builder.endpoints(vec![url]) {
            Ok(b) => builder = b,
            Err(e) => {
                status.error = Some(e.to_string());
                return remember(app, status, None);
            }
        }
    }
    let result = match builder.build() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    match result {
        Ok(Some(update)) => {
            status.available = Some(update.version.clone());
            status.notes = update.body.clone();
            status.can_install = kind != Install::Package;
            remember(app, status, Some(update))
        }
        Ok(None) => remember(app, status, None),
        Err(e) => {
            status.error = Some(e.to_string());
            remember(app, status, None)
        }
    }
}

fn remember(app: &AppHandle, status: Status, update: Option<Update>) -> Status {
    let state = app.state::<Updates>();
    let mut pending = state.0.lock().unwrap();
    // Une mise à jour déjà installée reste prête, même si une nouvelle vérification échoue.
    if pending.status.ready && status.available.is_none() {
        return pending.status.clone();
    }
    pending.update = update;
    pending.status = status.clone();
    status
}

/// Télécharge (signature vérifiée) et installe la mise à jour trouvée par `check`.
pub async fn install(app: &AppHandle) -> Status {
    let update = {
        let state = app.state::<Updates>();
        let pending = state.0.lock().unwrap();
        if pending.status.ready {
            return pending.status.clone();
        }
        pending.update.clone()
    };
    let Some(update) = update else {
        return check(app).await;
    };
    let kind = install_kind();
    let result: Result<(), String> = async {
        let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
        match kind {
            Install::AppImage => update.install(&bytes).map_err(|e| e.to_string()),
            Install::Binary => replace_binary(&bytes),
            Install::Windows => {
                // L'installeur ferme Écho : il ne tourne qu'au redémarrage demandé.
                app.state::<Updates>().0.lock().unwrap().windows_installer = Some(bytes);
                Ok(())
            }
            Install::Package => Err("Mise à jour par le gestionnaire de paquets".into()),
        }
    }
    .await;
    let state = app.state::<Updates>();
    let mut pending = state.0.lock().unwrap();
    match result {
        Ok(()) => {
            pending.status.ready = true;
            pending.status.error = None;
        }
        Err(e) => pending.status.error = Some(e),
    }
    pending.status.clone()
}

/// Remplace le binaire en cours d'exécution par celui de l'archive (tar.gz signée).
/// Sous Linux, le programme lancé garde l'ancien fichier jusqu'à sa fermeture.
fn replace_binary(archive: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let binary = extract_binary(archive)?;
    let staged: PathBuf = exe.with_extension("update");
    std::fs::write(&staged, &binary).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staged, &exe).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        e.to_string()
    })
}

/// Le binaire « echo-player » contenu dans l'archive.
pub fn extract_binary(archive: &[u8]) -> Result<Vec<u8>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let is_binary = entry
            .path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == "echo-player"))
            .unwrap_or(false);
        if is_binary {
            // Taille plafonnée (200 Mo) : une archive piégée ne remplit pas la mémoire.
            const MAX: u64 = 200 * 1024 * 1024;
            let mut out = Vec::new();
            (&mut entry).take(MAX + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
            if out.len() as u64 > MAX {
                return Err("archive de mise à jour trop grosse".into());
            }
            if out.len() < 1024 {
                return Err("archive de mise à jour invalide".into());
            }
            return Ok(out);
        }
    }
    Err("binaire absent de l'archive de mise à jour".into())
}

/// Redémarre sur la nouvelle version (Windows : lance l'installeur, qui relance Écho).
pub fn restart(app: &AppHandle) {
    let (update, installer) = {
        let state = app.state::<Updates>();
        let mut pending = state.0.lock().unwrap();
        (pending.update.clone(), pending.windows_installer.take())
    };
    if let (Some(update), Some(bytes)) = (update, installer) {
        let _ = update.install(bytes);
    }
    app.restart();
}

/// Windows : une mise à jour téléchargée s'installe quand on quitte Écho.
pub fn install_on_quit(app: &AppHandle) {
    let (update, installer) = {
        let state = app.state::<Updates>();
        let mut pending = state.0.lock().unwrap();
        (pending.update.clone(), pending.windows_installer.take())
    };
    if let (Some(update), Some(bytes)) = (update, installer) {
        let _ = update.install(bytes);
    }
}

/// Vérification en tâche de fond : peu après le démarrage, puis toutes les 6 heures.
/// Mode « auto » : la mise à jour est installée aussitôt et s'applique au prochain lancement.
pub fn start_background(app: AppHandle, mode: impl Fn(&AppHandle) -> String + Send + 'static) {
    let _ = std::thread::Builder::new().name("mises-a-jour".into()).spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(25));
        loop {
            let wanted = mode(&app);
            if wanted != "off" {
                let status = tauri::async_runtime::block_on(check(&app));
                if status.available.is_some() {
                    let _ = app.emit("update-status", &status);
                    if wanted == "auto" && status.can_install {
                        let status = tauri::async_runtime::block_on(install(&app));
                        let _ = app.emit("update-status", &status);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive_with(name: &str, size: usize) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut tar = tar::Builder::new(&mut gz);
            let data = vec![7u8; size];
            let mut header = tar::Header::new_gnu();
            header.set_size(size as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, data.as_slice()).unwrap();
            tar.finish().unwrap();
        }
        gz.flush().unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn le_binaire_est_extrait_de_l_archive() {
        let bin = extract_binary(&archive_with("echo-player", 4096)).unwrap();
        assert_eq!(bin.len(), 4096);
    }

    #[test]
    fn une_archive_sans_binaire_est_refusee() {
        assert!(extract_binary(&archive_with("autre-chose", 4096)).is_err());
        assert!(extract_binary(&archive_with("echo-player", 10)).is_err());
        assert!(extract_binary(b"pas une archive").is_err());
    }

    #[test]
    fn les_chemins_systeme_sont_reconnus() {
        assert!(is_system_path(Path::new("/usr/bin/echo-player")));
        assert!(!is_system_path(Path::new("/home/moi/.local/bin/echo-player")));
    }
}
