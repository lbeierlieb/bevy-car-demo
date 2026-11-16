//! What wears and breaks in the engine. Each part has a wear from 0 (new) to 1 (failed).
//! Pistons and valves wear per cylinder: a holed piston or bent valves kill the cylinder,
//! and the others run on.
//! A blown head gasket costs compression and heats the coolant. Spun bearings wreck the
//! engine for good.

/// Most cylinders an engine can have (the per-cylinder state is a fixed array).
pub const MAX_CYLINDERS: usize = 8;
/// How hard each cylinder has it against the average (cooling, mixture and tolerances
/// differ), so they don't all fail at once.
const SPREAD: [f32; MAX_CYLINDERS] = [0.96, 1.03, 1.08, 0.93, 1.01, 0.97, 1.05, 0.99];
/// Wear per second: per degree² of spark past the knock limit at full load, and per
/// kelvin of coolant (warping the head, lifting the gasket) or oil (thinning it, wearing
/// the bearings) over its limit.
const KNOCK_WEAR: f32 = 2e-4;
const HEAT_WEAR: f32 = 2e-3;
const COOLANT_LIMIT: f32 = 393.0;
const OIL_LIMIT: f32 = 423.0;
/// Valve float (spring force short of the valves' inertia, relative to it) where the
/// valves start to touch the pistons, and the wear per cycle per unit of float beyond.
const VALVE_CONTACT: f32 = 0.06;
const VALVE_WEAR: f32 = 0.3;
/// Efficiency lost with the pistons and rings worn out (blow-by), and with the head
/// gasket blown (leaking compression).
const PISTON_EFFICIENCY: f32 = 0.3;
const GASKET_EFFICIENCY: f32 = 0.15;
/// Share of the fuel's energy a blown head gasket puts into the coolant (combustion gas
/// pushed into the water jacket).
const GASKET_HEAT: f32 = 0.1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CylinderWear {
    /// Pistons and rings, from knock.
    pub piston: f32,
    /// Valves, from meeting the piston when they float.
    pub valves: f32,
}

impl CylinderWear {
    /// Why this cylinder no longer fires, if it doesn't.
    pub fn failure(&self) -> Option<&'static str> {
        if self.piston >= 1.0 {
            Some("holed piston")
        } else if self.valves >= 1.0 {
            Some("bent valves")
        } else {
            None
        }
    }
}

/// What the engine went through in one step, for the wear.
pub struct Strain {
    pub rpm: f32,
    /// Valve float: spring force short of the valves' inertia, relative to it.
    pub float: f32,
    /// Spark advance past the knock limit (degrees) and load (air relative to filling the
    /// displacement at ambient).
    pub knock: f32,
    pub load: f32,
    /// Coolant and oil temperatures (K).
    pub coolant: f32,
    pub oil: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Damage {
    pub cylinders: [CylinderWear; MAX_CYLINDERS],
    pub head_gasket: f32,
    pub bearings: f32,
}

impl Damage {
    /// Wears the parts of an engine with `cylinders` cylinders by what it went through in
    /// `dt`.
    pub fn step(&mut self, cylinders: usize, strain: &Strain, dt: f32) {
        let knock = KNOCK_WEAR * strain.knock.powi(2) * strain.load;
        let cycles = strain.rpm / 120.0;
        let contact = VALVE_WEAR * (strain.float - VALVE_CONTACT).max(0.0) * cycles;
        for (cylinder, spread) in self.cylinders.iter_mut().zip(SPREAD).take(cylinders) {
            if cylinder.failure().is_none() {
                cylinder.piston = (cylinder.piston + knock * spread * dt).min(1.0);
                cylinder.valves = (cylinder.valves + contact * spread * dt).min(1.0);
            }
        }
        let wear = |part: &mut f32, rate: f32| *part = (*part + rate * dt).min(1.0);
        wear(
            &mut self.head_gasket,
            HEAT_WEAR * (strain.coolant - COOLANT_LIMIT).max(0.0),
        );
        wear(
            &mut self.bearings,
            HEAT_WEAR * (strain.oil - OIL_LIMIT).max(0.0),
        );
    }

    /// Share of the `cylinders` cylinders that still fire.
    pub fn firing(&self, cylinders: usize) -> f32 {
        let cylinders = cylinders.min(MAX_CYLINDERS);
        let alive = self.cylinders[..cylinders]
            .iter()
            .filter(|cylinder| cylinder.failure().is_none())
            .count();
        alive as f32 / cylinders as f32
    }

    /// Broken beyond running: it won't turn over, or no cylinder fires.
    pub fn wrecked(&self, cylinders: usize) -> bool {
        self.bearings >= 1.0 || self.firing(cylinders) == 0.0
    }

