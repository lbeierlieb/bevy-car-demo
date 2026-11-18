//! Simplified Pacejka "Magic Formula" tire with combined slip and camber, its contact patch
//! across the width, and the rubber's temperatures, the inflation pressure and wear.
//!
//! Each tire is of a compound, which sets how the grip builds and falls off with slip and
//! how it depends on the rubber's temperature. The tread is five ribs across, from the
//! inside to the outside, each carrying its own share of the load (set by camber, by the
//! carcass rolling under in a corner and by the pressure) and gripping by its own load,
//! surface temperature and wear.
//!
//! Heat is in three layers. The surface of each rib is thin: the rubber sliding over the
//! road heats it within moments, and it cools as quickly into the rubber below, the road
//! and the air, so a spin or a burnout flashes it hot without cooking the tire. The core
//! of each rib (the bulk of the tread) warms with the rubber flexing under load as it rolls
//! and with heat soaking in from the surface: that takes steady driving, and lasts. The
//! carcass and the air inside warm with the sidewalls flexing and from the tread, and set
//! the pressure. Grip follows the surface temperature.

use std::f32::consts::FRAC_PI_2;

/// Below this speed (m/s) the rubber damps the patch's motion, fading in from nothing, by
/// slipping it as far as this many seconds of the motion would, along and across (the
/// tread is much stiffer along, and needs less).
const LOW_DAMPING_SPEED: f32 = 4.0;
const LOW_DAMPING: [f32; 2] = [0.01, 0.03];
/// Relaxation length (m) of the reference tire under its design load at the design
/// pressure: how far it rolls for its sideways force to catch up with a new slip angle.
pub const RELAXATION: f32 = 0.5;
/// The relaxation length along the tire (spinning up or braking) as a share of the one
/// across: the tread is much stiffer that way.
const ALONG: f32 = 0.15;
/// The shortest relaxation length, sliding, as a share of the full one.
const SLIDING_LAG: f32 = 0.1;
/// Air and road temperature (°C): where the tire starts and cools towards, and where its
/// cold pressure is set.
pub const AMBIENT: f32 = 20.0;
/// Ribs across the tread, from the inside to the outside.
pub const RIBS: usize = 5;
/// Each rib's middle across the tread, from -1 (the inside edge) to 1 (the outside).
pub const RIB_AT: [f32; RIBS] = [-0.8, -0.4, 0.0, 0.4, 0.8];
/// Tread area (width times circumference, m²) of the tire the thermal constants and the
/// compounds' tread life are for; a bigger tread warms and cools slower in proportion,
/// and lasts longer.
const REFERENCE_TREAD: f32 = 0.44;
/// Heat capacities of the reference tire (J/K): the surface of the whole tread, the core
/// of the whole tread, and the carcass with the air inside.
const SURFACE_CAPACITY: f32 = 150.0;
const CORE_CAPACITY: f32 = 2000.0;
const CARCASS_CAPACITY: f32 = 4000.0;
/// Heat flow of the reference tire per kelvin of difference (W/K): surface into core (rubber
/// conducts poorly: the core warms mostly by flexing), into the road (sliding fast over it,
/// under the average rib's load; more under more), into the air (standing, and per √(m/s)
/// of speed); core into the carcass; core between neighbouring ribs (through the belts);
/// carcass into the air and the rim (standing, and per √(m/s)).
const SURFACE_CORE: f32 = 60.0;
const SURFACE_ROAD: f32 = 150.0;
const SURFACE_AIR: f32 = 15.0;
const SURFACE_AIR_SPEED: f32 = 8.0;
const CORE_CARCASS: f32 = 40.0;
/// Rolling, the patch stays on the same bit of road while it touches it and passes little
/// heat; sliding, it keeps meeting cool road: the road takes `SURFACE_ROAD` times `s² / (s²
/// + this²)` at a sliding speed `s` (m/s).
const ROAD_SLIDE: f32 = 4.0;
const CORE_RIB: f32 = 3.0;
const CARCASS_AIR: f32 = 6.0;
const CARCASS_AIR_SPEED: f32 = 3.0;
/// How the sliding power is shared between the tread's surface and the road: the faster the
/// road slides under the patch, the more of it the road takes (the tread gets `1 / (1 + this
/// · √(m/s))`).
const SLIDING_SHARE: f32 = 0.2;
/// The rubber flexing as it rolls turns the rolling resistance's power into heat, more the
/// harder the tire is pushed sideways or along (per newton of grip force, as per newton of
/// load); this share of it heats the carcass, the rest the tread's core.
const FLEX_FORCE: f32 = 1.0;
const FLEX_CARCASS: f32 = 0.3;
/// Rolling resistance of a street tire (the share of its load that holds it back); the
/// power it takes is what heats the rubber as it flexes.
pub const ROLLING: f32 = 0.012;
/// Peak friction of street tires at the load they're made for (see `TireParams::mu`).
pub const GRIP: f32 = 1.05;
/// Grip lost with the tread worn through.
const WORN_GRIP: f32 = 0.25;
/// Inflation pressure (bar) the carcass figures are for, at the tire's design load; and
/// the pressure the sidewalls' own stiffness is worth: the tire's stiffness follows the
/// pressure plus this.
pub const DESIGN_PRESSURE: f32 = 2.5;
const SIDEWALL_PRESSURE: f32 = 0.6;
/// How the pressure a load wants grows with the load (`(load / design)^this`), and how
/// strongly a tire under- or over-inflated for its load carries more on its shoulders or its
/// middle (share of the deflection per share off the right pressure).
const PRESSURE_LOAD: f32 = 0.5;
const CROWN: f32 = 1.0;
/// How much a stiffer tire peaks at a smaller slip angle, and rolls easier.
const PRESSURE_PEAK: f32 = 0.4;
const PRESSURE_ROLLING: f32 = 0.5;
/// How much of a tilt across the tread (camber against the road) reaches the ribs: the
/// belts are stiff across and spread the load.
const BELT: f32 = 0.6;
/// How far the carcass rolls under (rad) with a sideways force as big as the design load,
/// at the design pressure: it moves the load towards the shoulder the force pushes away
/// from, like leaning the wheel out of the corner.
const ROLL_UNDER: f32 = 0.04;
/// Camber thrust: a leaning wheel pushes towards the side it leans to as if it slipped
/// sideways by this share of the lean.
const CAMBER_SLIP: f32 = 0.06;

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
    /// Surface temperature with the most grip (°C), the share of that grip at `AMBIENT`,
    /// and how far past the best temperature the grip has dropped by a tenth (K).
    pub best: f32,
    pub cold: f32,
    pub overheat: f32,
    /// Sliding energy (J) that wears the reference tread through at its best temperature.
    pub life: f32,
    /// Rolling resistance relative to a street tire's.
    pub rolling: f32,
}

