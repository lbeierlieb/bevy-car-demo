//! The wheel base itself, on Linux with the hid-fanatecff driver: its rotation range is set
//! to the game's through sysfs, so the wheel's axis covers exactly that range, and set back
//! when the game exits (on the newer bases it's a setting of the active tuning slot). On
//! other systems there's nothing to find, and the range is set on the base by hand.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::input::WheelConfig;

/// Where hid-fanatecff lists its devices; each has a `range` file in degrees.
const DRIVER_DIR: &str = "/sys/module/hid_fanatec/drivers/hid:fanatec";

pub struct WheelBasePlugin;

impl Plugin for WheelBasePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WheelBase>()
            .add_systems(Update, apply_range);
    }
}

#[derive(Resource, Default)]
pub struct WheelBase {
    /// The range files written to, with the value each had before.
    ranges: Vec<(PathBuf, String)>,
    /// The range last set on a base (degrees).
    pub applied: Option<f32>,
}

impl WheelBase {
    /// Writes `range` to every base listed in `driver_dir`, remembering their old ranges.
    fn set_range(&mut self, driver_dir: &Path, range: f32) {
        let Ok(entries) = std::fs::read_dir(driver_dir) else {
            return;
        };
        self.applied = None;
        for entry in entries.flatten() {
            let path = entry.path().join("range");
            let Ok(old) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !self.ranges.iter().any(|(p, _)| *p == path) {
                self.ranges.push((path.clone(), old.trim().to_string()));
            }
            match std::fs::write(&path, format!("{}", range.round() as u32)) {
                Ok(()) => self.applied = Some(range),
                Err(err) => warn!("Couldn't set the wheel range in {}: {err}", path.display()),
            }
        }
    }
}

impl Drop for WheelBase {
    fn drop(&mut self) {
        for (path, old) in &self.ranges {
            let _ = std::fs::write(path, old);
        }
    }
}

/// Sets the range when it's changed (once the slider has settled) and when a wheel
/// connects.
fn apply_range(
    config: Res<WheelConfig>,
    connected: Query<(), Added<Gamepad>>,
    time: Res<Time>,
    mut base: ResMut<WheelBase>,
    mut pending: Local<Option<f32>>,
) {
    if config.is_changed() && base.applied != Some(config.range) || !connected.is_empty() {
        *pending = Some(0.3);
    }
    if let Some(left) = &mut *pending {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            base.set_range(Path::new(DRIVER_DIR), config.range);
            *pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_range_and_restores_it() {
        let dir = std::env::temp_dir().join(format!("car-sim-wheel-{}", std::process::id()));
        let device = dir.join("0003:0EB7:0020.0001");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::write(device.join("range"), "900\n").unwrap();
        let read = || std::fs::read_to_string(device.join("range")).unwrap();

        let mut base = WheelBase::default();
        base.set_range(&dir, 540.0);
        base.set_range(&dir, 720.0);
        assert_eq!(read(), "720");
        assert_eq!(base.applied, Some(720.0));
        // Dropped at exit: back to the range from before the first change.
        drop(base);
        assert_eq!(read(), "900");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