    pub fn gasket_blown(&self) -> bool {
        self.head_gasket >= 1.0
    }

    /// Share of the indicated efficiency the firing cylinders keep.
    pub fn efficiency(&self, cylinders: usize) -> f32 {
        let firing = self.cylinders[..cylinders.min(MAX_CYLINDERS)]
            .iter()
            .filter(|cylinder| cylinder.failure().is_none());
        let (sum, count) = firing.fold((0.0, 0), |(sum, n), c| (sum + c.piston, n + 1));
        let piston = if count > 0 { sum / count as f32 } else { 0.0 };
        let gasket = if self.gasket_blown() {
            GASKET_EFFICIENCY
        } else {
            0.0
        };
        (1.0 - PISTON_EFFICIENCY * piston) * (1.0 - gasket)
    }

    /// Share of the fuel's energy that goes into the coolant on top of the walls' heat.
    pub fn extra_coolant_heat(&self) -> f32 {
        if self.gasket_blown() {
            GASKET_HEAT
        } else {
            0.0
        }
    }

    /// The parts by name with their wear (0..1), the worst first.
    pub fn parts(&self, cylinders: usize) -> Vec<(String, f32)> {
        let mut parts: Vec<(String, f32)> = self.cylinders[..cylinders.min(MAX_CYLINDERS)]
            .iter()
            .enumerate()
            .flat_map(|(i, c)| {
                [
                    (format!("Piston {}", i + 1), c.piston),
                    (format!("Valves {}", i + 1), c.valves),
                ]
            })
            .collect();
        parts.push(("Head gasket".to_string(), self.head_gasket));
        parts.push(("Bearings".to_string(), self.bearings));
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        parts
    }

    /// What has failed, as the driver would hear it from a mechanic.
    pub fn failures(&self, cylinders: usize) -> Vec<String> {
        let mut failures: Vec<String> = self.cylinders[..cylinders.min(MAX_CYLINDERS)]
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some(format!("cylinder {}: {}", i + 1, c.failure()?)))
            .collect();
        if self.gasket_blown() {
            failures.push("head gasket blown".to_string());
        }
        if self.bearings >= 1.0 {
            failures.push("spun bearing".to_string());
        }
        failures
    }

    /// The worst wear of any part (0..1).
    pub fn worst(&self, cylinders: usize) -> f32 {
        self.parts(cylinders).first().map_or(0.0, |part| part.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strain(knock: f32) -> Strain {
        Strain {
            rpm: 3000.0,
            float: 0.0,
            knock,
            load: 1.0,
            coolant: 360.0,
            oil: 370.0,
        }
    }

    #[test]
    fn pistons_hole_one_by_one() {
        let mut damage = Damage::default();
        let mut first = None;
        for step in 0..100_000 {
            damage.step(4, &strain(12.0), 0.01);
            let firing = damage.firing(4);
            if firing < 1.0 && first.is_none() {
                first = Some(step);
                // The hottest cylinder goes first, and the engine runs on three.
                assert_eq!(damage.cylinders[2].failure(), Some("holed piston"));
                assert_eq!(firing, 0.75);
                assert!(!damage.wrecked(4));
            }
            if damage.wrecked(4) {
                let first = first.unwrap() as f32;
                assert!(step as f32 > first * 1.1, "{first} then {step}");
                return;
            }
        }
        panic!("never wrecked");
    }

    #[test]
    fn floating_valves_bend_on_a_missed_shift() {
        let mut damage = Damage::default();
        // Just floating: no contact yet.
        let floating = Strain {
            rpm: 7300.0,
            float: 0.03,
            ..strain(0.0)
        };
        damage.step(4, &floating, 10.0);
        assert_eq!(damage.worst(4), 0.0);
        // A fifth-to-second shift: 9000 rpm for a tenth of a second.
        let missed = Strain {
            rpm: 9000.0,
            float: 0.55,
            ..strain(0.0)
        };
        for _ in 0..12 {
            damage.step(4, &missed, 0.01);
        }
        assert!(damage.firing(4) < 0.5, "{:?}", damage.cylinders);
    }

    #[test]
    fn overheating_blows_the_gasket_and_spins_a_bearing() {
        let mut damage = Damage::default();
        let hot = Strain {
            coolant: 400.0,
            oil: 440.0,
            ..strain(0.0)
        };
        for _ in 0..100 {
            damage.step(4, &hot, 1.0);
        }
        assert!(damage.gasket_blown() && damage.efficiency(4) < 0.9);
        assert!(damage.extra_coolant_heat() > 0.0);
        assert!(damage.wrecked(4));
        assert_eq!(damage.failures(4), ["head gasket blown", "spun bearing"]);
    }
}
