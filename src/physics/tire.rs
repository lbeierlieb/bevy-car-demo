//! Simplified Pacejka "Magic Formula" tire with combined slip, and the tread's temperature
//! and wear.
//!
//! Each tire is of a compound, which sets how the grip builds and falls off with slip and
//! how it depends on the tread's temperature. The tread heats up with the power of the
//! rubber sliding over the road and with the carcass flexing as it rolls, and cools in the
//! air going past; it wears with the sliding, faster when it's overheated.

use std::f32::consts::FRAC_PI_2;

/// Below this speed, slips are computed against this speed instead, which keeps the
/// forces bounded and stable when the car is (nearly) standing still.
const LOW_SPEED: f32 = 3.0;
/// Air and road temperature (°C): where the tread starts and cools towards.
pub const AMBIENT: f32 = 20.0;
/// Heat capacity of the tread (J/K): how fast it warms up and cools down.
const TREAD_CAPACITY: f32 = 2500.0;
/// Share of the sliding power that goes into the tread (the rest heats the road).
const SLIDING_HEAT: f32 = 0.8;
/// Power the carcass turns into heat as it flexes rolling: this share of the load times
/// the speed (the rolling resistance).
const HYSTERESIS: f32 = 0.012;
/// Heat the tread loses to the air per kelvin above it (W/K): standing, and growing with
/// the square root of the speed (per √(m/s)).
const COOLING: f32 = 15.0;
const COOLING_SPEED: f32 = 8.0;
/// Peak friction of street tires at the load they're made for (see `TireParams::mu`).
pub const GRIP: f32 = 1.05;
/// Grip lost with the tread worn through.
const WORN_GRIP: f32 = 0.25;

/// A tire's rubber and construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compound {
    pub name: &'static str,
    pub about: &'static str,
    /// Grip at its best temperature, relative to the chassis' street tires.
    pub grip: f32,
    /// Slip angle (rad) and slip ratio where the grip peaks, and the Magic Formula shapes:
    /// the higher, the more the grip drops off past the peak.
    pub peak_angle: f32,
    pub peak_ratio: f32,
    pub c_lat: f32,
    pub c_long: f32,
    /// Tread temperature with the most grip (°C), the share of that grip at `AMBIENT`, and
    /// how far past the best temperature the grip has dropped by a tenth (K).
    pub best: f32,
    pub cold: f32,
    pub overheat: f32,
    /// Sliding energy (J) that wears the tread through at its best temperature.
    pub life: f32,
}

impl Compound {
    /// Share of the grip at the best temperature the tread has at `temperature` (°C):
    /// falling off smoothly on either side.
    pub fn warmth(&self, temperature: f32) -> f32 {
        if temperature < self.best {
            let below = ((self.best - temperature) / (self.best - AMBIENT)).min(1.5);
            (1.0 - (1.0 - self.cold) * below * below).max(0.5 * self.cold)
        } else {
            let above = (temperature - self.best) / self.overheat;
            (1.0 - 0.1 * above * above).max(0.5)
        }
    }

    /// How much faster than at its best temperature the tread wears at `temperature`.
    fn wear_rate(&self, temperature: f32) -> f32 {
        let above = ((temperature - self.best) / self.overheat).max(0.0);
        1.0 + above * above
    }
}

/// The compounds by their place in `COMPOUNDS`.
pub const TOURING: usize = 0;
pub const STREET: usize = 1;
pub const SPORT: usize = 2;
pub const RACE: usize = 3;

