//! Saved ECU tunes, by name: text files in the config directory
//! (`~/.config/car-sim/tunes/<name>.tune`), or the browser's local storage on the web.

use crate::storage::Folder;

/// Longest tune name.
const NAME_LENGTH: usize = 40;

const TUNES: Folder = Folder {
    cache: false,
    name: "tunes",
    extension: "tune",
    web_prefix: "car-sim-tune:",
};

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

pub fn list() -> Vec<String> {
    TUNES.list()
}

pub fn load(name: &str) -> Option<String> {
    TUNES.load(name)
}

pub fn save(name: &str, text: &str) -> Result<(), String> {
    TUNES.save(name, text)
}

pub fn delete(name: &str) {
    TUNES.delete(name);
}

#[cfg(test)]
mod tests {
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
}
