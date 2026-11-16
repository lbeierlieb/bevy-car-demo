//! A mean-value engine model: cycle-averaged flows and torque, cheap enough for every
//! physics step.
//!
//! Air flows through the filter (and the turbo's compressor and intercooler) into the
//! ducts ahead of the throttle, and through the throttle (and the idle valve) into the
//! intake manifold; the pressures in both are the model's state. The cylinders draw from
//! the manifold at a rate set by the volumetric efficiency, and push the exhaust through
//! the turbine and wastegate and the exhaust's restrictions. The ECU sees only the
//! sensors and commands fuel,
//! spark and the idle valve. Torque comes from the fuel's energy that the trapped air can
//! burn, through the indicated efficiency and the spark and mixture curves, less pumping
//! and friction. What depends on the cylinder's insides comes from maps baked per engine
//! build (`maps`); the parts between the manifolds and the outside act live.

pub mod cycle;
pub mod damage;
pub mod ecu;
pub mod hardware;
pub mod maps;
pub mod parts;
pub mod table;
pub mod thermal;
pub mod turbo;

use std::f32::consts::PI;

use damage::{Damage, Strain};
use ecu::{Actuators, Ecu, EcuState, Sensors};
use hardware::EngineHardware;
use maps::EngineMaps;
use thermal::Thermal;

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
/// Steps of the intake per call (its filling is stiff at high rpm).
const SUBSTEPS: usize = 4;
/// Pressure step for the flows' slopes in the implicit intake step (Pa).
const SLOPE_STEP: f32 = 50.0;
/// Turbo shaft: bearing drag per rad/s (N·m·s), and the fastest it may spin relative to
/// its limit (its wheel bursts before).
const BEARING_DRAG: f32 = 5e-7;
const TURBO_LIMIT: f32 = 1.2;
/// Time constants of the wastegate actuator and of the intercooler's heat soak (s), and
/// the air flow at which the intercooler follows at its full rate (kg/s).
const WASTEGATE_LAG: f32 = 0.05;
const INTERCOOLER_LAG: f32 = 2.0;
const INTERCOOLER_FLOW: f32 = 0.05;
/// Volumetric efficiency lost per unit of valve float (spring force short of what the
/// valves' inertia needs, relative to it), and the least that's left.
const FLOAT_LOSS: f32 = 2.5;
const FLOAT_LEAST: f32 = 0.1;
/// Friction mean effective pressure per newton of valve spring force over the stock
/// springs' (Pa/N), which the friction map includes.
const SPRING_FRICTION: f32 = 20.0;
const STOCK_SPRING_FORCE: f32 = 370.0;
/// Rise of the highest cylinder pressure per degree of spark advance from MBT (relative),
/// and most it rises (with the burn all around top dead centre); and the rise per degree
/// of knock (the pressure waves of the end gas exploding), which hits the head gasket
/// and the pistons rather than the rods.
const PEAK_PER_DEGREE: f32 = 0.03;
const PEAK_MOST: f32 = 0.35;
const KNOCK_PEAK: f32 = 0.03;
/// Time over which the highest cylinder pressure is averaged for the wear (s): a step's
/// spike (the ECU's spark lagging a sudden change) shouldn't break anything.
const PEAK_LAG: f32 = 0.1;
/// Intake manifold pressure after the starter has cranked the engine (Pa).
const CRANKED_PRESSURE: f32 = 30e3;

fn ambient_density() -> f32 {
    AMBIENT_PRESSURE / (R_AIR * AMBIENT_TEMPERATURE)
}

/// An engine as built and tuned: its parts, the maps baked from them, its ECU, and the
/// fuel it runs on.
#[derive(Clone, Debug)]
pub struct EngineBuild {
    pub hardware: EngineHardware,
    pub maps: EngineMaps,
    pub ecu: Ecu,
    /// Research octane number of the fuel.
    pub octane: f32,
}