impl Compound {
    /// Share of the grip at the best temperature the rubber has at `temperature` (°C):
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
        best: 55.0,
        cold: 0.95,
        overheat: 35.0,
        life: 100e6,
        rolling: 0.8,
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
        rolling: 1.0,
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
        rolling: 1.1,
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
        rolling: 1.25,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct TireParams {
    pub radius: f32,
    /// Tread width (m).
    pub width: f32,
    /// Vertical stiffness (N/m) and damping (N·s/m) of the tire at `DESIGN_PRESSURE`.
    pub stiffness: f32,
    pub damping: f32,
    pub compound: Compound,
    /// Peak friction coefficient at the nominal load and the best temperature, new: the
    /// Magic Formula `D = mu · Fz`.
    pub mu: f32,
    /// Load (N) at which the friction is `mu` (what the tire's size is made for), and how
    /// it falls off with more: `mu · (Fz / nominal)^-load_sensitivity`. A loaded tire grips
    /// harder, but less than in proportion, so weight shifting across an axle (or across
    /// the tread) costs it grip, and a wider tire grips better under the same load.
    pub nominal_load: f32,
    pub load_sensitivity: f32,
    /// How far behind the contact centre the lateral force acts at small slip angles (m).
    pub pneumatic_trail: f32,
    /// How far the sidewall lets the tread twist: the compound's peak slip angle is scaled
    /// by it (a taller sidewall peaks later).
    pub sidewall: f32,
    /// Tread area: width times circumference (m²).
    pub tread_area: f32,
    /// Rolling resistance at `DESIGN_PRESSURE`: the share of the load that holds the
    /// wheel back.
    pub rolling: f32,
    /// Inflation pressure when cold (bar, at `AMBIENT`).
    pub inflation: f32,
    /// Relaxation length under the design load at the design pressure (m).
    pub relaxation: f32,
}

impl Default for TireParams {
    fn default() -> Self {
        Self {
            radius: 0.33,
            width: 0.215,
            stiffness: 200_000.0,
            damping: 1_500.0,
            compound: COMPOUNDS[STREET],
            mu: GRIP,
            nominal_load: 3000.0,
            load_sensitivity: 0.27,
            pneumatic_trail: 0.04,
            sidewall: 1.0,
            tread_area: REFERENCE_TREAD,
            rolling: ROLLING,
            inflation: DESIGN_PRESSURE,
            relaxation: RELAXATION,
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

/// How far the tire rolls to take up its slips (see `TireParams::relaxation`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Relaxation {
    /// Along and across (m), shorter as the tire saturates.
    pub lengths: [f32; 2],
    /// Along and across (m), the carcass' own.
    pub carcass: [f32; 2],
}

/// How the tire meets the road: what each rib carries and grips.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Patch {
    /// Each rib's load (N), inside to outside.
    pub loads: [f32; RIBS],
    /// Each rib's peak grip force (N): the Magic Formula `D`, warm or cold and worn as it
    /// is.
    pub grip: [f32; RIBS],
    /// How far the wheel leans towards its right against the road (rad).
    pub camber: f32,
    /// The peak slip angle relative to the tire's at `DESIGN_PRESSURE`.
    pub peak: f32,
}

impl Patch {
    pub fn load(&self) -> f32 {
        self.loads.iter().sum()
    }

