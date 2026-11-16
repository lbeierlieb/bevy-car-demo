//! Engine control units: they read the sensors and drive the actuators, using their own
//! idea of the engine (their maps), which matches the real one only as well as the tune.

use super::hardware::EngineHardware;
use super::maps::{self, EngineMaps};
use super::table::{self, Table};
use super::turbo;
use super::{AMBIENT_PRESSURE, AMBIENT_TEMPERATURE, R_AIR, STOICHIOMETRIC};

/// Opening of the idle air valve with the engine at its idle speed, and the controller's
/// gains on the speed error (per rpm, and per rpm·s).
const IDLE_BASE: f32 = 0.02;
const IDLE_P: f32 = 6e-4;
const IDLE_I: f32 = 6e-4;
/// Below this the engine is stopped or cranking, and idle control waits.
const RUNNING_RPM: f32 = 300.0;
/// Overrun fuel cut with the throttle closed: from this far above idle, until the revs
/// drop to the lower margin.
const OVERRUN_CUT: f32 = 700.0;
const OVERRUN_RESUME: f32 = 400.0;

/// What the engine's sensors read.
#[derive(Clone, Copy, Debug)]
pub struct Sensors {
    pub rpm: f32,
    /// Intake manifold pressure (Pa).
    pub map: f32,
    /// Throttle position (0 closed, 1 wide open).
    pub throttle: f32,
    /// Air temperature in the intake manifold (K).
    pub intake_temperature: f32,
}

/// What the ECU commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct Actuators {
    /// Fuel injected per cylinder and cycle (kg).
    pub fuel: f32,
    /// Spark advance (degrees before top dead centre).
    pub spark: f32,
    /// Idle air valve opening (0..1).
    pub idle_valve: f32,
    /// Cam profile in use, and the phasers' positions (crank degrees; intake advanced,
    /// exhaust retarded).
    pub cam_profile: usize,
    pub intake_phase: f32,
    pub exhaust_phase: f32,
}

/// The first ECU: speed-density fuelling from rpm, manifold pressure and intake air
/// temperature, a spark map, a rev limiter and idle control. The cams stay on the first profile, unphased.
#[derive(Clone, Debug)]
pub struct Ecu {
    /// Engine size as set up in the ECU: displacement (m³) and cylinders.
    pub displacement: f32,
    pub cylinders: u32,
    /// Volumetric efficiency the ECU assumes, by rpm and manifold pressure (Pa).
    pub ve: Table,
    /// Air-fuel ratio to aim for (λ), by rpm and manifold pressure.
    pub lambda: Table,
    /// Spark advance (degrees before top dead centre), by rpm and manifold pressure.
    pub spark: Table,
    pub idle_rpm: f32,
    /// Rev limiter: at `rev_limit` the fuel is cut for `limiter_cut`, then back on for at
    /// least `limiter_fuel` (s), so the engine audibly bounces off the limiter.
    pub rev_limit: f32,
    pub limiter_cut: f32,
    pub limiter_fuel: f32,
}

/// The ECU's memory between steps.
#[derive(Clone, Copy, Debug, Default)]
pub struct EcuState {
    /// Time left in the rev limiter's cycle: fuel cut, then fuel back on.
    limiter: f32,
    /// Integral term of the idle controller (valve opening).
    idle: f32,
    /// Fuel cut on the overrun.
    overrun: bool,
}

