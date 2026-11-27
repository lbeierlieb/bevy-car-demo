//! Text files kept between runs, by name: in a folder under the config or cache directory
//! (`~/.config/car-sim/…`, `~/.cache/car-sim/…`; on Windows `%APPDATA%\car-sim\…` and
//! `%LOCALAPPDATA%\car-sim\…`), or the browser's local storage on the web.

/// A folder of text files with one extension, or on the web the local storage keys with
/// one prefix.
pub struct Folder {
    /// Kept in the cache directory (can be deleted), rather than with the settings.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub cache: bool,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub name: &'static str,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub extension: &'static str,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub web_prefix: &'static str,
}

/// What a folder of the game's holds, which decides where it is.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy)]
pub enum Kind {
    /// Settings, tunes (`~/.config`).
    Config,
    /// What can be worked out again: engine maps (`~/.cache`).
    Cache,
    /// Playing alone's database (`~/.local/share`).
    Data,
}

/// The game's folder of a kind: `car-sim` under the XDG directory (or its default under
/// `HOME`); on Windows under `%APPDATA%` (config and data) or `%LOCALAPPDATA%` (cache).
#[cfg(not(target_arch = "wasm32"))]
pub fn game_dir(kind: Kind) -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let (variable, fallback) = match kind {
        Kind::Config => ("XDG_CONFIG_HOME", ".config"),
        Kind::Cache => ("XDG_CACHE_HOME", ".cache"),
        Kind::Data => ("XDG_DATA_HOME", ".local/share"),
    };
    let xdg = || {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(fallback)))
    };
    let base = if cfg!(windows) {
        let windows = match kind {
            Kind::Cache => "LOCALAPPDATA",
            Kind::Config | Kind::Data => "APPDATA",
        };
        std::env::var_os(windows).map(PathBuf::from).or_else(xdg)
    } else {
        xdg()
    }?;
    Some(base.join("car-sim"))
}

/// The folder for the game's data (`~/.local/share/car-sim`, or under `XDG_DATA_HOME`):
/// playing alone's database.
#[cfg(not(target_arch = "wasm32"))]
pub fn data_dir() -> Option<std::path::PathBuf> {
    game_dir(Kind::Data)
}

#[cfg(not(target_arch = "wasm32"))]
impl Folder {
    fn dir(&self) -> Option<std::path::PathBuf> {
        let kind = if self.cache {
            Kind::Cache
        } else {
            Kind::Config
        };
        Some(game_dir(kind)?.join(self.name))
    }

    pub fn list(&self) -> Vec<String> {
        self.dir()
            .map_or_else(Vec::new, |dir| list_in(&dir, self.extension))
    }

    pub fn load(&self, name: &str) -> Option<String> {
        load_in(&self.dir()?, name, self.extension)
    }

    pub fn save(&self, name: &str, text: &str) -> Result<(), String> {
        let dir = self.dir().ok_or("no home directory")?;
        save_in(&dir, name, self.extension, text)
    }

    pub fn delete(&self, name: &str) {
        if let Some(dir) = self.dir() {
            delete_in(&dir, name, self.extension);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod files {
    use std::path::Path;

    pub fn list_in(dir: &Path, extension: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == extension))
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
            .collect();
        names.sort();
        names
    }

    pub fn load_in(dir: &Path, name: &str, extension: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(name).with_extension(extension)).ok()
    }

    pub fn save_in(dir: &Path, name: &str, extension: &str, text: &str) -> Result<(), String> {
        let path = dir.join(name).with_extension(extension);
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, text))
            .map_err(|err| format!("{}: {err}", path.display()))
    }

    pub fn delete_in(dir: &Path, name: &str, extension: &str) {
        let _ = std::fs::remove_file(dir.join(name).with_extension(extension));
    }
}

#[cfg(not(target_arch = "wasm32"))]
use files::*;

#[cfg(target_arch = "wasm32")]
impl Folder {
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    pub fn list(&self) -> Vec<String> {
        let Some(storage) = Self::storage() else {
            return Vec::new();
        };
        let count = storage.length().unwrap_or(0);
        let mut names: Vec<String> = (0..count)
            .filter_map(|i| storage.key(i).ok()?)
            .filter_map(|key| Some(key.strip_prefix(self.web_prefix)?.to_string()))
            .collect();
        names.sort();
        names
    }

    pub fn load(&self, name: &str) -> Option<String> {
        Self::storage()?
            .get_item(&format!("{}{name}", self.web_prefix))
            .ok()?
    }

    pub fn save(&self, name: &str, text: &str) -> Result<(), String> {
        let storage = Self::storage().ok_or("no local storage")?;
        storage
            .set_item(&format!("{}{name}", self.web_prefix), text)
            .map_err(|_| "the browser refused to store it".to_string())
    }

    pub fn delete(&self, name: &str) {
        if let Some(storage) = Self::storage() {
            let _ = storage.remove_item(&format!("{}{name}", self.web_prefix));
        }
    }
}

/// The files themselves (natively).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn saves_lists_loads_and_deletes() {
        let dir = std::env::temp_dir().join(format!("car-sim-storage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(list_in(&dir, "tune").is_empty());
        save_in(&dir, "b tune", "tune", "two").unwrap();
        save_in(&dir, "a tune", "tune", "one").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a tune").unwrap();
        assert_eq!(list_in(&dir, "tune"), ["a tune", "b tune"]);
        assert_eq!(load_in(&dir, "b tune", "tune").as_deref(), Some("two"));
        delete_in(&dir, "a tune", "tune");
        assert_eq!(list_in(&dir, "tune"), ["b tune"]);
        assert_eq!(load_in(&dir, "a tune", "tune"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
