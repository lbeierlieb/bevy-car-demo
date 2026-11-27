//! Brakes as parts: each corner's brake has a torque (at the pedal floored, with the
//! alignment shop's force at 100 %), a heat capacity and cooling, a temperature its pads
//! start to fade at, and pads that wear with the energy they take (faster hot). A brake
//! gets worse before it fails: hot, it fades; worn, its pads grip less, and worn through,
//! metal on metal, it barely stops the car.

use super::engine::AMBIENT_TEMPERATURE;

/// How far over its fade temperature a brake has lost the most it loses (K), and what's
/// left then (a share of its torque).
const FADE_SPAN: f32 = 200.0;
const FADED: f32 = 0.45;
/// Worn past this the pads grip less, down to `WORN` just before they're through; worn
/// through, metal on metal, `THROUGH` is left.
const WEARING: f32 = 0.8;
const WORN: f32 = 0.6;
const THROUGH: f32 = 0.3;
/// The pads wear this much faster per this many kelvins over the fade temperature.
const HOT_WEAR: f32 = 100.0;

/// What a brake is (a part's, see `items::Does::Brake`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brake {
    /// Torque at the wheel with the pedal floored (N·m).
    pub torque: f32,
    /// The smallest rim that clears it (inches).
    pub min_rim: u8,
    /// Heat capacity of the disc or drum (J/K).
    pub heat: f32,
    /// Heat it gives off per kelvin over the air, standing and more per m/s (W/K).
    pub cooling: [f32; 2],
    /// Temperature the pads start to fade at (K).
    pub fade: f32,
    /// Energy the pads take until they're worn through (MJ).
    pub life: f32,
}

/// A brake as it's used: hot or cold, its pads worn or new.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrakeState {
    pub temperature: f32,
    /// 0 new, 1 worn through.
    pub wear: f32,
    /// What the pads hold the wheel back with this step (N·m), for the heat.
    pub torque: f32,
}

impl Default for BrakeState {
    fn default() -> Self {
        Self {
            temperature: AMBIENT_TEMPERATURE,
            wear: 0.0,
            torque: 0.0,
        }
    }
}

impl BrakeState {
    /// The share of its torque the brake gives as it is: less hot (fading), less worn.
    pub fn grip(&self, brake: &Brake) -> f32 {
        let over = ((self.temperature - brake.fade) / FADE_SPAN).clamp(0.0, 1.0);
        let fade = 1.0 - (1.0 - FADED) * over;
        let pads = if self.wear >= 1.0 {
            THROUGH
        } else if self.wear > WEARING {
            1.0 - (1.0 - WORN) * (self.wear - WEARING) / (1.0 - WEARING)
        } else {
            1.0
        };
        fade * pads
    }

    /// Advances by `dt` with the wheel turning at `omega` (rad/s) and the car moving at
    /// `speed` (m/s, for the cooling): the pads' work heats the disc and wears them.
    pub fn step(&mut self, brake: &Brake, omega: f32, speed: f32, dt: f32) {
        let power = self.torque.abs() * omega.abs();
        let cooling = (brake.cooling[0] + brake.cooling[1] * speed.abs())
            * (self.temperature - AMBIENT_TEMPERATURE);
        self.temperature += (power - cooling) * dt / brake.heat;
        let hot = 1.0 + ((self.temperature - brake.fade) / HOT_WEAR).max(0.0);
        self.wear = (self.wear + power * dt * hot / (brake.life * 1e6)).min(1.0);
    }
}

crate::snapshot::snapshot!(BrakeState {
    temperature,
    wear,
    torque
});

#[cfg(test)]
mod tests {
    use super::*;

    fn brake() -> Brake {
        Brake {
            torque: 1000.0,
            min_rim: 14,
            heat: 3000.0,
            cooling: [10.0, 2.0],
            fade: 650.0,
            life: 100.0,
        }
    }

    #[test]
    fn a_hot_brake_fades_and_a_worn_one_grips_less() {
        let brake = brake();
        let cold = BrakeState::default();
        assert_eq!(cold.grip(&brake), 1.0);
        let hot = BrakeState {
            temperature: 800.0,
            ..cold
        };
        assert!(hot.grip(&brake) < 0.7);
        let worn = BrakeState { wear: 0.9, ..cold };
        let through = BrakeState { wear: 1.0, ..cold };
        assert!(worn.grip(&brake) < 1.0 && through.grip(&brake) < worn.grip(&brake));
    }

    #[test]
    fn braking_heats_the_disc_and_the_air_cools_it() {
        let brake = brake();
        let mut state = BrakeState {
            torque: 1000.0,
            ..Default::default()
        };
        for _ in 0..120 {
            state.step(&brake, 50.0, 15.0, 1.0 / 120.0);
        }
        let hot = state.temperature;
        assert!(hot > AMBIENT_TEMPERATURE + 10.0, "{hot} K");
        assert!(state.wear > 0.0);
        state.torque = 0.0;
        for _ in 0..1200 {
            state.step(&brake, 50.0, 15.0, 1.0 / 120.0);
        }
        assert!(state.temperature < hot);
    }
}