impl EngineBuild {
    /// The maps for `hardware` (baked now unless it's a stock build) and a factory tune for
    /// 95 octane.
    pub fn new(hardware: EngineHardware) -> Self {
        let octane = 95.0;
        let maps = EngineMaps::for_hardware(&hardware);
        let ecu = Ecu::calibrated(&hardware, &maps, octane, ecu::EcuModel::Stock);
        Self {
            hardware,
            maps,
            ecu,
            octane,
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
    /// Pressure ahead of the throttle: boost after the intercooler, or just below ambient
    /// without a turbo (Pa).
    pub boost_pressure: f32,
    /// Intake and exhaust manifold pressures (Pa).
    pub intake_pressure: f32,
    pub exhaust_pressure: f32,
    /// Temperature of the air ahead of and behind the throttle (K).
    pub charge_temperature: f32,
    /// Turbo shaft speed (rad/s), wastegate opening (0..1), and air through the compressor
    /// (kg/s, for the pressure drops of the filter and the intercooler).
    pub turbo_speed: f32,
    pub wastegate: f32,
    compressor_flow: f32,
    ecu: EcuState,
    /// What the ECU commanded in the last step.
    pub actuators: Actuators,
    /// The cylinders fire: the engine runs and the ignition isn't cut.
    firing: bool,
    pub thermal: Thermal,
    /// Speed of the air the car drives through (m/s), for the radiator; set by the car.
    pub airspeed: f32,
    /// Wear and failures of the parts.
    pub damage: Damage,
    /// Highest cylinder pressure, averaged over `PEAK_LAG` (Pa).
    pub peak_pressure: f32,
}

/// One cycle (two revolutions) of all cylinders.
pub struct Cycle {
    /// Air trapped, relative to filling the displacement at ambient conditions.
    pub load: f32,
    /// Air-fuel ratio (infinite without fuel).
    pub lambda: f32,
    /// Fuel burnt (kg).
    pub fuel: f32,
    /// Work done by the gas on the pistons, lost to friction, and lost to pumping and
    /// friction together (J).
    pub indicated: f32,
    pub friction: f32,
    pub losses: f32,
    /// Gas leaving through the exhaust valves (kg) and its temperature (K).
    pub exhaust: f32,
    pub exhaust_temperature: f32,
    /// Spark advance past the knock limit (degrees; 0 without knock).
    pub knock: f32,
    /// Highest cylinder pressure, without the knock's pressure waves (Pa).
    pub peak_pressure: f32,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            boost_pressure: AMBIENT_PRESSURE,
            intake_pressure: CRANKED_PRESSURE,
            exhaust_pressure: AMBIENT_PRESSURE,
            charge_temperature: AMBIENT_TEMPERATURE,
            turbo_speed: 0.0,
            wastegate: 0.0,
            compressor_flow: 0.0,
            ecu: EcuState::default(),
            actuators: Actuators::default(),
            firing: true,
            thermal: Thermal::warm(),
            airspeed: 0.0,
            damage: Damage::default(),
            peak_pressure: AMBIENT_PRESSURE,
        }
    }
}