/// The tires in the catalogue, by their place.
pub const COMPOUNDS: [Compound; 4] = [
    Compound {
        name: "Touring",
        about: "Made to last and to roll easily: grips from cold and never overheats on the \
                road, but has the least grip and lets go early",
        grip: 0.9,
        peak_angle: 0.2,
        peak_ratio: 0.13,
        c_lat: 1.3,
        c_long: 1.45,
        best: 50.0,
        cold: 0.95,
        overheat: 25.0,
        life: 100e6,
    },
    Compound {
        name: "Street",
        about: "Grips from cold and lasts, and lets go gently past the limit, but overheats \
                when driven hard for long",
        grip: 1.0,
        peak_angle: 0.183,
        peak_ratio: 0.12,
        c_lat: 1.35,
        c_long: 1.5,
        best: 65.0,
        cold: 0.93,
        overheat: 30.0,
        life: 60e6,
    },
    Compound {
        name: "Sport",
        about: "More grip and sharper, with a higher working temperature and faster wear",
        grip: 1.1,
        peak_angle: 0.131,
        peak_ratio: 0.11,
        c_lat: 1.45,
        c_long: 1.6,
        best: 80.0,
        cold: 0.82,
        overheat: 30.0,
        life: 25e6,
    },
    Compound {
        name: "Race",
        about: "The most grip once hot, little when cold; peaks early and drops off past \
                it, and wears fast",
        grip: 1.25,
        peak_angle: 0.105,
        peak_ratio: 0.09,
        c_lat: 1.65,
        c_long: 1.75,
        best: 95.0,
        cold: 0.68,
        overheat: 35.0,
        life: 8e6,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct TireParams {
    pub radius: f32,
    /// Vertical stiffness (N/m) and damping (N·s/m) of the tire carcass.
    pub stiffness: f32,
    pub damping: f32,
    pub compound: Compound,
    /// Peak friction coefficient at the nominal load and the best temperature, new: the
    /// Magic Formula `D = mu · Fz`.
    pub mu: f32,
    /// Load (N) at which the friction is `mu` (what the tire's size is made for), and how
    /// it falls off with more: `mu · (Fz / nominal)^-load_sensitivity`. A loaded tire grips
    /// harder, but less than in proportion, so weight shifting across an axle costs it grip,
    /// and a wider tire grips better under the same load.
    pub nominal_load: f32,
    pub load_sensitivity: f32,
    /// How far behind the contact centre the lateral force acts at small slip angles (m).
    pub pneumatic_trail: f32,
}

impl Default for TireParams {
    fn default() -> Self {
        Self {
            radius: 0.33,
            stiffness: 200_000.0,
            damping: 1_500.0,
            compound: COMPOUNDS[STREET],
            mu: GRIP,
            nominal_load: 3000.0,
            load_sensitivity: 0.27,
            pneumatic_trail: 0.04,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TireForce {
    /// Longitudinal force along the tire's forward direction.
    pub fx: f32,
    /// Lateral force along the tire's right direction.
    pub fy: f32,
    pub slip_ratio: f32,
    pub slip_angle: f32,
}

/// The tread's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tread {
    /// Temperature (°C).
    pub temperature: f32,
    /// How far it's worn, from 0 (new) to 1 (worn through).
    pub wear: f32,
}

impl Default for Tread {
    fn default() -> Self {
        Self {
            temperature: AMBIENT,
            wear: 0.0,
        }
    }
}

/// The pure-slip Magic Formula (with `E = 0`): `D · sin(C · atan(B · s))`.
fn magic_formula(slip: f32, b: f32, c: f32, d: f32) -> f32 {
    d * (c * (b * slip).atan()).sin()
}

/// Magic Formula stiffness `B` that puts the peak at `peak`: `C · atan(B · peak) = π/2`.
fn stiffness_for_peak(peak: f32, c: f32) -> f32 {
    (FRAC_PI_2 / c).tan() / peak
}

impl TireParams {
    /// Peak friction coefficient under a load of `fz` (N), at the best temperature, new.
    pub fn friction(&self, fz: f32) -> f32 {
        // Light loads (a wheel lifting) don't grip without bound.
        let relative = (fz / self.nominal_load).max(0.25);
        self.mu * relative.powf(-self.load_sensitivity)
    }

    /// Share of the best grip the tread has, warm or cold and worn as it is.
    pub fn condition(&self, tread: &Tread) -> f32 {
        self.compound.warmth(tread.temperature) * (1.0 - WORN_GRIP * tread.wear.min(1.0))
    }

    /// Pneumatic trail at a slip: it shrinks as more of the contact patch slides and is
    /// gone at the peak slip angle, so the self-aligning torque (lateral force times trail)
    /// fades before the grip does. That's what makes the steering go light near the limit.
    pub fn trail(&self, force: &TireForce) -> f32 {
        let slip = (force.slip_angle / self.compound.peak_angle).abs();
        self.pneumatic_trail * (1.0 - slip).max(0.0)
    }

    /// Tire forces for a contact patch moving at (`vx` forward, `vy` right) with the wheel
    /// spinning at `omega` (rad/s, positive rolls forward) under normal load `fz`, with
    /// `condition` of its best grip (see `condition`).
    pub fn force(&self, vx: f32, vy: f32, omega: f32, fz: f32, condition: f32) -> TireForce {
        let denom = vx.abs().max(LOW_SPEED);
        let slip_ratio = (omega * self.radius - vx) / denom;
        let slip_angle = (vy / denom).atan();

        // Combined slip: normalise both slips by their peak, so that the total force follows
        // the direction of the combined slip vector and saturates along a friction ellipse.
        let compound = &self.compound;
        let (peak_long, peak_lat) = (compound.peak_ratio, compound.peak_angle);
        let sx = slip_ratio / peak_long;
        let sy = slip_angle / peak_lat;
        let rho = sx.hypot(sy);
        if rho < 1e-6 {
            return TireForce {
                slip_ratio,
                slip_angle,
                ..Default::default()
            };
        }
        let d = condition * self.friction(fz) * fz;
        let (c_long, c_lat) = (compound.c_long, compound.c_lat);
        let b_long = stiffness_for_peak(peak_long, c_long);
        let b_lat = stiffness_for_peak(peak_lat, c_lat);
        let fx = sx / rho * magic_formula(rho * peak_long, b_long, c_long, d);
        // Lateral force opposes sideways sliding.
        let fy = -sy / rho * magic_formula(rho * peak_lat, b_lat, c_lat, d);
        TireForce {
            fx,
            fy,
            slip_ratio,
            slip_angle,
        }
    }

    /// Advances the tread by `dt`: heated by `force` with the rubber sliding at (`slide_x`
    /// along, `slide_y` across, m/s) under `load` (N) rolling at `speed` (m/s), cooled by
    /// the air, and worn by the sliding.
    #[allow(clippy::too_many_arguments)]
    pub fn heat(
        &self,
        tread: &mut Tread,
        force: &TireForce,
        slide_x: f32,
        slide_y: f32,
        load: f32,
        speed: f32,
        dt: f32,
    ) {
        let sliding = (force.fx * slide_x).abs() + (force.fy * slide_y).abs();
        let heating = SLIDING_HEAT * sliding + HYSTERESIS * load * speed.abs();
        tread.temperature += self.cooling(tread, speed, heating) * dt / TREAD_CAPACITY;
        let rate = self.compound.wear_rate(tread.temperature);
        tread.wear = (tread.wear + sliding * rate * dt / self.compound.life).min(1.0);
    }

    /// Heat flowing into the tread (W) with `heating` and the air going past at `speed`.
    fn cooling(&self, tread: &Tread, speed: f32, heating: f32) -> f32 {
        let conductance = COOLING + COOLING_SPEED * speed.abs().sqrt();
        heating - conductance * (tread.temperature - AMBIENT)
    }

    /// The tread cooling in the air with the wheel off the ground.
    pub fn cool(&self, tread: &mut Tread, speed: f32, dt: f32) {
        tread.temperature += self.cooling(tread, speed, 0.0) * dt / TREAD_CAPACITY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_force_equals_mu_times_load() {
        for compound in COMPOUNDS {
            let tire = TireParams {
                compound,
                ..TireParams::default()
            };
            let peak = compound.peak_ratio;
            let f = tire.force(20.0, 0.0, 20.0 * (1.0 + peak) / tire.radius, 4000.0, 1.0);
            assert!((f.fx - tire.friction(4000.0) * 4000.0).abs() < 1.0);
            assert!(f.fy.abs() < 1e-3);
        }
    }

    #[test]
    fn combined_slip_stays_inside_friction_circle() {
        let tire = TireParams::default();
        let f = tire.force(20.0, 5.0, 30.0 / tire.radius, 4000.0, 1.0);
        assert!(f.fx > 0.0 && f.fy < 0.0);
        assert!(f.fx.hypot(f.fy) <= tire.friction(4000.0) * 4000.0 + 1.0);
    }

    #[test]
    fn more_load_grips_harder_but_less_than_in_proportion() {
        let tire = TireParams::default();
        assert_eq!(tire.friction(tire.nominal_load), tire.mu);
        let (light, heavy) = (tire.nominal_load * 0.5, tire.nominal_load * 1.5);
        assert!(tire.friction(light) > tire.mu && tire.friction(heavy) < tire.mu);
        // Weight moving from one side of an axle to the other costs it grip.
        let axle =
            |left: f32, right: f32| left * tire.friction(left) + right * tire.friction(right);
        let level = axle(tire.nominal_load, tire.nominal_load);
        assert!(axle(light, heavy) < 0.99 * level);
    }

    /// Sideways grip at `angle` (rad) relative to the peak's.
    fn lateral(compound: Compound, angle: f32) -> f32 {
        let tire = TireParams {
            compound,
            ..TireParams::default()
        };
        let at = |angle: f32| {
            let v = 20.0;
            -tire
                .force(v, v * angle.tan(), v / tire.radius, 4000.0, 1.0)
                .fy
        };
        at(angle) / at(compound.peak_angle)
    }

    #[test]
    fn street_tires_let_go_gently_and_race_tires_drop_off() {
        let [touring, street, sport, race] = COMPOUNDS;
        assert!(touring.grip < street.grip && touring.peak_angle > street.peak_angle);
        // Twice the peak slip angle, and sliding at 45°.
        for angle in [2.0, 0.0] {
            let at = |compound: Compound| {
                let angle = if angle > 0.0 {
                    angle * compound.peak_angle
                } else {
                    0.785
                };
                lateral(compound, angle)
            };
            assert!(at(street) > at(sport) && at(sport) > at(race));
        }
        assert!(lateral(street, 0.785) > 0.9 && lateral(race, 0.785) < 0.8);
        // Race tires peak earliest.
        assert!(race.peak_angle < sport.peak_angle && sport.peak_angle < street.peak_angle);
    }

    #[test]
    fn grip_by_temperature() {
        let [touring, street, sport, race] = COMPOUNDS;
        // Touring tires are at their best soonest.
        assert!(touring.best < street.best && touring.cold > street.cold);
        for compound in COMPOUNDS {
            assert_eq!(compound.warmth(compound.best), 1.0);
            assert!((compound.warmth(AMBIENT) - compound.cold).abs() < 1e-6);
            // A tenth less at `overheat` past the best, and more beyond.
            let hot = compound.best + compound.overheat;
            assert!((compound.warmth(hot) - 0.9).abs() < 1e-6);
            assert!(compound.warmth(hot + 20.0) < 0.8);
        }
        // Cold, street tires grip the most; hot, race tires do.
        let grip = |compound: Compound, temperature| compound.grip * compound.warmth(temperature);
        assert!(grip(street, AMBIENT) > grip(sport, AMBIENT));
        assert!(grip(sport, AMBIENT) > grip(race, AMBIENT));
        assert!(grip(race, 95.0) > grip(sport, 95.0) && grip(sport, 95.0) > grip(street, 95.0));
    }

    #[test]
    fn sliding_heats_and_wears_the_tread_and_the_air_cools_it() {
        let tire = TireParams::default();
        let mut tread = Tread::default();
        let force = TireForce {
            fx: 0.0,
            fy: -4000.0,
            slip_ratio: 0.0,
            slip_angle: 0.1,
        };
        // Sliding sideways at 2 m/s for 10 s heats it up and wears it a little.
        for _ in 0..1000 {
            tire.heat(&mut tread, &force, 0.0, 2.0, 4000.0, 20.0, 0.01);
        }
        let hot = tread.temperature;
        assert!(hot > AMBIENT + 5.0, "{tread:?}");
        assert!(tread.wear > 0.0 && tread.wear < 0.01, "{tread:?}");
        // Rolling straight, it cools down, and it doesn't wear.
        let rolling = TireForce::default();
        let worn = tread.wear;
        for _ in 0..30000 {
            tire.heat(&mut tread, &rolling, 0.0, 0.0, 4000.0, 20.0, 0.01);
        }
        assert!(tread.temperature < hot - 5.0, "{tread:?}");
        assert_eq!(tread.wear, worn);
        // Worn through, it grips a quarter less.
        tread.wear = 1.0;
        let warm = tire.compound.warmth(tread.temperature);
        assert!((tire.condition(&tread) - 0.75 * warm).abs() < 1e-6);
    }
}
