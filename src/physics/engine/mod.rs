//! A mean-value engine model: cycle-averaged flows and torque, cheap enough for every
//! physics step.
//!
//! Air flows through the filter and throttle (and the idle valve) into the intake
//! manifold, whose pressure is the model's state; the cylinders draw from it at a rate
//! set by the volumetric efficiency. The ECU sees only the sensors and commands fuel,
//! spark and the idle valve. Torque comes from the fuel's energy that the trapped air can
//! burn, through the indicated efficiency and the spark and mixture curves, less pumping
//! and friction. What depends on the cylinder's insides comes from maps baked per engine
//! build (`maps`); the parts between the manifolds and the outside act live.

pub mod ecu;
pub mod hardware;
pub mod maps;
mod table;

use std::f32::consts::PI;

use ecu::{Actuators, Ecu, EcuState, Sensors};
use hardware::EngineHardware;
use maps::EngineMaps;

pub const AMBIENT_PRESSURE: f32 = 101_325.0;
pub const AMBIENT_TEMPERATURE: f32 = 298.0;
/// Gas constant of air (J/(kg·K)) and its ratio of specific heats.
const R_AIR: f32 = 287.0;
const GAMMA: f32 = 1.4;
/// Air-fuel mass ratio of a stoichiometric (λ = 1) petrol mixture.
const STOICHIOMETRIC: f32 = 14.7;
/// Lower heating value of petrol (J/kg).
const FUEL_ENERGY: f32 = 44e6;
/// Discharge coefficient of the throttle and idle valve, and the throttle plate's angle
/// when closed (it still leaks a little).
const THROTTLE_DISCHARGE: f32 = 0.8;
const THROTTLE_CLOSED: f32 = 7.0 * PI / 180.0;
const THROTTLE_LEAK: f32 = 8e-6;
/// Steps of the intake manifold per call (its filling is stiff at high rpm).
const SUBSTEPS: usize = 4;
/// Intake manifold pressure after the starter has cranked the engine (Pa).
const CRANKED_PRESSURE: f32 = 30e3;

fn ambient_density() -> f32 {
    AMBIENT_PRESSURE / (R_AIR * AMBIENT_TEMPERATURE)
}

/// An engine as built and tuned: its parts, the maps baked from them, and its ECU.
#[derive(Clone, Debug)]
pub struct EngineBuild {
    pub hardware: EngineHardware,
    pub maps: EngineMaps,
    pub ecu: Ecu,
}

impl EngineBuild {
    /// Bakes the maps for `hardware` and gives it a factory tune.
    pub fn new(hardware: EngineHardware) -> Self {
        let maps = EngineMaps::bake(&hardware);
        let ecu = Ecu::calibrated(&hardware, &maps);
        Self {
            hardware,
            maps,
            ecu,
        }
    }
}

impl Default for EngineBuild {
    fn default() -> Self {
        Self::new(EngineHardware::default())
    }
}

/// The engine's state while running.
#[derive(Clone, Copy, Debug)]
pub struct Engine {
    /// Intake and exhaust manifold pressures (Pa).
    pub intake_pressure: f32,
    pub exhaust_pressure: f32,
    /// Air through the throttle (kg/s), for the filter's pressure drop.
    throttle_flow: f32,
    ecu: EcuState,
    /// What the ECU commanded in the last step.
    pub actuators: Actuators,
    /// The cylinders fire: the engine runs and the ignition isn't cut.
    firing: bool,
}

/// One cycle (two revolutions) of all cylinders.
pub struct Cycle {
    /// Air trapped, relative to filling the displacement at ambient conditions.
    pub load: f32,
    /// Air-fuel ratio (infinite without fuel).
    pub lambda: f32,
    /// Work done by the gas on the pistons, and lost to pumping and friction (J).
    pub indicated: f32,
    pub losses: f32,
    /// Gas leaving through the exhaust valves (kg) and its temperature (K).
    pub exhaust: f32,
    pub exhaust_temperature: f32,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            intake_pressure: CRANKED_PRESSURE,
            exhaust_pressure: AMBIENT_PRESSURE,
            throttle_flow: 0.0,
            ecu: EcuState::default(),
            actuators: Actuators::default(),
            firing: true,
        }
    }
}

impl Engine {
    /// Cranking with the starter pumps the intake manifold down before the engine fires.
    pub fn start(&mut self) {
        self.intake_pressure = self.intake_pressure.min(CRANKED_PRESSURE);
    }

