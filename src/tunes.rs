//! Saved ECU tunes, by name: text files in the config directory
//! (`~/.config/car-sim/tunes/<name>.tune`), or the browser's local storage on the web.

/// Longest tune name.
const NAME_LENGTH: usize = 40;

/// A tune name that is safe as a file name: letters, digits, spaces, `-` and `_`, trimmed.
/// None if nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(NAME_LENGTH)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::path::{Path, PathBuf};

    const EXTENSION: &str = "tune";

    fn dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
        Some(base.join("car-sim").join("tunes"))
    }

    pub fn list() -> Vec<String> {
        dir().map_or_else(Vec::new, |dir| list_in(&dir))
    }

    pub fn load(name: &str) -> Option<String> {
        load_in(&dir()?, name)
    }

    pub fn save(name: &str, text: &str) -> Result<(), String> {
        save_in(&dir().ok_or("no config directory")?, name, text)
    }

    pub fn delete(name: &str) {
        if let Some(dir) = dir() {
            delete_in(&dir, name);
        }
    }

    pub(super) fn list_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == EXTENSION))
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
            .collect();
        names.sort();
        names
    }

    pub(super) fn load_in(dir: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(name).with_extension(EXTENSION)).ok()
    }

    pub(super) fn save_in(dir: &Path, name: &str, text: &str) -> Result<(), String> {
        let path = dir.join(name).with_extension(EXTENSION);
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, text))
            .map_err(|err| format!("{}: {err}", path.display()))
    }

    pub(super) fn delete_in(dir: &Path, name: &str) {
        let _ = std::fs::remove_file(dir.join(name).with_extension(EXTENSION));
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    /// Local storage keys of tunes: this prefix and the name.
    const PREFIX: &str = "car-sim-tune:";

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    pub fn list() -> Vec<String> {
        let Some(storage) = storage() else {
            return Vec::new();
        };
        let count = storage.length().unwrap_or(0);
        let mut names: Vec<String> = (0..count)
            .filter_map(|i| storage.key(i).ok()?)
            .filter_map(|key| Some(key.strip_prefix(PREFIX)?.to_string()))
            .collect();
        names.sort();
        names
    }

    pub fn load(name: &str) -> Option<String> {
        storage()?.get_item(&format!("{PREFIX}{name}")).ok()?
    }

    pub fn save(name: &str, text: &str) -> Result<(), String> {
        let storage = storage().ok_or("no local storage")?;
        storage
            .set_item(&format!("{PREFIX}{name}"), text)
            .map_err(|_| "the browser refused to store it".to_string())
    }

    pub fn delete(name: &str) {
        if let Some(storage) = storage() {
            let _ = storage.remove_item(&format!("{PREFIX}{name}"));
        }
    }
}

pub use store::{delete, list, load, save};

#[cfg(test)]
mod tests {
    use super::store::*;
    use super::*;

    #[test]
    fn names_are_safe_for_files() {
        assert_eq!(clean_name("  track day ").as_deref(), Some("track day"));
        assert_eq!(clean_name("../etc/passwd").as_deref(), Some("etcpasswd"));
        assert_eq!(clean_name(" /// "), None);
        assert_eq!(
            clean_name(&"x".repeat(100)).map(|n| n.len()),
            Some(NAME_LENGTH)
        );
    }

    #[test]
    fn saves_lists_loads_and_deletes() {
        let dir = std::env::temp_dir().join(format!("car-sim-tunes-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(list_in(&dir).is_empty());
        save_in(&dir, "b tune", "two").unwrap();
        save_in(&dir, "a tune", "one").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a tune").unwrap();
        assert_eq!(list_in(&dir), ["a tune", "b tune"]);
        assert_eq!(load_in(&dir, "b tune").as_deref(), Some("two"));
        delete_in(&dir, "a tune");
        assert_eq!(list_in(&dir), ["b tune"]);
        assert_eq!(load_in(&dir, "a tune"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
