//! The wheel base itself, on Linux with the hid-fanatecff driver: its rotation range is set
//! to the game's through sysfs, so the wheel's axis covers exactly that range, and set back
//! when the game exits (on the newer bases it's a setting of the active tuning slot). If
//! that isn't allowed, the base's range is still read from there; on other systems it's
//! given in the settings. A base turning farther than the game's range gets a soft lock:
//! force feedback holds the wheel at either end, like in BeamNG.
//!
//! Force feedback (Linux) is one constant-force effect on the wheel's evdev device, updated
//! every chassis step with what the front wheels do to the steering (`sim::steering`), as
//! a share of what the car's tires reach near the limit, so light and heavy cars feel alike.
//! The quick part of it (bumps, tugs from the driven front wheels) can be made stronger
//! than the steady pull ("Road feel"). Damping and friction are left to the base's own
//! tuning menu, which filters at a far higher rate.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::car::{ActiveCar, Car};
use crate::input::{Action, WheelConfig, WheelInput};
use crate::sim::steering;

/// Where hid-fanatecff lists its devices; each has a `range` file in degrees.
const DRIVER_DIR: &str = "/sys/module/hid_fanatec/drivers/hid:fanatec";
/// Time constant (s) of the steering's steady pull: what changes faster than that is the
/// road's detail, scaled by the road feel. Faster than a driver turns the wheel, so turning
/// in doesn't feel heavier for it.
const DETAIL: f32 = 0.03;
/// Seconds for the force to ramp up after the wheel connects, the window gets focus or
/// the car changes, so the wheel never jerks.
const FADE_IN: f32 = 1.0;
/// Fastest the force may change (full scale per second): kerbs, landings and crashes spike
/// the tire forces, and a direct-drive base would pass that on as a hard jolt.
const MAX_SLEW: f32 = 15.0;
/// Speed (m/s) above which the tires' torque comes through in full: at a standstill their
/// slips are just noise.
const ROLLING: f32 = 3.0;
/// Force of the panel's test push, and how long it lasts.
const TEST_LEVEL: f32 = 0.25;
const TEST_TIME: f32 = 0.5;
/// Degrees past the end of the range over which the soft lock builds up to the base's full
/// force.
const SOFT_LOCK_SPAN: f32 = 10.0;

pub struct WheelBasePlugin;

impl Plugin for WheelBasePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WheelBase>()
            .init_resource::<ForceFeedback>()
            .add_systems(Update, apply_range)
            .add_systems(FixedUpdate, force_feedback.after(crate::sim::simulate));
    }
}

#[derive(Resource, Default)]
pub struct WheelBase {
    /// The range files written to, with the value each had before.
    ranges: Vec<(PathBuf, String)>,
    /// The range last set on a base (degrees).
    pub applied: Option<f32>,
    /// The range a base is at after trying to set it (degrees), if one was found.
    pub actual: Option<f32>,
}

impl WheelBase {
    /// Writes `range` to every base listed in `driver_dir`, remembering their old ranges.
    fn set_range(&mut self, driver_dir: &Path, range: f32) {
        let Ok(entries) = std::fs::read_dir(driver_dir) else {
            return;
        };
        self.applied = None;
        self.actual = None;
        for entry in entries.flatten() {
            let path = entry.path().join("range");
            let Ok(old) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !self.ranges.iter().any(|(p, _)| *p == path) {
                self.ranges.push((path.clone(), old.trim().to_string()));
            }
            match std::fs::write(&path, format!("{}", range.round() as u32)) {
                Ok(()) => {
                    self.applied = Some(range);
                    self.actual = Some(range);
                }
                Err(err) => {
                    self.actual = old.trim().parse().ok().or(self.actual);
                    warn!(
                        "Couldn't set the wheel range in {} ({err}), soft lock at {range:.0}° instead",
                        path.display()
                    );
                }
            }
        }
    }