    /// Advances by `dt` at `rpm`, with the throttle plate at `throttle` (0..1). Without
    /// `firing`, the engine only pumps air.
    pub fn step(&mut self, build: &EngineBuild, throttle: f32, rpm: f32, firing: bool, dt: f32) {
        let hardware = &build.hardware;
        let sensors = Sensors {
            rpm,
            map: self.intake_pressure,
            throttle,
        };
        self.actuators = build.ecu.step(&mut self.ecu, &sensors, dt);
        self.firing = firing;

        // The intake manifold fills through the throttle and empties into the cylinders.
        // Both flows are linearised in the manifold pressure for an implicit step.
        let intake = &hardware.intake;
        let upstream = AMBIENT_PRESSURE
            - self.throttle_flow.powi(2) / (2.0 * ambient_density() * intake.filter_area.powi(2));
        let area = throttle_area(intake.throttle_diameter, throttle)
            + intake.idle_valve_area * self.actuators.idle_valve
            + THROTTLE_LEAK;
        let fill = R_AIR * AMBIENT_TEMPERATURE / intake.plenum_volume;
        let h = dt / SUBSTEPS as f32;
        for _ in 0..SUBSTEPS {
            let p = self.intake_pressure;
            let draw = self.cylinder_flow(build, rpm, p) / p;
            let flow = orifice_flow(area, upstream, p);
            let slope = (orifice_flow(area, upstream, p + 50.0) - flow) / 50.0;
            let dp = h * fill * (flow - draw * p) / (1.0 + h * fill * (draw - slope));
            self.intake_pressure = (p + dp).clamp(1e3, upstream);
            self.throttle_flow = flow;
        }

        let cycle = self.cycle(build, rpm);
        let flow = cycle.exhaust * rpm.abs() / 120.0;
        self.exhaust_pressure = exhaust_pressure(hardware, flow, cycle.exhaust_temperature);
    }

    /// Torque on the crank at `rpm`. A stopped engine only drags, against the way it's
    /// turned (fading out over the last few rpm, so it holds still rather than chattering).
    pub fn torque(&self, build: &EngineBuild, rpm: f32) -> f32 {
        let cycle = self.cycle(build, rpm);
        if self.firing {
            (cycle.indicated - cycle.losses) / (4.0 * PI)
        } else {
            -cycle.losses / (4.0 * PI) * (rpm / 30.0).clamp(-1.0, 1.0)
        }
    }

    /// Air the cylinders draw (kg/s) at `rpm` with the intake manifold at `intake` (Pa).
    fn cylinder_flow(&self, build: &EngineBuild, rpm: f32, intake: f32) -> f32 {
        let a = &self.actuators;
        let ratio = self.exhaust_pressure / intake;
        let ve = build.maps.ve(
            a.cam_profile,
            rpm.abs(),
            intake,
            ratio,
            a.intake_phase,
            a.exhaust_phase,
        );
        ve * intake / (R_AIR * AMBIENT_TEMPERATURE) * build.hardware.displacement() * rpm.abs()
            / 120.0
    }

    /// What one cycle does at `rpm` in the current state.
    pub fn cycle(&self, build: &EngineBuild, rpm: f32) -> Cycle {
        let (hardware, maps) = (&build.hardware, &build.maps);
        let rpm = rpm.abs();
        let a = &self.actuators;
        let p = self.intake_pressure;
        let ve = maps.ve(
            a.cam_profile,
            rpm,
            p,
            self.exhaust_pressure / p,
            a.intake_phase,
            a.exhaust_phase,
        );
        let displacement = hardware.displacement();
        let air = ve * p / (R_AIR * AMBIENT_TEMPERATURE) * displacement;
        let load = air / (ambient_density() * displacement);
        let fuel = if self.firing {
            a.fuel * hardware.cylinders as f32
        } else {
            0.0
        };
        let (lambda, indicated, exhaust_temperature) = if fuel > 0.0 {
            let lambda = air / (fuel * STOICHIOMETRIC);
            let offset = a.spark - maps.mbt(rpm, load);
            let indicated = air / STOICHIOMETRIC
                * FUEL_ENERGY
                * maps.efficiency(rpm, load)
                * maps::spark_efficiency(offset)
                * maps::lambda_efficiency(lambda);
            let temperature = maps.exhaust_temperature(rpm, load)
                + maps::exhaust_temperature_shift(offset, lambda);
            (lambda, indicated, temperature)
        } else {
            (f32::INFINITY, 0.0, AMBIENT_TEMPERATURE)
        };
        let pumping = (self.exhaust_pressure - p) * displacement;
        let friction = maps.friction(rpm) * displacement;
        Cycle {
            load,
            lambda,
            indicated,
            losses: pumping + friction,
            exhaust: air + fuel,
            exhaust_temperature,
        }
    }
}

/// Effective flow area of a butterfly throttle opened to `throttle` (0..1) of its travel.
fn throttle_area(diameter: f32, throttle: f32) -> f32 {
    let angle = THROTTLE_CLOSED + throttle.clamp(0.0, 1.0) * (PI / 2.0 - THROTTLE_CLOSED);
    PI / 4.0 * diameter * diameter * (1.0 - angle.cos() / THROTTLE_CLOSED.cos())
}