impl Engine {
    /// Wrecked: it won't fire again until it's repaired.
    pub fn blown(&self, hardware: &EngineHardware) -> bool {
        self.damage.wrecked(hardware.cylinders as usize)
    }

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
            intake_temperature: self.charge_temperature,
        };
        self.actuators = build.ecu.step(&mut self.ecu, &sensors, dt);
        self.firing = firing && !self.blown(hardware);

        // The ducts fill through the filter (or the compressor) and empty through the
        // throttle into the manifold, which the cylinders empty. All flows are linearised
        // in both pressures for an implicit step.
        let intake = &hardware.intake;
        let temperature = self.charge_temperature;
        let area = THROTTLE_DISCHARGE
            * (throttle_area(intake.throttle_diameter, throttle)
                + intake.idle_valve_area * self.actuators.idle_valve
                + THROTTLE_LEAK);
        let tract_fill = R_AIR * temperature / intake.tract_volume;
        let manifold_fill = R_AIR * temperature / intake.plenum_volume;
        let h = dt / SUBSTEPS as f32;
        let d = SLOPE_STEP;
        for _ in 0..SUBSTEPS {
            let (pt, pm) = (self.boost_pressure, self.intake_pressure);
            let through = |pt, pm| orifice_flow(area, pt, pm, temperature);
            let (compressed, vented) = self.tract_inflow(hardware, pt);
            let (fill, pass, draw) = (
                compressed - vented,
                through(pt, pm),
                self.cylinder_flow(build, rpm, pm),
            );
            let (compressed_up, vented_up) = self.tract_inflow(hardware, pt + d);
            let fill_slope = (compressed_up - vented_up - fill) / d;
            let pass_up = (through(pt + d, pm) - pass) / d;
            let pass_down = (through(pt, pm + d) - pass) / d;
            let draw_slope = (self.cylinder_flow(build, rpm, pm + d) - draw) / d;
            let (ht, hm) = (h * tract_fill, h * manifold_fill);
            let (a, b) = (1.0 - ht * (fill_slope - pass_up), ht * pass_down);
            let (c, e) = (-hm * pass_up, 1.0 - hm * (pass_down - draw_slope));
            let (rt, rm) = (ht * (fill - pass), hm * (pass - draw));
            let det = a * e - b * c;
            self.boost_pressure = (pt + (rt * e - b * rm) / det).clamp(1e3, 1e6);
            self.intake_pressure = (pm + (a * rm - c * rt) / det).clamp(1e3, 1e6);
            self.compressor_flow = compressed;
        }

        let cycle = self.cycle(build, rpm);
        let cycles = rpm.abs() / 120.0;
        let flow = cycle.exhaust * cycles;
        let gas = cycle.exhaust_temperature;
        self.exhaust_pressure = exhaust_pressure(hardware, flow, gas, self.wastegate);
        if let Some(turbo) = &hardware.turbo {
            self.step_turbo(turbo, hardware, flow, gas, dt);
        }

        // The cylinder walls heat the coolant; friction heats the coolant (mostly through
        // the liners) and the oil (through the bearings), and the oil also cools the
        // pistons.
        let cylinders = hardware.cylinders as usize;
        let fuel_power = cycle.fuel * FUEL_ENERGY * cycles * self.damage.firing(cylinders);
        let friction_power = cycle.friction * cycles;
        let walls = build.maps.heat_to_coolant(rpm.abs(), cycle.load);
        let to_coolant =
            (walls + self.damage.extra_coolant_heat()) * fuel_power + 0.7 * friction_power;
        let to_oil = 0.3 * friction_power + 0.02 * fuel_power;
        let (cooling, airspeed) = (&hardware.cooling, self.airspeed);
        self.thermal.step(cooling, to_coolant, to_oil, airspeed, dt);

        self.peak_pressure += (cycle.peak_pressure - self.peak_pressure) * (dt / PEAK_LAG).min(1.0);
        let strain = Strain {
            rpm: rpm.abs(),
            float: self.valve_float(hardware, rpm.abs()),
            rod: hardware.rod_load(rpm.abs(), self.peak_pressure),
            gasket: self.peak_pressure * (1.0 + KNOCK_PEAK * cycle.knock) / hardware.head_gasket,
            turbo_speed: hardware
                .turbo
                .map_or(0.0, |turbo| self.turbo_speed / turbo.max_speed()),
            turbine_temperature: gas,
            knock: cycle.knock,
            load: cycle.load,
            coolant: self.thermal.coolant,
            oil: self.thermal.oil,
        };
        self.damage.step(hardware, &strain, dt);
    }

    /// Air into the ducts ahead of the throttle with them at `pressure` (Pa): through the
    /// filter, or through the filter, compressor and intercooler; and air vented from them
    /// by the diverter valve (both kg/s).
    fn tract_inflow(&self, hardware: &EngineHardware, pressure: f32) -> (f32, f32) {
        let filter = hardware.intake.filter_area;
        let Some(turbo) = &hardware.turbo else {
            let flow = orifice_flow(filter, AMBIENT_PRESSURE, pressure, AMBIENT_TEMPERATURE);
            return (flow, 0.0);
        };
        let (inlet, outlet) = self.compressor_pressures(hardware, turbo, pressure);
        let speed = self.turbo_speed;
        let compressor = &turbo.compressor;
        let flow = turbo::compressor(compressor, speed, inlet, outlet, AMBIENT_TEMPERATURE).flow;
        let vented = turbo.diverter.map_or(0.0, |valve| {
            let open = (pressure - self.intake_pressure - valve.opens) / valve.span;
            let area = valve.area * open.clamp(0.0, 1.0);
            orifice_flow(area, pressure, inlet, self.charge_temperature)
        });
        (flow, vented)
    }

    /// Pressures at the compressor's inlet and outlet with the ducts at `pressure`: less
    /// the filter's drop ahead of it, plus the intercooler's after it.
    fn compressor_pressures(
        &self,
        hardware: &EngineHardware,
        turbo: &hardware::Turbo,
        pressure: f32,
    ) -> (f32, f32) {
        let drop =
            |area: f32| self.compressor_flow.powi(2) / (2.0 * ambient_density() * area * area);
        let inlet = AMBIENT_PRESSURE - drop(hardware.intake.filter_area);
        (inlet, pressure + drop(turbo.intercooler.area))
    }

    /// The turbo shaft between turbine and compressor, the wastegate following the boost,
    /// and the charge temperature following the intercooler (with some heat soak).
    fn step_turbo(
        &mut self,
        turbo: &hardware::Turbo,
        hardware: &EngineHardware,
        exhaust_flow: f32,
        gas: f32,
        dt: f32,
    ) {
        let (inlet, outlet) = self.compressor_pressures(hardware, turbo, self.boost_pressure);
        let speed = self.turbo_speed;
        let compressor =
            turbo::compressor(&turbo.compressor, speed, inlet, outlet, AMBIENT_TEMPERATURE);
        let after = restricted(hardware, exhaust_flow, gas);
        let (_, drive) = turbo::turbine(&turbo.turbine, speed, self.exhaust_pressure, after, gas);
        let net = drive - compressor.torque - BEARING_DRAG * speed;
        let limit = TURBO_LIMIT * turbo.max_speed();
        self.turbo_speed = if self.damage.turbo_failed() {
            0.0
        } else {
            (speed + net / turbo.inertia * dt).clamp(0.0, limit)
        };

        let target =
            turbo::wastegate_opening(&turbo.wastegate, self.boost_pressure - AMBIENT_PRESSURE);
        self.wastegate += (target - self.wastegate) * (dt / WASTEGATE_LAG).min(1.0);
        let cooled = turbo::intercooled(turbo, compressor.temperature);
        let rate = (compressor.flow / INTERCOOLER_FLOW).min(1.0) * dt / INTERCOOLER_LAG;
        self.charge_temperature += (cooled - self.charge_temperature) * rate.min(1.0);
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

    /// The cam profile the valvetrain is on.
    fn cam(&self, hardware: &EngineHardware) -> hardware::CamProfile {
        hardware.cams[self.actuators.cam_profile.min(hardware.cams.len() - 1)]
    }

    /// How far past valve float the valvetrain is at `rpm`: the force the valves' inertia
    /// needs over what the springs have, less one (0 below the float speed).
    pub fn valve_float(&self, hardware: &EngineHardware, rpm: f32) -> f32 {
        let float = hardware.valves.float_rpm(&self.cam(hardware));
        ((rpm / float).powi(2) - 1.0).max(0.0)
    }

    /// Volumetric efficiency at `rpm` with the intake manifold at `intake` (Pa), as the
    /// actuators set the cams; floating valves lose it.
    fn ve(&self, build: &EngineBuild, rpm: f32, intake: f32) -> f32 {
        let a = &self.actuators;
        let ve = build.maps.ve(
            a.cam_profile,
            rpm,
            intake,
            self.exhaust_pressure / intake,
            a.intake_phase,
            a.exhaust_phase,
        );
        let float = self.valve_float(&build.hardware, rpm);
        ve * (1.0 - FLOAT_LOSS * float).max(FLOAT_LEAST)
    }

    /// Air the cylinders draw (kg/s) at `rpm` with the intake manifold at `intake` (Pa).
    fn cylinder_flow(&self, build: &EngineBuild, rpm: f32, intake: f32) -> f32 {
        let ve = self.ve(build, rpm.abs(), intake);
        ve * intake / (R_AIR * self.charge_temperature) * build.hardware.displacement() * rpm.abs()
            / 120.0
    }

    /// What one cycle does at `rpm` in the current state.
    pub fn cycle(&self, build: &EngineBuild, rpm: f32) -> Cycle {
        let (hardware, maps) = (&build.hardware, &build.maps);
        let rpm = rpm.abs();
        let a = &self.actuators;
        let p = self.intake_pressure;
        let ve = self.ve(build, rpm, p);
        let displacement = hardware.displacement();
        let air = ve * p / (R_AIR * self.charge_temperature) * displacement;
        let load = air / (ambient_density() * displacement);
        let fuel = if self.firing {
            a.fuel * hardware.cylinders as f32
        } else {
            0.0
        };
        let cylinders = hardware.cylinders as usize;
        let firing = self.damage.firing(cylinders);
        // Compressed without burning, the charge peaks at top dead centre.
        let motored = p * hardware.compression_ratio.powf(1.3);
        let (lambda, indicated, exhaust_temperature, knock, peak) = if fuel > 0.0 && firing > 0.0 {
            let lambda = air / (fuel * STOICHIOMETRIC);
            let offset = a.spark - maps.mbt(rpm, load);
            let indicated = air / STOICHIOMETRIC
                * FUEL_ENERGY
                * maps.efficiency(rpm, load)
                * maps::spark_efficiency(offset)
                * maps::lambda_efficiency(lambda)
                * self.damage.efficiency(cylinders)
                * firing;
            // The dead cylinders pump their charge through unburnt.
            let burnt = maps.exhaust_temperature(rpm, load)
                + maps::exhaust_temperature_shift(offset, lambda);
            let temperature = firing * burnt + (1.0 - firing) * self.charge_temperature;
            let (charge, coolant) = (self.charge_temperature, self.thermal.coolant);
            let shift = maps::knock_limit_shift(charge, coolant, build.octane);
            let knock = (a.spark - maps.knock_limit(rpm, load) - shift).max(0.0);
            let rise = PEAK_MOST * (PEAK_PER_DEGREE * offset / PEAK_MOST).tanh();
            let peak = maps.peak_pressure(rpm, load) * (1.0 + rise);
            (lambda, indicated, temperature, knock, peak.max(motored))
        } else {
            (f32::INFINITY, 0.0, AMBIENT_TEMPERATURE, 0.0, motored)
        };
        let pumping = (self.exhaust_pressure - p) * displacement;
        let springs = hardware.valves.force(self.cam(hardware).lift) - STOCK_SPRING_FORCE;
        let friction =
            (maps.friction(rpm, self.thermal.oil) + SPRING_FRICTION * springs) * displacement;
        Cycle {
            load,
            lambda,
            fuel,
            indicated,
            friction,
            losses: pumping + friction,
            exhaust: air + fuel,
            exhaust_temperature,
            knock,
            peak_pressure: peak,
        }
    }
}

