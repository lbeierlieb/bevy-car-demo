//! The fuel tank: what's in it, where it sits, and whether the pump's pickup draws fuel
//! or air when the fuel sloshes away from it.

use bevy::prelude::*;

/// Density of petrol (kg/L).
pub const DENSITY: f32 = 0.745;
/// Fuel the pump can't reach even standing level, as a share of the tank.
const UNUSABLE: f32 = 0.01;
/// How much of the tank the fuel needs to keep the pickup covered per g of sideways or
/// lengthwise acceleration (a baffled tank with a small swirl pot).
const SLOSH: f32 = 0.12;
/// Over how much of the tank the pickup goes from covered to drawing only air.
const UNCOVERING: f32 = 0.015;
/// How quickly the fuel sloshes over to a new acceleration (s).
const SLOSH_TIME: f32 = 0.4;
/// How long the fuel in the lines and the rail keeps the injectors fed once the pickup
/// draws air, and how quickly the pressure comes back (s).
const LINES_TIME: f32 = 0.25;
/// What a car comes with (RON).
pub const PUMP_OCTANE: f32 = 95.0;

/// Where a chassis has its tank and how big it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tank {
    /// Capacity (L).
    pub litres: f32,
    /// Its centre: behind the front axle, and above the ground (m).
    pub at: f32,
    pub height: f32,
}

/// The fuel in a car's tank.
#[derive(Clone, Copy, Debug)]
pub struct Fuel {
    /// What's left, and what's been burnt since filling up (L; a step's worth is too
    /// little to take off a full tank in single precision).
    litres: f64,
    pub burnt: f64,
    /// Its research octane number: fuels of different grades in the tank mix by volume.
    octane: f32,
    /// Distance driven since filling up (m).
    pub distance: f64,
    /// The acceleration the fuel has settled to, sideways and lengthwise, as a share of
    /// the acceleration pressing it into the tank's floor (a slope counts too).
    slosh: f32,
    /// Share of what the injectors ask for that reaches them.
    pub supply: f32,
}

impl Fuel {
    pub fn new(litres: f32) -> Self {
        Self {
            litres: litres.into(),
            burnt: 0.0,
            octane: PUMP_OCTANE,
            distance: 0.0,
            slosh: 0.0,
            supply: 1.0,
        }
    }

    /// `litres` of fuel of `octane` (RON), as a tank was left.
    pub fn with(litres: f32, octane: f32) -> Self {
        Self {
            octane,
            ..Self::new(litres)
        }
    }

    /// What's left (L).
    pub fn litres(&self) -> f32 {
        self.litres as f32
    }

    /// Its research octane number.
    pub fn octane(&self) -> f32 {
        self.octane
    }

    /// Fills in `litres` of fuel of `octane` (RON), up to what fits into `tank`: the
    /// octane numbers mix by volume (near enough for pump grades). Filling up starts the
    /// consumption over.
    pub fn fill(&mut self, tank: &Tank, litres: f32, octane: f32) {
        let before = self.litres;
        let added = f64::from(litres.max(0.0)).min(f64::from(tank.litres) - before);
        if added <= 0.0 {
            return;
        }
        self.litres = before + added;
        let mixed = (before * f64::from(self.octane) + added * f64::from(octane)) / self.litres;
        self.octane = mixed as f32;
        (self.burnt, self.distance) = (0.0, 0.0);
    }

    /// Mass of the fuel (kg).
    pub fn mass(&self) -> f32 {
        self.litres() * DENSITY
    }

    /// Average consumption since filling up (L/100 km), once there's a kilometre to go by.
    pub fn average(&self) -> Option<f32> {
        (self.distance > 1000.0).then(|| (self.burnt / self.distance * 100e3) as f32)
    }

    /// Keeps no more than fits into `tank`.
    pub fn fit(&mut self, tank: &Tank) {
        self.litres = self.litres.min(tank.litres.into());
    }

