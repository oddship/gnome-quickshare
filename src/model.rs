use crate::i18n::{ntr, tr, trf};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    pub downloads: PathBuf,
    pub background: bool,
    pub autostart: bool,
    pub tray: bool,
    pub port: Option<u32>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            name: std::env::var("HOSTNAME").unwrap_or_else(|_| tr("My computer")),
            downloads: dirs::download_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"))
                .join("Quickshare"),
            background: false,
            autostart: false,
            tray: false,
            port: None,
        }
    }
}
impl Settings {
    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_default()
            .join("gnome-quickshare/settings.json")
    }
    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        self.validate()?;
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(self).unwrap())
            .map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.len() > 32 {
            return Err(tr("Choose a short, non-empty device name."));
        }
        if !self.downloads.is_absolute() {
            return Err(tr("Choose an absolute download folder."));
        }
        if self.port.is_some_and(|p| !(1024..=65535).contains(&p)) {
            return Err(tr("Use a port between 1024 and 65535."));
        }
        Ok(())
    }
}
pub fn validate_files(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    if paths.len() > 1024 {
        return Err(tr("Share up to 1,024 files at a time."));
    }
    if paths.is_empty() {
        return Err(tr("Choose at least one file."));
    }
    paths
        .iter()
        .map(|path| {
            let metadata =
                std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
            if !metadata.is_file() {
                return Err(trf(
                    "{path} is a folder. Choose files to share.",
                    &[("path", &path.display().to_string())],
                ));
            }
            let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            path.to_str()
                .map(str::to_owned)
                .ok_or_else(|| tr("A file path is not valid UTF-8."))
        })
        .collect()
}
pub fn safe_offer(files: &[String]) -> bool {
    files.iter().all(|name| {
        let path = Path::new(name);
        !name.is_empty()
            && !name.contains(['/', '\\', '\0'])
            && path.components().count() == 1
            && name != "."
            && name != ".."
    })
}
pub fn bytes(value: u64) -> String {
    gtk::glib::format_size(value).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_offers() {
        for name in ["../escape", "/etc/passwd", "..", ".", "a/b", "a\\b", ""] {
            assert!(!safe_offer(&[name.into()]));
        }
        assert!(safe_offer(&["photo.jpg".into(), "Résumé.pdf".into()]));
    }
    #[test]
    fn validates_settings() {
        let mut s = Settings::default();
        s.name.clear();
        assert!(s.validate().is_err());
        s.name = "Laptop".into();
        s.port = Some(65536);
        assert!(s.validate().is_err());
        s.port = Some(53317);
        assert!(s.validate().is_ok());
    }
    #[test]
    fn rejects_empty_selections_and_directories() {
        assert!(validate_files(&[std::env::temp_dir()]).is_err());
        assert!(validate_files(&[]).is_err());
    }
    #[test]
    fn empty_regular_files_are_valid_selections() {
        let path = std::env::temp_dir().join(format!("quickshare-empty-{}", std::process::id()));
        let file = std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        assert_eq!(file.metadata().unwrap().len(), 0);
        assert!(validate_files(std::slice::from_ref(&path)).is_ok());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub device: String,
    pub incoming: bool,
    pub bytes: u64,
    pub outcome: String,
    pub timestamp: u64,
}
pub fn history_path() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_default())
        .join("gnome-quickshare/history.json")
}
pub fn load_history() -> Vec<HistoryEntry> {
    std::fs::read(history_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn store_history(entries: &[HistoryEntry]) -> Result<(), String> {
    let path = history_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(entries).unwrap())
        .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

impl Settings {
    pub fn install_autostart(&self) -> Result<(), String> {
        let directory = dirs::config_dir()
            .ok_or_else(|| tr("No configuration directory is available."))?
            .join("autostart");
        let path = directory.join("io.github.oddship.GnomeQuickshare.desktop");
        if self.autostart {
            std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
            std::fs::write(path, "[Desktop Entry]\nType=Application\nName=Quickshare\nExec=gnome-quickshare --background\nIcon=io.github.oddship.GnomeQuickshare\nX-GNOME-Autostart-enabled=true\n").map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}

pub fn file_count(count: usize) -> String {
    ntr("{count} file", "{count} files", count)
}

pub fn display_path(path: &std::path::Path) -> String {
    let pretty = if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(home)
    {
        if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        }
    } else {
        path.display().to_string()
    };
    if pretty.chars().count() <= 28 {
        return pretty;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let name = if name.chars().count() > 20 {
        format!(
            "{}…{}",
            name.chars().take(8).collect::<String>(),
            name.chars()
                .rev()
                .take(8)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    } else {
        name.into_owned()
    };
    if pretty.starts_with("~/") {
        format!("~/…/{name}")
    } else {
        let first = path
            .components()
            .find_map(|part| {
                if let std::path::Component::Normal(part) = part {
                    Some(part.to_string_lossy().chars().take(8).collect::<String>())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        format!("/{first}/…/{name}")
    }
}