/// Compressible flow (kg/s) through an orifice of effective `area` from `upstream` to
/// `downstream` pressure, with air at ambient temperature; choked below the critical
/// ratio. Near equal pressures the flow is made linear, which keeps its slope finite.
fn orifice_flow(area: f32, upstream: f32, downstream: f32) -> f32 {
    let psi = |ratio: f32| {
        let choked = 0.528;
        let ratio = ratio.max(choked);
        let g = GAMMA;
        (2.0 * g / (g - 1.0) * (ratio.powf(2.0 / g) - ratio.powf((g + 1.0) / g))).sqrt()
    };
    let ratio = (downstream / upstream).clamp(0.0, 1.0);
    let psi = if ratio > 0.99 {
        psi(0.99) * (1.0 - ratio) / 0.01
    } else {
        psi(ratio)
    };
    THROTTLE_DISCHARGE * area * upstream / (R_AIR * AMBIENT_TEMPERATURE).sqrt() * psi
}

/// Exhaust manifold pressure (Pa) that pushes `flow` (kg/s) of gas at `temperature` (K)
/// through the exhaust's restrictions to ambient.
fn exhaust_pressure(hardware: &EngineHardware, flow: f32, temperature: f32) -> f32 {
    // The drop is ṁ²·R·T·resistance / (2p) at the manifold's pressure p: solve for p.
    let k = flow * flow * R_AIR * temperature * hardware.exhaust.resistance() / 2.0;
    (AMBIENT_PRESSURE + (AMBIENT_PRESSURE.powi(2) + 4.0 * k).sqrt()) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Torque after settling at a fixed `rpm` and `throttle`.
    fn steady_torque(build: &EngineBuild, rpm: f32, throttle: f32) -> f32 {
        let mut engine = Engine::default();
        for _ in 0..240 {
            engine.step(build, throttle, rpm, true, 1.0 / 120.0);
        }
        engine.torque(build, rpm)
    }

    #[test]
    fn full_throttle_torque_curve() {
        let build = EngineBuild::default();
        let torque = |rpm| steady_torque(&build, rpm, 1.0);
        let curve: Vec<_> = [1000.0, 2000.0, 3000.0, 4200.0, 5000.0, 6000.0, 6800.0]
            .map(|rpm| (rpm, torque(rpm).round()))
            .to_vec();
        let peak = curve.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!((3500.0..=5500.0).contains(&peak.0), "{curve:?}");
        assert!((180.0..=230.0).contains(&peak.1), "{curve:?}");
        assert!(curve.iter().all(|&(_, t)| t > 120.0), "{curve:?}");
    }

    #[test]
    fn closed_throttle_brakes_and_idles() {
        let build = EngineBuild::default();
        assert!(steady_torque(&build, 3000.0, 0.0) < -20.0);
        // At idle, the idle valve settles the engine where it makes no net torque.
        let mut engine = Engine::default();
        let mut rpm = 1200.0;
        for _ in 0..(5.0 * 120.0) as usize {
            engine.step(&build, 0.0, rpm, true, 1.0 / 120.0);
            let torque = engine.torque(&build, rpm);
            rpm += torque / build.hardware.inertia / 120.0 * 60.0 / (2.0 * PI);
        }
        assert!((rpm - build.ecu.idle_rpm).abs() < 30.0, "rpm {rpm}");
        let pressure = engine.intake_pressure;
        assert!((20e3..45e3).contains(&pressure), "{pressure} Pa");
    }

    #[test]
    fn the_factory_tune_hits_its_mixture() {
        let build = EngineBuild::default();
        for (rpm, throttle, target) in [(2500.0, 0.1, 1.0), (5000.0, 1.0, 0.88)] {
            let mut engine = Engine::default();
            for _ in 0..240 {
                engine.step(&build, throttle, rpm, true, 1.0 / 120.0);
            }
            let lambda = engine.cycle(&build, rpm).lambda;
            assert!((lambda - target).abs() < 0.02, "λ {lambda} at {rpm} rpm");
        }
    }

    #[test]
    fn a_freer_exhaust_makes_more_top_end() {
        let stock = EngineBuild::default();
        let mut hardware = EngineHardware::default();
        hardware.exhaust.cat_area = None;
        hardware.exhaust.silencer_area = None;
        // Same ECU: the mixture leans out a little, but the engine breathes better.
        let free = EngineBuild {
            hardware,
            ..stock.clone()
        };
        assert!(steady_torque(&free, 6500.0, 1.0) > steady_torque(&stock, 6500.0, 1.0) + 2.0);
    }

    #[test]
    fn retarded_spark_costs_torque_and_heats_the_exhaust() {
        let stock = EngineBuild::default();
        let mut retarded = stock.clone();
        retarded.ecu.spark = table::Table::bake(vec![vec![0.0], vec![0.0]], |_| 0.0);
        let rpm = 4000.0;
        let torque = |build| steady_torque(build, rpm, 1.0);
        assert!(torque(&retarded) < torque(&stock) * 0.85);
        let temperature = |build: &EngineBuild| {
            let mut engine = Engine::default();
            for _ in 0..240 {
                engine.step(build, 1.0, rpm, true, 1.0 / 120.0);
            }
            engine.cycle(build, rpm).exhaust_temperature
        };
        assert!(temperature(&retarded) > temperature(&stock) + 100.0);
    }
}