    /// The rotation range the wheel's axis covers (degrees): as found on the base, or else
    /// as given in the settings.
    pub fn range(&self, config: &WheelConfig) -> f32 {
        self.actual.or(config.base_range).unwrap_or(config.range)
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
    mut wanted: Local<Option<f32>>,
) {
    // Compared by value: the open settings panel marks the config changed every frame.
    if *wanted != Some(config.range) || !connected.is_empty() {
        *wanted = Some(config.range);
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

#[derive(Resource, Default)]
pub struct ForceFeedback {
    #[cfg(target_os = "linux")]
    output: Option<linux::Output>,
    /// Name of the device the force goes to, if one was found.
    pub device: Option<String>,
    /// Why there's no force feedback device, if there isn't.
    pub problem: Option<String>,
    /// Force last sent, -1 to 1 of the base's maximum (positive pushes the wheel left).
    pub level: f32,
    /// Soft lock force last added to `level`.
    pub soft_lock: f32,
    /// Time left in a test push from the panel.
    pub test: f32,
    fade: f32,
    /// The steady part of the steering torque (shares of full scale), see `DETAIL`.
    steady: f32,
    /// Time until looking for a device again.
    retry: f32,
    car: usize,
}

impl ForceFeedback {
    /// Pushes the wheel left for a moment, to check the direction.
    pub fn test_push(&mut self) {
        self.test = TEST_TIME;
    }
}

/// Force for a steering torque, as a share of the car's full scale: positive (turning the
/// front wheels left) pushes the wheel left; `fade` ramps it in.
fn level(config: &WheelConfig, share: f32, fade: f32) -> f32 {
    (config.ffb_gain * share * fade).clamp(-1.0, 1.0)
}

/// A steering torque `share` with its quick part, what it is off `steady`, scaled by the
/// road feel.
fn with_road_feel(config: &WheelConfig, share: f32, steady: f32) -> f32 {
    steady + config.ffb_road * (share - steady)
}

/// Force holding the wheel within the rotation range, for a steering wheel `angle` from the
/// centre (degrees, positive left): none inside, then pushing back, up to the base's full
/// force `SOFT_LOCK_SPAN` past either end.
fn soft_lock(angle: f32, range: f32) -> f32 {
    let lock = range / 2.0;
    let past = angle - angle.clamp(-lock, lock);
    -(past / SOFT_LOCK_SPAN).clamp(-1.0, 1.0)
}

fn force_feedback(
    config: Res<WheelConfig>,
    wheel: Res<WheelInput>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    windows: Query<&Window>,
    time: Res<Time>,
    mut ffb: ResMut<ForceFeedback>,
) {
    let dt = time.delta_secs();
    let device = config.binding(Action::Steer).map(|b| b.device.clone());
    if ffb.device != device {
        ffb.retry -= dt;
        if ffb.retry <= 0.0 {
            ffb.retry = 2.0;
            let problem = match device.as_deref() {
                None => None,
                #[cfg(target_os = "linux")]
                Some(name) => match linux::Output::open(name) {
                    Ok(output) => {
                        ffb.output = Some(output);
                        ffb.device = device;
                        None
                    }
                    Err(problem) => Some(problem),
                },
                #[cfg(not(target_os = "linux"))]
                Some(_) => Some("force feedback is only supported on Linux".into()),
            };
            if problem.is_some() && problem != ffb.problem {
                warn!(
                    "No force feedback: {}",
                    problem.as_deref().unwrap_or_default()
                );
            }
            ffb.problem = problem;
            ffb.fade = 0.0;
        }
    }
    let focused = windows.iter().any(|window| window.focused);
    let car = cars.iter().find(|car| car.index == active.0);
    if !focused || car.is_none_or(|car| car.index != ffb.car) {
        ffb.fade = 0.0;
        ffb.car = active.0;
    } else {
        ffb.fade = (ffb.fade + dt / FADE_IN).min(1.0);
    }
    let target = if ffb.test > 0.0 {
        ffb.test -= dt;
        TEST_LEVEL
    } else if config.ffb_enabled
        && let Some(car) = car
    {
        let rolling = (car.body.velocity.length() / ROLLING).min(1.0);
        let share = car.rack.total() * rolling / steering::full_scale(&car.setup);
        ffb.steady = if ffb.fade > 0.0 {
            ffb.steady + (share - ffb.steady) * (dt / DETAIL).min(1.0)
        } else {
            share
        };
        let felt = with_road_feel(&config, share, ffb.steady);
        level(&config, felt, ffb.fade)
    } else {
        0.0
    };
    // Smoothed a little, so the 120 Hz steps don't buzz, and rate-limited.
    let max_step = MAX_SLEW * dt;
    ffb.level += ((target - ffb.level) * 0.5).clamp(-max_step, max_step);
    // The soft lock isn't part of the feedback: it stays on with the tire forces off.
    ffb.soft_lock = wheel
        .angle
        .map_or(0.0, |angle| soft_lock(angle, config.range) * ffb.fade);
    #[cfg(target_os = "linux")]
    {
        let level = (ffb.level + ffb.soft_lock).clamp(-1.0, 1.0);
        let sent = if config.ffb_invert { -level } else { level };
        if let Some(output) = &mut ffb.output
            && let Err(err) = output.set(sent)
        {
            warn!("Force feedback stopped: {err}");
            ffb.output = None;
            ffb.device = None;
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    use evdev::{
        FFEffect, FFEffectCode, FFEffectData, FFEffectKind, FFEnvelope, FFReplay, FFTrigger,
    };

    pub struct Output {
        /// Kept open: the effect lives as long as the device.
        _device: evdev::Device,
        effect: FFEffect,
        sent: i16,
    }

    impl Output {
        /// The first input device with this name that can play a constant force (some wheel
        /// bases register several under one name). Otherwise, what's wrong with each.
        pub fn open(name: &str) -> Result<Self, String> {
            let mut nodes: Vec<String> = std::fs::read_dir("/sys/class/input")
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| {
                    std::fs::read_to_string(entry.path().join("device/name"))
                        .is_ok_and(|device| device.trim() == name.trim())
                })
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|node| node.starts_with("event"))
                .collect();
            nodes.sort();
            let mut problems = Vec::new();
            for node in nodes {
                let path = Path::new("/dev/input").join(&node);
                if let Err(err) = std::fs::OpenOptions::new().write(true).open(&path) {
                    problems.push(format!("{node}: no write access ({err})"));
                    continue;
                }
                let mut device = match evdev::Device::open(&path) {
                    Ok(device) => device,
                    Err(err) => {
                        problems.push(format!("{node}: can't open ({err})"));
                        continue;
                    }
                };
                let constant = device
                    .supported_ff()
                    .is_some_and(|ff| ff.contains(FFEffectCode::FF_CONSTANT));
                if !constant {
                    problems.push(format!("{node}: no constant-force support"));
                    continue;
                }
                let started = device
                    .upload_ff_effect(effect(0))
                    .and_then(|mut effect| effect.play(1).map(|()| effect));
                match started {
                    Ok(effect) => {
                        return Ok(Self {
                            _device: device,
                            effect,
                            sent: 0,
                        });
                    }
                    Err(err) => problems.push(format!("{node}: can't start a force ({err})")),
                }
            }
            if problems.is_empty() {
                Err(format!("no input device named \"{}\"", name.trim()))
            } else {
                Err(problems.join("; "))
            }
        }

        /// Sets the force, -1 to 1.
        pub fn set(&mut self, level: f32) -> std::io::Result<()> {
            let level = (level * i16::MAX as f32) as i16;
            if level != self.sent {
                self.effect.update(effect(level))?;
                self.sent = level;
            }
            Ok(())
        }
    }

    /// A constant force that plays until it's removed (length 0).
    fn effect(level: i16) -> FFEffectData {
        FFEffectData {
            direction: 0x4000,
            trigger: FFTrigger::default(),
            replay: FFReplay {
                length: 0,
                delay: 0,
            },
            kind: FFEffectKind::Constant {
                level,
                // No attack or fade: the level applies as it is.
                envelope: FFEnvelope {
                    attack_length: 0,
                    attack_level: 0,
                    fade_length: 0,
                    fade_level: 0,
                },
            },
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

    #[test]
    fn reads_the_range_it_cant_set() {
        let dir = std::env::temp_dir().join(format!("car-sim-wheel-ro-{}", std::process::id()));
        let device = dir.join("0003:0EB7:0020.0001");
        std::fs::create_dir_all(&device).unwrap();
        let file = device.join("range");
        std::fs::write(&file, "2520\n").unwrap();
        let mut permissions = std::fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&file, permissions).unwrap();
        // Root may write anyway; there's nothing to check then.
        if std::fs::OpenOptions::new().write(true).open(&file).is_err() {
            let mut base = WheelBase::default();
            base.set_range(&dir, 540.0);
            assert_eq!(base.applied, None);
            assert_eq!(base.actual, Some(2520.0));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn force_follows_the_tires_and_fades_in() {
        let config = WheelConfig {
            ffb_gain: 0.5,
            ..default()
        };
        // Turning left, the tires pull the wheels back to the right: the wheel is pushed
        // right, and harder the more they pull, up to the base's maximum.
        assert!(level(&config, -0.2, 1.0) < 0.0);
        assert!(level(&config, -0.4, 1.0) < level(&config, -0.2, 1.0));
        assert_eq!(level(&config, -10.0, 1.0), -1.0);
        assert_eq!(level(&config, -0.2, 0.0), 0.0);
    }

    #[test]
    fn road_feel_scales_the_quick_part_only() {
        let config = WheelConfig {
            ffb_road: 3.0,
            ..default()
        };
        // Steady, it's as the tires make it; a kick off the steady pull comes through
        // three times as strong.
        assert_eq!(with_road_feel(&config, -0.3, -0.3), -0.3);
        assert!((with_road_feel(&config, -0.2, -0.3) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn soft_lock_holds_the_wheel_in_the_range() {
        assert_eq!(soft_lock(269.0, 540.0), 0.0);
        assert_eq!(soft_lock(-269.0, 540.0), 0.0);
        // Past the left end it pushes right, and the other way round.
        assert!((soft_lock(275.0, 540.0) + 0.5).abs() < 1e-6);
        assert!((soft_lock(-275.0, 540.0) - 0.5).abs() < 1e-6);
        assert_eq!(soft_lock(900.0, 540.0), -1.0);
    }

    #[test]
    fn falls_back_to_the_base_range_from_the_settings() {
        let mut config = WheelConfig::default();
        let mut base = WheelBase::default();
        assert_eq!(base.range(&config), 540.0);
        config.base_range = Some(1080.0);
        assert_eq!(base.range(&config), 1080.0);
        base.actual = Some(2520.0);
        assert_eq!(base.range(&config), 2520.0);
    }
}