/// Effective flow area of a butterfly throttle opened to `throttle` (0..1) of its travel.
fn throttle_area(diameter: f32, throttle: f32) -> f32 {
    let angle = THROTTLE_CLOSED + throttle.clamp(0.0, 1.0) * (PI / 2.0 - THROTTLE_CLOSED);
    PI / 4.0 * diameter * diameter * (1.0 - angle.cos() / THROTTLE_CLOSED.cos())
}

/// Compressible flow (kg/s) through an orifice of effective `area` from `upstream` to
/// `downstream` pressure, with gas at `temperature` upstream; choked below the critical
/// ratio. Near equal pressures the flow is made linear, which keeps its slope finite.
fn orifice_flow(area: f32, upstream: f32, downstream: f32, temperature: f32) -> f32 {
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
    area * upstream / (R_AIR * temperature).sqrt() * psi
}

/// Pressure (Pa) that pushes `flow` (kg/s) of gas at `temperature` (K) through the
/// exhaust's restrictions to ambient.
fn restricted(hardware: &EngineHardware, flow: f32, temperature: f32) -> f32 {
    // The drop is ṁ²·R·T·resistance / (2p) at the pressure p: solve for p.
    let k = flow * flow * R_AIR * temperature * hardware.exhaust.resistance() / 2.0;
    (AMBIENT_PRESSURE + (AMBIENT_PRESSURE.powi(2) + 4.0 * k).sqrt()) / 2.0
}