    pub fn total_grip(&self) -> f32 {
        self.grip.iter().sum()
    }
}

/// The tire's state: its rubber's temperatures and wear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tread {
    /// Each rib's surface and core temperature (°C), inside to outside.
    pub surface: [f32; RIBS],
    pub core: [f32; RIBS],
    /// The carcass and the air inside (°C).
    pub carcass: f32,
    /// How far each rib is worn, from 0 (new) to 1 (worn through).
    pub wear: [f32; RIBS],
}

impl Default for Tread {
    fn default() -> Self {
        Self::at(AMBIENT)
    }
}

impl Tread {
    /// A new tire at `temperature` (°C) through and through.
    pub fn at(temperature: f32) -> Self {
        Self {
            surface: [temperature; RIBS],
            core: [temperature; RIBS],
            carcass: temperature,
            wear: [0.0; RIBS],
        }
    }

    /// The tread's average core temperature (°C).
    pub fn core_temperature(&self) -> f32 {
        self.core.iter().sum::<f32>() / RIBS as f32
    }

    /// The tread's average surface temperature (°C).
    pub fn surface_temperature(&self) -> f32 {
        self.surface.iter().sum::<f32>() / RIBS as f32
    }

    /// How far the tread is worn on average.
    pub fn wear(&self) -> f32 {
        self.wear.iter().sum::<f32>() / RIBS as f32
    }

    /// Wears every rib to `wear`.
    pub fn set_wear(&mut self, wear: f32) {
        self.wear = [wear; RIBS];
    }

    /// As warm as after a few laps at its best (°C): the tread at `best`, the carcass a
    /// little below.
    pub fn warm(&mut self, best: f32) {
        self.surface = [best; RIBS];
        self.core = [best; RIBS];
        self.carcass = best - 0.25 * (best - AMBIENT);
    }