    /// Advances by `dt` with the engine burning `flow` (kg/s) and the car feeling
    /// `specific_force` (its acceleration less gravity, in the car's frame: y up, m/s²).
    pub fn step(&mut self, tank: &Tank, flow: f32, specific_force: Vec3, dt: f32) {
        let burnt = (f64::from(flow / DENSITY) * f64::from(dt)).min(self.litres);
        self.litres -= burnt;
        self.burnt += burnt;
        let f = specific_force;
        let tilt = f.xz().length() / f.y.max(1.0);
        self.slosh += (tilt - self.slosh) * (dt / SLOSH_TIME).min(1.0);
        let covered = self.covered(tank);
        self.supply += (covered - self.supply) * (dt / LINES_TIME).min(1.0);
    }

    /// How well the pickup is covered (0 draws only air, 1 only fuel).
    fn covered(&self, tank: &Tank) -> f32 {
        let needed = tank.litres * (UNUSABLE + SLOSH * self.slosh.min(2.0));
        ((self.litres() - needed) / (tank.litres * UNCOVERING)).clamp(0.0, 1.0)
    }
}

crate::snapshot::snapshot!(Fuel {
    litres,
    burnt,
    octane,
    distance,
    slosh,
    supply
});

#[cfg(test)]
mod tests {
    use super::*;

    const TANK: Tank = Tank {
        litres: 50.0,
        at: 2.3,
        height: 0.35,
    };
    const LEVEL: Vec3 = Vec3::new(0.0, 9.81, 0.0);

    #[test]
    fn grades_mix_by_volume() {
        let mut fuel = Fuel::new(20.0);
        fuel.fill(&TANK, 20.0, 102.0);
        assert_eq!(fuel.litres(), 40.0);
        assert!((fuel.octane() - 98.5).abs() < 1e-4, "{}", fuel.octane());
        // Only what fits goes in.
        fuel.fill(&TANK, 30.0, 95.0);
        assert_eq!(fuel.litres(), 50.0);
        assert!((fuel.octane() - 97.8).abs() < 1e-4, "{}", fuel.octane());
        // An empty tank takes the grade put in.
        let mut fuel = Fuel::new(0.0);
        fuel.fill(&TANK, 5.0, 98.0);
        assert_eq!(fuel.octane(), 98.0);
    }

    fn settle(fuel: &mut Fuel, force: Vec3) {
        for _ in 0..240 {
            fuel.step(&TANK, 0.0, force, 1.0 / 120.0);
        }
    }

    #[test]
    fn burning_empties_the_tank() {
        let mut fuel = Fuel::new(10.0);
        // 3 g/s for a minute is about a quarter of a litre.
        for _ in 0..60 * 120 {
            fuel.step(&TANK, 0.003, LEVEL, 1.0 / 120.0);
        }
        assert!(
            (fuel.litres() - (10.0 - 0.18 / DENSITY)).abs() < 1e-4,
            "{}",
            fuel.litres
        );
        assert!((fuel.burnt - 0.18 / f64::from(DENSITY)).abs() < 1e-6);
        assert_eq!(fuel.supply, 1.0);
    }

    #[test]
    fn a_nearly_empty_tank_starves_in_corners() {
        let cornering = Vec3::new(9.0, 9.81, 0.0);
        let mut fuel = Fuel::new(15.0);
        settle(&mut fuel, cornering);
        assert!(fuel.supply > 0.99, "plenty left: {}", fuel.supply);
        let mut fuel = Fuel::new(4.0);
        settle(&mut fuel, LEVEL);
        assert!(fuel.supply > 0.99, "level: {}", fuel.supply);
        settle(&mut fuel, cornering);
        assert!(fuel.supply < 0.05, "in a long corner: {}", fuel.supply);
        // A short jolt is bridged by the fuel in the lines and the slow slosh.
        let mut fuel = Fuel::new(4.0);
        for _ in 0..12 {
            fuel.step(&TANK, 0.0, cornering, 1.0 / 120.0);
        }
        assert!(fuel.supply > 0.9, "short jolt: {}", fuel.supply);
        let mut fuel = Fuel::new(0.2);
        settle(&mut fuel, LEVEL);
        assert!(fuel.supply < 0.01, "empty: {}", fuel.supply);
    }
}