/// Exhaust manifold pressure (Pa) that pushes `flow` (kg/s) of gas at `temperature` (K)
/// out: through the turbine and the wastegate (open by `wastegate`, 0..1), if there's a
/// turbo, and then the restrictions.
fn exhaust_pressure(hardware: &EngineHardware, flow: f32, temperature: f32, wastegate: f32) -> f32 {
    let after = restricted(hardware, flow, temperature);
    let Some(turbo) = &hardware.turbo else {
        return after;
    };
    let gate = turbo.wastegate.area * wastegate;
    let passes = |p| {
        orifice_flow(turbo.turbine.area, p, after, temperature)
            + orifice_flow(gate, p, after, temperature)
    };
    let (mut low, mut high) = (after, after + 1e5);
    while passes(high) < flow && high < 1e7 {
        high = after + 2.0 * (high - after);
    }
    for _ in 0..30 {
        let mid = (low + high) / 2.0;
        if passes(mid) < flow {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
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
    fn sustained_full_load_stays_in_the_safe_range() {
        // Flat out at speed for two minutes, naturally aspirated and turbo.
        for build in [
            EngineBuild::default(),
            EngineBuild::new(EngineHardware::turbo()),
        ] {
            let mut engine = Engine {
                airspeed: 40.0,
                ..Engine::default()
            };
            for _ in 0..120 * 120 {
                engine.step(&build, 1.0, 6000.0, true, 1.0 / 120.0);
            }
            let (coolant, oil) = (engine.thermal.coolant - 273.15, engine.thermal.oil - 273.15);
            assert!(
                coolant < 105.0 && oil < 135.0,
                "coolant {coolant} °C, oil {oil} °C"
            );
            let wear = engine.damage.worst(4);
            assert!(wear < 1e-3, "wear {wear}");
        }
    }

    #[test]
    fn the_turbo_boosts_up_to_its_wastegate() {
        let na = EngineBuild::default();
        let build = EngineBuild::new(EngineHardware::turbo());
        let rpm = 5000.0;
        let engine = run_flat_out(&build, rpm, 6.0);
        let boost = (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5;
        assert!((0.6..0.9).contains(&boost), "{boost} bar");
        assert!(engine.wastegate > 0.2, "wastegate {}", engine.wastegate);
        let torque = engine.torque(&build, rpm);
        assert!(torque > 1.3 * steady_torque(&na, rpm, 1.0), "{torque} N·m");
        // Intercooled, tuned for it: warm charge, rich mixture, no knock.
        let cycle = engine.cycle(&build, rpm);
        let charge = engine.charge_temperature - 273.15;
        assert!((30.0..70.0).contains(&charge), "charge {charge} °C");
        assert!((0.76..0.86).contains(&cycle.lambda), "λ {}", cycle.lambda);
        assert_eq!(cycle.knock, 0.0);
        // Low down there's little exhaust to drive it.
        let low = run_flat_out(&build, 1500.0, 6.0);
        assert!(low.boost_pressure - AMBIENT_PRESSURE < 0.1e5);
    }

    #[test]
    fn the_turbo_spools_and_vents() {
        let build = EngineBuild::new(EngineHardware::turbo());
        let rpm = 3500.0;
        let mut engine = Engine::default();
        let run = |engine: &mut Engine, throttle, seconds: f32| {
            for _ in 0..(seconds * 120.0) as usize {
                engine.step(&build, throttle, rpm, true, 1.0 / 120.0);
            }
            (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5
        };
        assert!(run(&mut engine, 0.15, 5.0) < 0.1);
        // Floored: boost builds within a couple of tenths, most of it within a second
        // and a half.
        let early = run(&mut engine, 1.0, 0.2);
        let later = run(&mut engine, 1.0, 1.3);
        assert!(
            early < later * 0.5 && later > 0.4,
            "{early} then {later} bar"
        );
        // Lifting: the diverter vents the boost, and the shaft spins down.
        let speed = engine.turbo_speed;
        assert!(run(&mut engine, 0.0, 0.5) < 0.15);
        assert!(engine.turbo_speed < speed * 0.8);
    }

    #[test]
    fn full_throttle_torque_curve() {
        let build = EngineBuild::default();
        let torque = |rpm| steady_torque(&build, rpm, 1.0);
        let curve: Vec<_> = [1000.0, 2000.0, 3000.0, 4200.0, 5000.0, 6000.0, 6800.0]
            .map(|rpm| (rpm, torque(rpm).round()))
            .to_vec();
        let peak = curve.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!((3000.0..=5500.0).contains(&peak.0), "{curve:?}");
        assert!((180.0..=230.0).contains(&peak.1), "{curve:?}");
        assert!(curve.iter().all(|&(_, t)| t > 120.0), "{curve:?}");
    }

    #[test]
    fn floating_valves_cap_the_revs_and_bend() {
        let build = EngineBuild::default();
        let float = build.hardware.valves.float_rpm(&build.hardware.cams[0]);
        assert!((7000.0..7500.0).contains(&float), "{float} rpm");
        // Held past the float speed (the rev limiter left behind): the power falls away.
        let below = steady_torque(&build, float - 200.0, 1.0);
        let past = steady_torque(&build, float * 1.05, 1.0);
        assert!(past < 0.85 * below, "{below} then {past} N·m");
        let mut engine = Engine::default();
        for _ in 0..60 {
            engine.step(&build, 1.0, float * 1.05, true, 1.0 / 120.0);
        }
        let damage = engine.damage;
        assert!(
            damage.firing(4) == 1.0 && damage.worst(4) > 0.0,
            "{damage:?}"
        );
        // Bent valves leave the engine running on the other cylinders.
        for _ in 0..240 {
            engine.step(&build, 1.0, float * 1.1, true, 1.0 / 120.0);
        }
        assert!(engine.damage.firing(4) < 1.0);
        assert!(engine.torque(&build, 4000.0) < steady_torque(&build, 4000.0, 1.0));
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

    /// An engine after `seconds` at a fixed `rpm` and full throttle.
    fn run_flat_out(build: &EngineBuild, rpm: f32, seconds: f32) -> Engine {
        let mut engine = Engine::default();
        for _ in 0..(seconds * 120.0) as usize {
            engine.step(build, 1.0, rpm, true, 1.0 / 120.0);
        }
        engine
    }

    #[test]
    fn the_factory_tune_stays_clear_of_knock() {
        let build = EngineBuild::default();
        for rpm in [1500.0, 2500.0, 4000.0, 6000.0] {
            let engine = run_flat_out(&build, rpm, 2.0);
            assert_eq!(engine.cycle(&build, rpm).knock, 0.0, "at {rpm} rpm");
            // At most a trace from the moment of tip-in.
            let wear = engine.damage.worst(4);
            assert!(wear < 1e-3, "wear {wear}");
        }
        // Cheaper fuel than it was tuned for: it knocks lightly at low rpm.
        let cheap = EngineBuild {
            octane: 91.0,
            ..build.clone()
        };
        let knock = run_flat_out(&cheap, 2000.0, 2.0)
            .cycle(&cheap, 2000.0)
            .knock;
        assert!((1.0..8.0).contains(&knock), "knock {knock}°");
    }

    #[test]
    fn heavy_knock_blows_the_engine() {
        let mut build = EngineBuild::default();
        build.ecu.spark = table::Table::bake(vec![vec![0.0], vec![0.0]], |_| 40.0);
        let rpm = 2500.0;
        let engine = run_flat_out(&build, rpm, 5.0);
        assert!(engine.cycle(&build, rpm).knock > 10.0);
        let wear = engine.damage.worst(4);
        assert!(wear > 0.05 && !engine.blown(&build.hardware), "wear {wear}");
        let engine = run_flat_out(&build, rpm, 120.0);
        assert!(engine.blown(&build.hardware));
        assert!(engine.torque(&build, rpm) < 0.0);
        let failures = engine.damage.failures(4);
        assert!(
            failures.iter().all(|f| f.ends_with("holed piston")),
            "{failures:?}"
        );
        // Light knock on cheaper fuel wears the pistons, and the rods hold.
        let cheap = EngineBuild {
            octane: 91.0,
            ..EngineBuild::default()
        };
        let engine = run_flat_out(&cheap, 2000.0, 60.0);
        let pistons = engine.damage.cylinders[2].piston;
        assert!(
            pistons > 0.01 && !engine.blown(&cheap.hardware),
            "{pistons}"
        );
        assert_eq!(engine.damage.cylinders[2].rod, 0.0);
    }

    #[test]
    fn big_boost_needs_a_strong_bottom_end() {
        // Lots of boost on race fuel, with the spark near MBT: the knock no longer holds
        // the cylinder pressure down (the factory tune on pump fuel does).
        let build = |parts: &parts::Parts| {
            let mut build = EngineBuild::new(parts.hardware());
            build.octane = 110.0;
            build.ecu = Ecu::calibrated(&build.hardware, &build.maps, 110.0, parts.ecu);
            build
        };
        let mut parts = parts::Parts::turbo();
        parts.boost = 1.4;
        let stock = build(&parts);
        let engine = run_flat_out(&stock, 5500.0, 20.0);
        assert!(engine.blown(&stock.hardware), "{:?}", engine.damage);
        // Forged internals and a steel gasket hold it.
        parts.chosen[3] = 2;
        parts.chosen[4] = 1;
        let strong = build(&parts);
        let engine = run_flat_out(&strong, 5500.0, 20.0);
        assert!(engine.damage.worst(4) < 0.05, "{:?}", engine.damage);
    }

    #[test]
    fn an_overspun_turbo_bursts() {
        // The small turbo with its wastegate held shut to 1.5 bar: its compressor runs out
        // of breath at high rpm and spins past its limit.
        let mut parts = parts::Parts::turbo();
        parts.chosen[12] = 1;
        parts.boost = 1.5;
        let build = EngineBuild::new(parts.hardware());
        let engine = run_flat_out(&build, 6000.0, 10.0);
        assert!(engine.damage.turbo_failed(), "{:?}", engine.damage);
        assert_eq!(engine.damage.failures(4), ["turbo failed"]);
        // It no longer boosts, but the engine runs on.
        assert!(engine.boost_pressure < AMBIENT_PRESSURE);
        assert!(engine.torque(&build, 6000.0) > 50.0);
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