impl Ecu {
    /// A factory tune for the hardware: its VE table matches the engine, it runs λ = 1 at
    /// part load and richer towards full load, and the spark just short of MBT or of the
    /// knock limit on `octane` fuel, whichever comes first. With a turbo, the maps reach
    /// 300 kPa and go richer still on boost.
    pub fn calibrated(hardware: &EngineHardware, maps: &EngineMaps, octane: f32) -> Self {
        let top = if hardware.turbo.is_some() { 30 } else { 11 };
        let axes = || {
            let rpm = (0..17).map(|i| i as f32 * 500.0).collect();
            let map = (1..=top).map(|i| i as f32 * 10e3).collect();
            vec![rpm, map]
        };
        let lambda = |map: f32| {
            1.0 - 0.12 * ((map - 60e3) / 35e3).clamp(0.0, 1.0)
                - 0.1 * ((map - 100e3) / 100e3).clamp(0.0, 1.0)
        };
        // On the dyno: the charge as warm as the intercooler leaves it, the exhaust
        // pressure whatever the turbo and exhaust make with the wastegate on its spring.
        let charge = |map| {
            hardware
                .turbo
                .as_ref()
                .map_or(AMBIENT_TEMPERATURE, |turbo| {
                    turbo::expected_charge(turbo, map)
                })
        };
        let wastegate = |map: f32| {
            hardware.turbo.as_ref().map_or(0.0, |turbo| {
                turbo::wastegate_opening(&turbo.wastegate, map - AMBIENT_PRESSURE)
            })
        };
        let displacement = hardware.displacement();
        let load = |ve: f32, map: f32| ve * map / (R_AIR * charge(map)) / super::ambient_density();
        let ve = Table::bake(axes(), |p| {
            let (rpm, map) = (p[0], p[1]);
            let mut exhaust = AMBIENT_PRESSURE;
            let mut ve = 0.0;
            for _ in 0..3 {
                ve = maps.ve(0, rpm, map, exhaust / map, 0.0, 0.0);
                let air = load(ve, map) * super::ambient_density() * displacement;
                let flow = air * (1.0 + 1.0 / (STOICHIOMETRIC * lambda(map))) * rpm / 120.0;
                let temperature = maps.exhaust_temperature(rpm, load(ve, map));
                exhaust = super::exhaust_pressure(hardware, flow, temperature, wastegate(map));
            }
            ve
        });
        let spark = Table::bake(axes(), |p| {
            let (rpm, map) = (p[0], p[1]);
            let load = load(ve.get(p), map);
            let shift = maps::knock_limit_shift(charge(map), 363.0, octane);
            let knock = maps.knock_limit(rpm, load) + shift;
            maps.mbt(rpm, load).min(knock) - 2.0
        });
        Self {
            displacement: hardware.displacement(),
            cylinders: hardware.cylinders,
            lambda: Table::bake(axes(), |p| lambda(p[1])),
            ve,
            spark,
            idle_rpm: 850.0,
            rev_limit: 6800.0,
            limiter_cut: 0.08,
            limiter_fuel: 0.04,
        }
    }

    pub fn step(&self, state: &mut EcuState, sensors: &Sensors, dt: f32) -> Actuators {
        let Sensors {
            rpm,
            map,
            throttle,
            intake_temperature,
        } = *sensors;
        // Rev limiter: a cut starts at the limit; after it the fuel comes back for a moment
        // even if the revs haven't dropped (in gear they barely do), unless the engine is
        // well over the limit.
        state.limiter = (state.limiter - dt).max(0.0);
        if state.limiter == 0.0 && rpm >= self.rev_limit {
            state.limiter = self.limiter_cut + self.limiter_fuel;
        }
        let limiter = state.limiter > self.limiter_fuel || rpm > self.rev_limit + 200.0;
        // Overrun: no fuel while the throttle is closed well above idle.
        let above_idle = rpm - self.idle_rpm;
        state.overrun = throttle < 0.01
            && (above_idle > OVERRUN_CUT || state.overrun && above_idle > OVERRUN_RESUME);
        let cut = limiter || state.overrun;

        // Idle: the valve holds the idle speed with the throttle closed; above idle it
        // closes for engine braking.
        let error = self.idle_rpm - rpm;
        if rpm < RUNNING_RPM {
            state.idle = 0.0;
        } else if throttle < 0.01 && error > -400.0 {
            state.idle = (state.idle + IDLE_I * error * dt).clamp(-IDLE_BASE, 0.5);
        }
        let idle_valve = (IDLE_BASE + state.idle + IDLE_P * error).clamp(0.0, 1.0);

        let point = [rpm, map];
        let air = self.ve.get(&point) * map * self.displacement
            / (R_AIR * intake_temperature * self.cylinders as f32);
        let fuel = if cut {
            0.0
        } else {
            air / (STOICHIOMETRIC * self.lambda.get(&point))
        };
        Actuators {
            fuel,
            spark: self.spark.get(&point),
            idle_valve,
            ..Actuators::default()
        }
    }
}