    /// This tire warmed (see `warm`).
    fn warmed(mut self, best: f32) -> Self {
        self.warm(best);
        self
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

    /// Share of its best grip a rib has, warm or cold and worn as it is.
    pub fn rib_condition(&self, tread: &Tread, rib: usize) -> f32 {
        self.compound.warmth(tread.surface[rib]) * (1.0 - WORN_GRIP * tread.wear[rib].min(1.0))
    }

    /// Share of the best grip the tread has on average.
    pub fn condition(&self, tread: &Tread) -> f32 {
        (0..RIBS)
            .map(|rib| self.rib_condition(tread, rib))
            .sum::<f32>()
            / RIBS as f32
    }

    /// Slip angle where the grip peaks (rad) at the design pressure: the compound's, on
    /// this sidewall.
    pub fn peak_angle(&self) -> f32 {
        self.compound.peak_angle * self.sidewall
    }

    /// Inflation pressure (bar) with the carcass as warm as it is.
    pub fn pressure(&self, tread: &Tread) -> f32 {
        self.inflation * (tread.carcass + 273.15) / (AMBIENT + 273.15)
    }

    /// The pressure (bar) `load` (N) wants, hot: under it, the tire carries on its
    /// shoulders, over it, on its middle.
    fn wanted_pressure(&self, load: f32) -> f32 {
        DESIGN_PRESSURE * (load / self.nominal_load).max(0.1).powf(PRESSURE_LOAD)
    }

    /// The cold pressure (bar) that suits the tire under `load` (N) once it's warm.
    pub fn suited_pressure(&self, load: f32) -> f32 {
        let warm = Tread::at(AMBIENT).warmed(self.compound.best).carcass;
        self.wanted_pressure(load) * (AMBIENT + 273.15) / (warm + 273.15)
    }

    /// Stiffness at `pressure` (bar) relative to the stiffness at the design pressure.
    fn inflated(&self, pressure: f32) -> f32 {
        (pressure + SIDEWALL_PRESSURE) / (DESIGN_PRESSURE + SIDEWALL_PRESSURE)
    }

    /// Vertical stiffness (N/m) at `pressure` (bar).
    pub fn vertical_stiffness(&self, pressure: f32) -> f32 {
        self.stiffness * self.inflated(pressure)
    }

    /// Rolling resistance (share of the load) at `pressure` (bar).
    fn rolling_at(&self, pressure: f32) -> f32 {
        self.rolling * self.inflated(pressure).powf(-PRESSURE_ROLLING)
    }

    /// Torque (N·m) rolling resistance holds the wheel back with under `load` (N), as
    /// inflated as `tread` is warm.
    pub fn rolling_torque(&self, load: f32, tread: &Tread) -> f32 {
        self.rolling_at(self.pressure(tread)) * load * self.radius
    }

    /// The contact patch of a tire whose ribs reach `depth` (m, along the road's normal,
    /// inside to outside; negative where they're clear of it) into the road, closing in on
    /// it at `closing` (m/s), with `outward` (N) of sideways force pushing its contact
    /// patch towards its outside (inward: negative), leaning `camber` (rad) to its right.
    pub fn patch(
        &self,
        depth: [f32; RIBS],
        closing: f32,
        outward: f32,
        camber: f32,
        tread: &Tread,
    ) -> Patch {
        let pressure = self.pressure(tread);
        let inflated = self.inflated(pressure);
        let stiffness = self.stiffness * inflated / RIBS as f32;
        let damping = self.damping / RIBS as f32;
        // The belts spread a tilt across the tread.
        let mean = depth.iter().sum::<f32>() / RIBS as f32;
        let mut depth = depth.map(|d| mean + BELT * (d - mean));
        // The sideways force rolls the carcass under, towards the side it pushes from.
        let roll = ROLL_UNDER * outward / (self.nominal_load * inflated);
        // Under-inflated for its load, a tire carries on its shoulders; over-inflated, on
        // its middle.
        let load = stiffness * RIBS as f32 * mean.max(0.0);
        let wanted = self.wanted_pressure(load);
        let crown = CROWN * (pressure / wanted - 1.0).clamp(-0.4, 0.4);
        let spread = RIB_AT.iter().map(|u| u * u).sum::<f32>() / RIBS as f32;
        for (d, u) in depth.iter_mut().zip(RIB_AT) {
            *d += -roll * u * self.width / 2.0 + mean.max(0.0) * crown * (spread - u * u);
        }
        let loads = depth.map(|d| {
            if d > 0.0 {
                (stiffness * d + damping * closing).max(0.0)
            } else {
                0.0
            }
        });
        let grip = std::array::from_fn(|rib| {
            let load = loads[rib];
            self.rib_condition(tread, rib) * self.friction(load * RIBS as f32) * load
        });
        Patch {
            loads,
            grip,
            camber,
            peak: inflated.powf(-PRESSURE_PEAK),
        }
    }

    /// An evenly loaded patch under `fz` (N) with `condition` of its best grip on every rib,
    /// upright, at the design pressure.
    pub fn even_patch(&self, fz: f32, condition: f32) -> Patch {
        let load = fz / RIBS as f32;
        let grip = condition * self.friction(fz) * load;
        Patch {
            loads: [load; RIBS],
            grip: [grip; RIBS],
            camber: 0.0,
            peak: 1.0,
        }
    }

    /// Pneumatic trail at a slip: it shrinks as more of the contact patch slides and is
    /// gone at the peak slip angle, so the self-aligning torque (lateral force times trail)
    /// fades before the grip does. That's what makes the steering go light near the limit.
    /// Driving or braking hard slides the rear of the patch as well, so the slip ratio
    /// counts too, against its own peak.
    pub fn trail(&self, force: &TireForce) -> f32 {
        let slip = (force.slip_angle / self.peak_angle())
            .hypot(force.slip_ratio / self.compound.peak_ratio);
        self.pneumatic_trail * (1.0 - slip).max(0.0)
    }

    /// Relaxation lengths (m) along and across with `patch` under the tire, as warm as
    /// `tread` is, acting on `slips` (see `relax`): longer under more load, shorter the
    /// harder it's pumped up, and shorter as the tire saturates (the carcass carries the
    /// force, and past the peak the force no longer grows with the slip, so there's
    /// nothing more to build up: a sliding tire answers at once).
    pub fn relaxation(&self, patch: &Patch, tread: &Tread, slips: [f32; 2]) -> Relaxation {
        let loaded = (patch.load() / self.nominal_load).clamp(0.25, 2.5).sqrt();
        let length = self.relaxation * loaded / self.inflated(self.pressure(tread));
        let carcass = [length * ALONG, length];
        let slope = |slip: f32, peak: f32, c: f32| {
            let x = stiffness_for_peak(peak, c) * slip;
            ((c * x.atan()).cos() / (1.0 + x * x)).max(SLIDING_LAG)
        };
        let compound = &self.compound;
        let across = slips[1].atan() - CAMBER_SLIP * patch.camber;
        Relaxation {
            lengths: [
                carcass[0] * slope(slips[0], compound.peak_ratio, compound.c_long),
                carcass[1] * slope(across, self.peak_angle() * patch.peak, compound.c_lat),
            ],
            carcass,
        }
    }

    /// Advances the slips the tire acts on, `slips` (the slip ratio, and the slip angle's
    /// tangent), by `dt`, with the patch moving at (`vx` forward, `vy` right) under the
    /// wheel spinning at `omega` (rad/s), relaxing as `relaxation` says:
    /// rolling, they take up the patch's slips over that much road; standing, the carcass
    /// just flexes with the creep, like a spring. They're kept to what the tire can grip,
    /// or the slips it rolls at.
    pub fn relax(
        &self,
        slips: [f32; 2],
        vx: f32,
        vy: f32,
        omega: f32,
        relaxation: &Relaxation,
        dt: f32,
    ) -> [f32; 2] {
        let lengths = relaxation.lengths;
        let speeds = [omega * self.radius - vx, vy];
        let peaks = [self.compound.peak_ratio, self.peak_angle()];
        std::array::from_fn(|i| {
            let rate = vx.abs() / lengths[i];
            let relaxed = if rate * dt > 1e-5 {
                let steady = speeds[i] / vx.abs();
                steady + (slips[i] - steady) * (-rate * dt).exp()
            } else {
                slips[i] + speeds[i] * dt / lengths[i]
            };
            let reach = (1.5 * peaks[i]).max(speeds[i].abs() / vx.abs().max(0.5));
            relaxed.clamp(-reach, reach)
        })
    }

    /// Tire forces for a contact patch moving at (`vx` forward, `vy` right) with the wheel
    /// spinning at `omega` (rad/s, positive rolls forward), meeting the road as `patch`,
    /// in a steady state (rolling).
    #[cfg(test)]
    pub fn force(&self, vx: f32, vy: f32, omega: f32, patch: &Patch) -> TireForce {
        self.force_at(
            [(omega * self.radius - vx) / vx.abs(), vy / vx.abs()],
            patch,
        )
    }

    /// Tire forces acting on `slips` (see `relax`), the patch moving at (`vx` forward, `vy`
    /// right) under the wheel spinning at `omega`: at walking pace and below, the rubber
    /// also damps the motion (against the carcass' stiffness), which the relaxing does at
    /// speed.
    pub fn relaxed_force(
        &self,
        vx: f32,
        vy: f32,
        omega: f32,
        slips: [f32; 2],
        relaxation: &Relaxation,
        patch: &Patch,
    ) -> TireForce {
        let lengths = relaxation.carcass;
        let fade = (1.0 - vx.abs() / LOW_DAMPING_SPEED).max(0.0);
        let speeds = [omega * self.radius - vx, vy];
        self.force_at(
            std::array::from_fn(|i| slips[i] + fade * LOW_DAMPING[i] * speeds[i] / lengths[i]),
            patch,
        )
    }

    /// Tire forces at `slips`: the slip ratio, and the slip angle's tangent.
    fn force_at(&self, slips: [f32; 2], patch: &Patch) -> TireForce {
        let slip_ratio = slips[0];
        let slip_angle = slips[1].atan();

        // Combined slip: normalise both slips by their peak, so that the total force follows
        // the direction of the combined slip vector and saturates along a friction ellipse.
        // Each rib grips by its own load and condition, all with the same slip, so the
        // forces are the ribs' grip together on one curve. A leaning wheel pushes the way it
        // leans as if it slipped the other way.
        let compound = &self.compound;
        let (peak_long, peak_lat) = (compound.peak_ratio, self.peak_angle() * patch.peak);
        let sx = slip_ratio / peak_long;
        let sy = (slip_angle - CAMBER_SLIP * patch.camber) / peak_lat;
        let rho = sx.hypot(sy);
        if rho < 1e-6 {
            return TireForce {
                slip_ratio,
                slip_angle,
                ..Default::default()
            };
        }
        let d = patch.total_grip();
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

    /// Advances the tire by `dt`: the surface heated by `force` with the rubber sliding at
    /// (`slide_x` along, `slide_y` across, m/s), shared by the ribs as they grip; the core
    /// and the carcass by the rubber flexing under each rib's load rolling at `speed`
    /// (m/s); everything cooled by the road, the air and each other; and the ribs worn by
    /// their sliding. Without a patch, the wheel is in the air.
    pub fn heat(
        &self,
        tread: &mut Tread,
        contact: Option<(&TireForce, &Patch)>,
        slide_x: f32,
        slide_y: f32,
        speed: f32,
        dt: f32,
    ) {
        let size = self.size();
        let rib_size = size / RIBS as f32;
        let speed = speed.abs();
        let slide = slide_x.hypot(slide_y);
        let (sliding, flex, load, grip) = match contact {
            Some((force, patch)) => {
                let sliding = (force.fx * slide_x).abs() + (force.fy * slide_y).abs();
                let share = 1.0 / (1.0 + SLIDING_SHARE * slide.sqrt());
                let sliding = (sliding, share);
                let pushed = FLEX_FORCE * force.fx.hypot(force.fy);
                let rolling = self.rolling_at(self.pressure(tread));
                let flex = rolling * speed * (patch.load() + pushed);
                (sliding, flex, patch.load(), patch.total_grip())
            }
            None => ((0.0, 0.0), 0.0, 0.0, 0.0),
        };
        let (sliding, heated) = sliding;
        let (loads, grips) = contact.map_or(([0.0; RIBS], [0.0; RIBS]), |(_, patch)| {
            (patch.loads, patch.grip)
        });
        let surface_capacity = SURFACE_CAPACITY * rib_size;
        let core_capacity = CORE_CAPACITY * rib_size;
        let air = (SURFACE_AIR + SURFACE_AIR_SPEED * speed.sqrt()) * rib_size;
        let between = SURFACE_CORE * rib_size;
        let to_carcass = CORE_CARCASS * rib_size;
        let sideways = CORE_RIB * size;
        let life = self.compound.life * rib_size;
        let old = *tread;
        let mut into_carcass = FLEX_CARCASS * flex;
        for rib in 0..RIBS {
            let (surface, core) = (old.surface[rib], old.core[rib]);
            let share = if grip > 0.0 { grips[rib] / grip } else { 0.0 };
            let carried = if load > 0.0 { loads[rib] / load } else { 0.0 };
            let sliding_over = slide * slide / (slide * slide + ROAD_SLIDE * ROAD_SLIDE);
            let road = SURFACE_ROAD * rib_size * (RIBS as f32 * carried).min(2.0) * sliding_over;
            let rubbed = sliding * share;
            let soaked = between * (surface - core);
            let heating = heated * rubbed - soaked - (air + road) * (surface - AMBIENT);
            tread.surface[rib] += heating * dt / surface_capacity;
            let neighbours = [rib.wrapping_sub(1), rib + 1]
                .into_iter()
                .filter(|&n| n < RIBS)
                .map(|n| sideways * (old.core[n] - core))
                .sum::<f32>();
            let lost = to_carcass * (core - old.carcass);
            into_carcass += lost;
            let flexed = (1.0 - FLEX_CARCASS) * flex * carried;
            tread.core[rib] += (flexed + soaked - lost + neighbours) * dt / core_capacity;
            let rate = self.compound.wear_rate(surface);
            tread.wear[rib] = (old.wear[rib] + rubbed * rate * dt / life).min(1.0);
        }
        let cooled = (CARCASS_AIR + CARCASS_AIR_SPEED * speed.sqrt()) * size;
        let carcass = into_carcass - cooled * (old.carcass - AMBIENT);
        tread.carcass += carcass * dt / (CARCASS_CAPACITY * size);
    }

    /// The tread's area relative to the reference tread's.
    fn size(&self) -> f32 {
        self.tread_area / REFERENCE_TREAD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn force(tire: &TireParams, vx: f32, vy: f32, omega: f32, fz: f32) -> TireForce {
        tire.force(vx, vy, omega, &tire.even_patch(fz, 1.0))
    }

    #[test]
    fn peak_force_equals_mu_times_load() {
        for compound in COMPOUNDS {
            let tire = TireParams {
                compound,
                ..TireParams::default()
            };
            let peak = compound.peak_ratio;
            let f = force(&tire, 20.0, 0.0, 20.0 * (1.0 + peak) / tire.radius, 4000.0);
            assert!((f.fx - tire.friction(4000.0) * 4000.0).abs() < 1.0);
            assert!(f.fy.abs() < 1e-3);
        }
    }

    #[test]
    fn combined_slip_stays_inside_friction_circle() {
        let tire = TireParams::default();
        let f = force(&tire, 20.0, 5.0, 30.0 / tire.radius, 4000.0);
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
            -force(&tire, v, v * angle.tan(), v / tire.radius, 4000.0).fy
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

    /// The tire rolling at 20 m/s under 4 kN on an even patch for `seconds`, sliding
    /// sideways at `slide` (m/s) against 4 kN.
    fn roll(tire: &TireParams, tread: &mut Tread, slide: f32, seconds: f32) {
        let force = TireForce {
            fy: if slide > 0.0 { -4000.0 } else { 0.0 },
            ..Default::default()
        };
        for _ in 0..(seconds / 0.001) as usize {
            let patch = tire.patch([0.02; RIBS], 0.0, 0.0, 0.0, tread);
            tire.heat(tread, Some((&force, &patch)), 0.0, slide, 20.0, 0.001);
        }
    }

    #[test]
    fn sliding_heats_and_wears_the_tread_and_the_air_cools_it() {
        let tire = TireParams::default();
        let mut tread = Tread::default();
        // Sliding sideways at 2 m/s for 10 s heats it up and wears it a little.
        roll(&tire, &mut tread, 2.0, 10.0);
        let (hot, core) = (tread.surface_temperature(), tread.core_temperature());
        assert!(core > AMBIENT + 5.0 && hot > core, "{tread:?}");
        assert!(tread.wear() > 0.0 && tread.wear() < 0.01, "{tread:?}");
        // Rolling straight, the surface cools below the core, which the flexing keeps a
        // little warm, and it doesn't wear.
        let worn = tread.wear();
        roll(&tire, &mut tread, 0.0, 300.0);
        assert!(tread.surface_temperature() < hot - 5.0, "{tread:?}");
        assert!(
            tread.surface_temperature() < tread.core_temperature(),
            "{tread:?}"
        );
        assert!(tread.core_temperature() < AMBIENT + 25.0, "{tread:?}");
        assert_eq!(tread.wear(), worn);
        // Worn through, it grips a quarter less.
        tread.set_wear(1.0);
        let warm = tire.compound.warmth(tread.surface[0]);
        assert!((tire.rib_condition(&tread, 0) - 0.75 * warm).abs() < 1e-6);
    }

    #[test]
    fn a_slide_flashes_the_surface_but_the_core_takes_steady_work() {
        let tire = TireParams::default();
        let mut tread = Tread::default();
        // Three seconds sliding at 12 m/s (a spin) makes the surface hot, the core warm.
        roll(&tire, &mut tread, 12.0, 3.0);
        let (surface, core) = (tread.surface_temperature(), tread.core_temperature());
        assert!(surface > 100.0, "{tread:?}");
        assert!(core < AMBIENT + 15.0, "{tread:?}");
        // Ten seconds later the surface is back near the core, and the core is barely
        // warmer than a tire's that only rolled.
        roll(&tire, &mut tread, 0.0, 10.0);
        assert!(
            tread.surface_temperature() < tread.core_temperature() + 10.0,
            "{tread:?}"
        );
        let mut rolled = Tread::default();
        roll(&tire, &mut rolled, 0.0, 13.0);
        let warmer = tread.core_temperature() - rolled.core_temperature();
        assert!(warmer < 12.0, "{warmer} K: {tread:?}");
        // A minute of hard cornering (sliding at 1.5 m/s) warms it through.
        let mut cornering = Tread::default();
        roll(&tire, &mut cornering, 1.5, 60.0);
        assert!(cornering.core_temperature() > core + 10.0, "{cornering:?}");
        // The carcass warms with it, and so the pressure rises.
        assert!(cornering.carcass > AMBIENT + 5.0, "{cornering:?}");
        assert!(tire.pressure(&cornering) > tire.inflation + 0.05);
    }

    #[test]
    fn camber_and_cornering_move_the_load_across_the_tread() {
        let tire = TireParams::default();
        let tread = Tread::default();
        // Upright at the right pressure for its load, the ribs share evenly.
        let mut tire = tire;
        for _ in 0..20 {
            let load = tire.patch([0.02; RIBS], 0.0, 0.0, 0.0, &tread).load();
            tire.inflation = DESIGN_PRESSURE * (load / tire.nominal_load).powf(PRESSURE_LOAD);
        }
        let wanted = tire.inflation;
        let even = tire.patch([0.02; RIBS], 0.0, 0.0, 0.0, &tread);
        assert!((even.loads[0] - even.loads[4]).abs() < 1.0, "{even:?}");
        assert!(
            (even.loads[0] - even.loads[2]).abs() < 0.02 * even.loads[2],
            "{even:?}"
        );
        // Pushed inwards in a corner (the outside wheel), the outside shoulder carries more.
        let cornering = tire.patch([0.02; RIBS], 0.0, -4000.0, 0.0, &tread);
        assert!(
            cornering.loads[4] > 1.1 * cornering.loads[0],
            "{cornering:?}"
        );
        // Leaning in (negative camber) evens that out.
        let tilt = 0.07 * tire.width / 2.0;
        let leaning = RIB_AT.map(|u| 0.02 - tilt * u);
        let cambered = tire.patch(leaning, 0.0, -4000.0, 0.0, &tread);
        let spread = |p: &Patch| p.loads[4] - p.loads[0];
        assert!(
            spread(&cambered).abs() < 0.5 * spread(&cornering),
            "{cambered:?}"
        );
        // That grips more than the uneven patch.
        assert!(cambered.total_grip() > cornering.total_grip());
        // Over-inflated, the middle carries more; under-inflated, the shoulders.
        let at = |inflation: f32| {
            let tire = TireParams { inflation, ..tire };
            tire.patch([0.02; RIBS], 0.0, 0.0, 0.0, &tread)
        };
        let (hard, soft) = (at(wanted + 0.6), at(wanted - 0.6));
        assert!(hard.loads[2] > 1.05 * hard.loads[0], "{hard:?}");
        assert!(soft.loads[0] > 1.05 * soft.loads[2], "{soft:?}");
    }

    #[test]
    fn a_leaning_wheel_pushes_the_way_it_leans() {
        let tire = TireParams::default();
        let mut patch = tire.even_patch(4000.0, 1.0);
        patch.camber = 0.05;
        let f = tire.force(20.0, 0.0, 20.0 / tire.radius, &patch);
        assert!(f.fy > 100.0 && f.fy < 400.0, "{f:?}");
    }

    #[test]
    fn the_sideways_force_builds_up_over_the_relaxation_length() {
        let tire = TireParams::default();
        let tread = Tread::default();
        let patch = tire.even_patch(tire.nominal_load, 1.0);
        let (vx, vy) = (20.0, 20.0 * 0.02);
        let lengths = |slip: f32| tire.relaxation(&patch, &tread, [0.0, slip]);
        let length = lengths(0.0).lengths[1];
        assert!((length - RELAXATION).abs() < 1e-4, "{length}");
        // Rolled one relaxation length, it's taken up about two thirds of the slip.
        let mut slip = 0.0;
        let dt = 0.001;
        let omega = vx / tire.radius;
        let step = |slip: f32| tire.relax([0.0, slip], vx, vy, omega, &lengths(slip), dt)[1];
        for _ in 0..(length / vx / dt).round() as usize {
            slip = step(slip);
        }
        assert!((slip / 0.02 - 0.65).abs() < 0.1, "{slip}");
        for _ in 0..1000 {
            slip = step(slip);
        }
        assert!((slip - 0.02).abs() < 1e-5, "{slip}");
        // Sliding, it answers sooner.
        assert!(lengths(0.3).lengths[1] < 0.2 * length);
        // Standing, the carcass flexes with the creep and springs back against it, both
        // ways.
        let lengths = lengths(0.0);
        let creep = tire.relax([0.0; 2], 0.0, 0.01, 0.0, &lengths, 0.1);
        assert!((creep[1] - 0.001 / length).abs() < 1e-6, "{creep:?}");
        let held = tire.relaxed_force(0.0, 0.0, 0.0, creep, &lengths, &patch);
        assert!(held.fy < -50.0, "{held:?}");
        let rolled = tire.relax([0.0; 2], -0.01, 0.0, 0.0, &lengths, 0.1);
        let held = tire.relaxed_force(0.0, 0.0, 0.0, rolled, &lengths, &patch);
        assert!(held.fx > 50.0, "{held:?}");
    }
}
