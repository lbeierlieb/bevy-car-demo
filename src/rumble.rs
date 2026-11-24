//! Gamepad rumble when the driven car crashes, on the gamepads the profile in use drives
//! with (never the force feedback wheel: its force is ours). Controllers without motors
//! just don't rumble; no settings. Native only (gilrs has no rumble in browsers).

use std::time::Duration;

use bevy::input::gamepad::{GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;

use crate::audio::effects::CrashGate;
use crate::car::{ActiveCar, Away, Held, Rescued};
use crate::input::{Input, WheelConfig};
use crate::wheel_base::ForceFeedback;

/// The weak (quick) motor's share of the strong one.
const WEAK: f32 = 0.6;
/// How long a crash rumbles (s): the lightest, the hardest.
const LENGTH: (f32, f32) = (0.12, 0.5);

pub struct RumblePlugin;

impl Plugin for RumblePlugin {
    fn build(&self, app: &mut App) {
        // In `Update`: after the simulation's knocks, before the sound takes them.
        if !cfg!(target_arch = "wasm32") {
            app.add_systems(Update, rumble);
        }
    }
}

/// The rumble for a crash of `strength` (0..1).
fn rumble_for(strength: f32) -> (GamepadRumbleIntensity, Duration) {
    let intensity = GamepadRumbleIntensity {
        strong_motor: strength,
        weak_motor: WEAK * strength,
    };
    let length = LENGTH.0 + (LENGTH.1 - LENGTH.0) * strength;
    (intensity, Duration::from_secs_f32(length))
}

#[allow(clippy::too_many_arguments)]
fn rumble(
    time: Res<Time>,
    config: Res<WheelConfig>,
    ffb: Option<Res<ForceFeedback>>,
    active: Res<ActiveCar>,
    (away, held, rescued): (Res<Away>, Res<Held>, Res<Rescued>),
    cars: Query<&crate::car::Car>,
    gamepads: Query<(Entity, &Name), With<Gamepad>>,
    mut requests: MessageWriter<GamepadRumbleRequest>,
    mut gate: Local<CrashGate>,
) {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let quiet = away.0 || held.0 || rescued.0;
    let Some(strength) = gate.hit(car.knocks, time.delta_secs()).filter(|_| !quiet) else {
        return;
    };
    let wheel = ffb.as_ref().and_then(|ffb| ffb.device.as_deref());
    let (intensity, duration) = rumble_for(strength);
    for (gamepad, name) in &gamepads {
        let drives = (config.profile().bindings.iter())
            .any(|(_, b)| matches!(b.input, Input::Pad(_)) && b.is_on(name));
        let is_wheel = wheel.is_some_and(|wheel| wheel.trim() == name.trim());
        if drives && !is_wheel {
            requests.write(GamepadRumbleRequest::Add {
                gamepad,
                intensity,
                duration,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harder_crashes_rumble_harder_and_longer() {
        let (light, short) = rumble_for(0.1);
        let (hard, long) = rumble_for(1.0);
        assert!(hard.strong_motor > light.strong_motor && long > short);
        assert_eq!(hard.strong_motor, 1.0);
        assert!(hard.weak_motor < hard.strong_motor);
        assert!(long.as_secs_f32() <= LENGTH.1 + 1e-6);
    }
}