/// Tunes as text: a comment, `engine <name>` (the build it was made for), the settings as
/// `<key> <value>`, and the maps (see `table::write_table`).
impl Ecu {
    pub fn to_text(&self, engine: &str) -> String {
        let mut text = format!("# car-sim ECU tune\nengine {engine}\n");
        text += &format!("cylinders {}\n", self.cylinders);
        for (key, value) in self.settings() {
            text += &format!("{key} {value}\n");
        }
        for (name, table) in [
            ("ve", &self.ve),
            ("lambda", &self.lambda),
            ("spark", &self.spark),
        ] {
            table::write_table(&mut text, name, table);
        }
        text
    }

    /// Reads a tune written by `to_text` into this ECU; whatever is missing or broken keeps
    /// its value. Returns the engine the tune was made for.
    pub fn read_text(&mut self, text: &str) -> Option<String> {
        let mut engine = None;
        let mut tables = Vec::new();
        table::read_text(
            text,
            |key, value| match key {
                "engine" => engine = Some(value.to_string()),
                "cylinders" => self.cylinders = value.parse().unwrap_or(self.cylinders),
                _ => {
                    let value = value.parse::<f32>().ok().filter(|v| v.is_finite());
                    if let (Some(field), Some(value)) = (self.setting(key), value) {
                        *field = value;
                    }
                }
            },
            |name, axes, values| tables.push((name.to_string(), axes, values)),
        );
        for (name, axes, values) in tables {
            self.set_table(&name, axes, values);
        }
        engine
    }

    fn settings(&self) -> [(&'static str, f32); 5] {
        [
            ("displacement", self.displacement),
            ("idle_rpm", self.idle_rpm),
            ("rev_limit", self.rev_limit),
            ("limiter_cut", self.limiter_cut),
            ("limiter_fuel", self.limiter_fuel),
        ]
    }

    fn setting(&mut self, key: &str) -> Option<&mut f32> {
        match key {
            "displacement" => Some(&mut self.displacement),
            "idle_rpm" => Some(&mut self.idle_rpm),
            "rev_limit" => Some(&mut self.rev_limit),
            "limiter_cut" => Some(&mut self.limiter_cut),
            "limiter_fuel" => Some(&mut self.limiter_fuel),
            _ => None,
        }
    }

    /// Replaces a map, if the tune's version of it fits together.
    fn set_table(&mut self, name: &str, axes: Vec<Vec<f32>>, values: Vec<f32>) {
        let target = match name {
            "ve" => &mut self.ve,
            "lambda" => &mut self.lambda,
            "spark" => &mut self.spark,
            _ => return,
        };
        if let Some(table) = Table::from_parts(axes, values) {
            *target = table;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::physics::engine::EngineBuild;
    use crate::physics::engine::hardware::EngineHardware;

    #[test]
    fn tunes_survive_text() {
        let build = EngineBuild::default();
        let mut tuned = build.ecu.clone();
        tuned.rev_limit = 7234.5;
        tuned.spark.set_cell(&[10, 8], 31.25);
        tuned.ve.scale_at(&[3300.0, 71e3], 1.07);
        let text = tuned.to_text("test engine");
        // Read into a different ECU (the turbo's, with longer maps): all of it comes over.
        let turbo = EngineBuild::new(EngineHardware::turbo());
        let mut read = turbo.ecu.clone();
        assert_eq!(read.read_text(&text).as_deref(), Some("test engine"));
        assert_eq!(read.to_text("test engine"), text);
        assert_eq!(read.spark.cell(&[10, 8]), 31.25);
        assert_eq!(read.ve.values(), tuned.ve.values());
    }

    #[test]
    fn broken_tunes_keep_what_they_cannot_replace() {
        let factory = EngineBuild::default().ecu;
        let mut ecu = factory.clone();
        let text = "rev_limit lots\nidle_rpm 900\ntable spark\naxis 0 1000\naxis 1 2\nvalues 1 2\n\
                    table lambda\naxis 0 500\nvalues 1 1\nvalues 1 1\n";
        assert_eq!(ecu.read_text(text), None);
        assert_eq!(ecu.rev_limit, factory.rev_limit);
        assert_eq!(ecu.idle_rpm, 900.0);
        // A spark map short of values and a λ map with the wrong axes stay as they were.
        assert_eq!(ecu.spark.values(), factory.spark.values());
        assert_eq!(ecu.lambda.values(), factory.lambda.values());
    }
}
