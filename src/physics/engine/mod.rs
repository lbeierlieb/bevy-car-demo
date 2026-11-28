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
pub mod families;
pub mod hardware;
pub mod maps;
pub mod parts;
pub mod store;
pub mod supercharger;
pub mod table;
pub mod thermal;
pub mod turbo;

use std::f32::consts::PI;

use damage::{Damage, Strain};
use ecu::{Actuators, Ecu, EcuState, Sensors};
use hardware::EngineHardware;
use maps::EngineMaps;
use thermal::Thermal;

use crate::physics::fuel::PUMP_OCTANE;

pub const AMBIENT_PRESSURE: f32 = 101_325.0;
pub const AMBIENT_TEMPERATURE: f32 = 298.0;
/// Gas constant of air (J/(kg·K)) and its ratio of specific heats.
const R_AIR: f32 = 287.0;
const GAMMA: f32 = 1.4;
/// Air-fuel mass ratio of a stoichiometric (λ = 1) petrol mixture.
const STOICHIOMETRIC: f32 = 14.7;
/// Lower heating value of petrol (J/kg).
const FUEL_ENERGY: f32 = 44e6;
/// Discharge coefficient of the throttle and idle valve, the throttle plate's angle when
/// closed, and the share of the bore's area it still leaks then.
const THROTTLE_DISCHARGE: f32 = 0.8;
const THROTTLE_CLOSED: f32 = 7.0 * PI / 180.0;
const THROTTLE_LEAK: f32 = 0.0025;
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
/// Time constant of the charge's temperature after a supercharger (s).
const CHARGE_LAG: f32 = 0.5;
/// Volumetric efficiency lost per unit of valve float (spring force short of what the
/// valves' inertia needs, relative to it), and the least that's left.
const FLOAT_LOSS: f32 = 2.5;
const FLOAT_LEAST: f32 = 0.1;
/// Friction mean effective pressure per newton of valve spring force over the stock
/// springs' (Pa/N), which the friction map includes: the force as on four valves with
/// flat tappets (`EngineHardware::spring_drag`), so two valves or rollers drag less.
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
    /// Research octane number of the fuel: a car sets it from what's in its tank.
    pub octane: f32,
}

impl EngineBuild {
    /// The maps for `hardware` (baked now unless it's a factory engine) and a factory tune
    /// of a `model` ECU for 95 octane.
    pub fn new(hardware: EngineHardware, model: ecu::EcuModel) -> Self {
        let octane = PUMP_OCTANE;
        let maps = EngineMaps::for_hardware(&hardware);
        let ecu = Ecu::calibrated(&hardware, &maps, octane, model);
        Self {
            hardware,
            maps,
            ecu,
            octane,
        }
    }
}

impl EngineBuild {
    /// The engine `parts` make, with its ECU's tune: a locked ECU's own, else a factory
    /// tune for the engine.
    pub fn of_parts(parts: &parts::Parts) -> Self {
        let def = parts.ecu_def();
        match &def.tune {
            Some(tune) if def.model.locked => {
                let hardware = parts.hardware();
                Self {
                    maps: EngineMaps::for_hardware(&hardware),
                    hardware,
                    ecu: tune.clone(),
                    octane: PUMP_OCTANE,
                }
            }
            _ => Self::new(parts.hardware(), def.model),
        }
    }
}

impl Default for EngineBuild {
    fn default() -> Self {
        Self::new(EngineHardware::default(), ecu::EcuModel::STOCK)
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
    /// Share of the fuel the ECU asks for that the pump delivers (less when it draws air
    /// from a nearly empty tank); set by the car.
    pub fuel_supply: f32,
    /// Fuel injected in the last step (kg/s).
    pub fuel_flow: f32,
    /// The exhaust's λ in the last step (infinite without fuel), for the lambda sensor.
    pub exhaust_lambda: f32,
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
            fuel_supply: 1.0,
            fuel_flow: 0.0,
            exhaust_lambda: 1.0,
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
            lambda: self.exhaust_lambda,
            wideband: hardware.wideband,
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
                + THROTTLE_LEAK * PI / 4.0 * intake.throttle_diameter.powi(2));
        let tract_fill = R_AIR * temperature / intake.tract_volume;
        let manifold_fill = R_AIR * temperature / intake.plenum_volume;
        let h = dt / SUBSTEPS as f32;
        let d = SLOPE_STEP;
        for _ in 0..SUBSTEPS {
            let (pt, pm) = (self.boost_pressure, self.intake_pressure);
            let through = |pt, pm| orifice_flow(area, pt, pm, temperature);
            let (compressed, vented) = self.tract_inflow(hardware, pt, rpm);
            let (fill, pass, draw) = (
                compressed - vented,
                through(pt, pm),
                self.cylinder_flow(build, rpm, pm),
            );
            let (compressed_up, vented_up) = self.tract_inflow(hardware, pt + d, rpm);
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
        self.fuel_flow = cycle.fuel * cycles;
        self.exhaust_lambda = cycle.lambda;
        let flow = cycle.exhaust * cycles;
        let gas = cycle.exhaust_temperature;
        self.exhaust_pressure = exhaust_pressure(hardware, flow, gas, self.wastegate);
        if let Some(turbo) = &hardware.turbo {
            self.step_turbo(turbo, hardware, flow, gas, dt);
        }
        if let Some(charger) = &hardware.supercharger {
            // The charge follows what the supercharger heats it to (no intercooler).
            let inlet = self.filtered(hardware);
            let blown = supercharger::point(
                charger,
                rpm.abs(),
                inlet,
                self.boost_pressure,
                AMBIENT_TEMPERATURE,
            );
            let rate = (blown.flow / INTERCOOLER_FLOW).min(1.0) * dt / CHARGE_LAG;
            self.charge_temperature +=
                (blown.temperature - self.charge_temperature) * rate.min(1.0);
        }

        // The cylinder walls heat the coolant; friction heats the coolant (mostly through
        // the liners) and the oil (through the bearings), and the oil also cools the
        // pistons.
        let cylinders = hardware.cylinders as usize;
        let fuel_power = cycle.fuel * FUEL_ENERGY * cycles * self.damage.firing(cylinders);
        let friction_power = cycle.friction * cycles;
        let profile = self.actuators.cam_profile;
        let walls = build.maps.heat_to_coolant(profile, rpm.abs(), cycle.load);
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
    fn tract_inflow(&self, hardware: &EngineHardware, pressure: f32, rpm: f32) -> (f32, f32) {
        let filter = hardware.intake.filter_area;
        if let Some(charger) = &hardware.supercharger {
            let inlet = self.filtered(hardware);
            let temperature = AMBIENT_TEMPERATURE;
            let flow = supercharger::point(charger, rpm.abs(), inlet, pressure, temperature).flow;
            let valve = charger.bypass;
            let open = (pressure - self.intake_pressure - valve.opens) / valve.span;
            let area = valve.area * open.clamp(0.0, 1.0);
            let bypassed = orifice_flow(area, pressure, inlet, self.charge_temperature);
            return (flow, bypassed);
        }
        let Some(turbo) = &hardware.turbo else {
            let flow = orifice_flow(filter, AMBIENT_PRESSURE, pressure, AMBIENT_TEMPERATURE);
            return (flow, 0.0);
        };
        // Every turbo (each with its diverter valve) alike, sharing the ducts.
        let turbos = hardware.turbo_count();
        let (inlet, outlet) = self.compressor_pressures(hardware, turbo, pressure);
        let speed = self.turbo_speed;
        let compressor = &turbo.compressor;
        let flow = turbo::compressor(compressor, speed, inlet, outlet, AMBIENT_TEMPERATURE).flow;
        let vented = turbo.diverter.map_or(0.0, |valve| {
            let open = (pressure - self.intake_pressure - valve.opens) / valve.span;
            let area = valve.area * open.clamp(0.0, 1.0);
            orifice_flow(area, pressure, inlet, self.charge_temperature)
        });
        (flow * turbos, vented * turbos)
    }

    /// Pressure after the air filter, with the air through it as it was (Pa).
    fn filtered(&self, hardware: &EngineHardware) -> f32 {
        let area = hardware.intake.filter_area;
        AMBIENT_PRESSURE - self.compressor_flow.powi(2) / (2.0 * ambient_density() * area * area)
    }

    /// What driving the supercharger takes at the crank at `rpm` (N·m; none without one).
    fn blower_torque(&self, hardware: &EngineHardware, rpm: f32) -> f32 {
        hardware.supercharger.map_or(0.0, |charger| {
            let inlet = self.filtered(hardware);
            let (outlet, temperature) = (self.boost_pressure, AMBIENT_TEMPERATURE);
            supercharger::point(&charger, rpm.abs(), inlet, outlet, temperature).torque
        })
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
    /// and the charge temperature following the intercooler (with some heat soak). With
    /// several turbos alike, each takes its share of the exhaust and gives its share of the
    /// air at the same pressures, so one shaft stands for them all.
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
        let flow = compressor.flow * hardware.turbo_count();
        let rate = (flow / INTERCOOLER_FLOW).min(1.0) * dt / INTERCOOLER_LAG;
        self.charge_temperature += (cooled - self.charge_temperature) * rate.min(1.0);
    }

    /// Torque on the crank at `rpm`. A stopped engine only drags, against the way it's
    /// turned (fading out over the last few rpm, so it holds still rather than chattering).
    pub fn torque(&self, build: &EngineBuild, rpm: f32) -> f32 {
        let cycle = self.cycle(build, rpm);
        let blower = self.blower_torque(&build.hardware, rpm) * rpm.signum();
        if self.firing {
            (cycle.indicated - cycle.losses) / (4.0 * PI) - blower
        } else {
            -cycle.losses / (4.0 * PI) * (rpm / 30.0).clamp(-1.0, 1.0) - blower
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
            a.fuel * hardware.cylinders as f32 * self.fuel_supply
        } else {
            0.0
        };
        let cylinders = hardware.cylinders as usize;
        let firing = self.damage.firing(cylinders);
        // Compressed without burning, the charge peaks at top dead centre.
        let motored = p * hardware.compression_ratio.powf(1.3);
        let (lambda, indicated, exhaust_temperature, knock, peak) = if fuel > 0.0 && firing > 0.0 {
            let lambda = air / (fuel * STOICHIOMETRIC);
            let profile = a.cam_profile;
            let offset = a.spark - maps.mbt(profile, rpm, load);
            let indicated = air / STOICHIOMETRIC
                * FUEL_ENERGY
                * maps.efficiency(profile, rpm, load)
                * maps::spark_efficiency(offset)
                * maps::lambda_efficiency(lambda)
                * self.damage.efficiency(cylinders)
                * firing;
            // The dead cylinders pump their charge through unburnt.
            let burnt = maps.exhaust_temperature(profile, rpm, load)
                + maps::exhaust_temperature_shift(offset, lambda);
            let temperature = firing * burnt + (1.0 - firing) * self.charge_temperature;
            let (charge, coolant) = (self.charge_temperature, self.thermal.coolant);
            let shift = maps::knock_limit_shift(charge, coolant, build.octane);
            let knock = (a.spark - maps.knock_limit(profile, rpm, load) - shift).max(0.0);
            let rise = PEAK_MOST * (PEAK_PER_DEGREE * offset / PEAK_MOST).tanh();
            let peak = maps.peak_pressure(profile, rpm, load) * (1.0 + rise);
            (lambda, indicated, temperature, knock, peak.max(motored))
        } else {
            (f32::INFINITY, 0.0, AMBIENT_TEMPERATURE, 0.0, motored)
        };
        let pumping = (self.exhaust_pressure - p) * displacement;
        let springs = hardware.spring_drag(self.cam(hardware).lift) - STOCK_SPRING_FORCE;
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
/// out: through the turbines and their wastegates (open by `wastegate`, 0..1), if there
/// are turbos, each taking its share, and then the restrictions.
fn exhaust_pressure(hardware: &EngineHardware, flow: f32, temperature: f32, wastegate: f32) -> f32 {
    let after = restricted(hardware, flow, temperature);
    let Some(turbo) = &hardware.turbo else {
        return after;
    };
    let gate = turbo.wastegate.area * wastegate;
    let turbos = hardware.turbo_count();
    let passes = |p| {
        turbos
            * (orifice_flow(turbo.turbine.area, p, after, temperature)
                + orifice_flow(gate, p, after, temperature))
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

crate::snapshot::snapshot!(Engine {
    boost_pressure,
    intake_pressure,
    exhaust_pressure,
    charge_temperature,
    turbo_speed,
    wastegate,
    compressor_flow,
    ecu,
    actuators,
    firing,
    thermal,
    airspeed,
    damage,
    peak_pressure,
    fuel_supply,
    fuel_flow,
    exhaust_lambda,
});

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
            EngineBuild::new(EngineHardware::turbo(), ecu::EcuModel::STOCK),
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
        let na = EngineBuild::new(
            super::parts::Parts::stock().hardware(),
            ecu::EcuModel::STOCK,
        );
        let build = EngineBuild::new(EngineHardware::turbo(), ecu::EcuModel::STOCK);
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
        let build = EngineBuild::new(EngineHardware::turbo(), ecu::EcuModel::STOCK);
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

    /// The ECU's fuel map `share` of what it should be, everywhere.
    fn fuel_map_off(build: &mut EngineBuild, share: f32) {
        let ve = &build.ecu.ve;
        let values = ve.values().iter().map(|v| v * share).collect();
        build.ecu.ve = table::Table::from_parts(ve.axes().to_vec(), values).unwrap();
    }

    /// λ and the fuel trim, settled at `rpm` and `throttle`, and λ's swing over the last
    /// second.
    fn settled_mixture(build: &EngineBuild, rpm: f32, throttle: f32) -> (f32, f32, f32) {
        let mut engine = Engine::default();
        let (mut low, mut high) = (f32::INFINITY, 0.0f32);
        for i in 0..(6 * 120) {
            engine.step(build, throttle, rpm, true, 1.0 / 120.0);
            if i >= 5 * 120 {
                low = low.min(engine.exhaust_lambda);
                high = high.max(engine.exhaust_lambda);
            }
        }
        (engine.exhaust_lambda, engine.actuators.trim, high - low)
    }

    /// A narrowband sensor holds λ = 1 at part throttle against a fuel map 10 % lean,
    /// hunting round it; at full throttle the loop is open and the map's error stays. A
    /// wideband one, with an ECU that reads it, holds any target at part throttle, but the
    /// loop is open at full throttle all the same.
    #[test]
    fn the_lambda_sensor_closes_the_loop() {
        let mut build = EngineBuild::default();
        fuel_map_off(&mut build, 0.9);
        let (lambda, trim, swing) = settled_mixture(&build, 3000.0, 0.15);
        assert!((lambda - 1.0).abs() < 0.03, "λ {lambda}");
        assert!((0.06..0.16).contains(&trim), "trim {trim}");
        assert!((0.005..0.08).contains(&swing), "swing {swing}");
        let target = build.ecu.lambda.get(&[5000.0, 100e3]);
        let (open, trim, _) = settled_mixture(&build, 5000.0, 1.0);
        assert!(open > target * 1.07, "λ {open} for {target}");
        assert!(trim.abs() < 0.01, "trim {trim}");
        // Aiming richer than 1 at part throttle: a narrowband sensor can't hold that, a
        // wideband one can; at full throttle neither closes the loop.
        let rich = &build.ecu.lambda;
        let values = vec![0.9; rich.values().len()];
        build.ecu.lambda = table::Table::from_parts(rich.axes().to_vec(), values).unwrap();
        let (open, trim, _) = settled_mixture(&build, 3000.0, 0.15);
        assert!(
            open > 0.9 * 1.07 && trim.abs() < 0.01,
            "λ {open}, trim {trim}"
        );
        build.hardware.wideband = true;
        build.ecu.model = ecu::EcuModel::PROGRAMMABLE;
        let (closed, trim, _) = settled_mixture(&build, 3000.0, 0.15);
        assert!((closed / 0.9 - 1.0).abs() < 0.02, "λ {closed}");
        assert!((0.06..0.16).contains(&trim), "trim {trim}");
        let (open, trim, _) = settled_mixture(&build, 5000.0, 1.0);
        assert!(
            open > 0.9 * 1.07 && trim.abs() < 0.01,
            "λ {open}, trim {trim}"
        );
        // A stock ECU reads a wideband sensor as a narrowband one.
        build.ecu.model = ecu::EcuModel::STOCK;
        assert!(settled_mixture(&build, 3000.0, 0.15).0 > 0.9 * 1.07);
        // No further than its limit: 40 % lean stays lean.
        let mut lean = EngineBuild::default();
        fuel_map_off(&mut lean, 0.6);
        let (lambda, trim, _) = settled_mixture(&lean, 3000.0, 0.15);
        assert_eq!(trim, lean.ecu.trim_limit);
        assert!(lambda > 1.15, "λ {lambda}");
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

    /// Peak torque (N·m), peak power (kW) and the speed it's made at (rpm) flat out, with
    /// a turbo given time to spool.
    fn peaks(build: &EngineBuild) -> (f32, f32, f32) {
        let (mut torque, mut power) = (0.0f32, (0.0f32, 0.0));
        let mut rpm = 1500.0;
        while rpm <= build.ecu.rev_limit - 100.0 {
            let engine = run_flat_out(build, rpm, 4.0);
            let at = engine.torque(build, rpm);
            let kilowatts = at * rpm * 2.0 * PI / 60.0 / 1000.0;
            torque = torque.max(at);
            if kilowatts > power.0 {
                power = (kilowatts, rpm);
            }
            rpm += 500.0;
        }
        (torque, power.0, power.1)
    }

    /// The R-series V10, two valves on pushrods: its torque early and flat, its power
    /// peaking well short of a four-valve engine's speeds, like a big American V10's.
    #[test]
    fn the_pushrod_v10_pulls_low_and_broad() {
        let catalogue = crate::catalogue::builtin();
        let (f, v) = catalogue.find_engine("R80").unwrap();
        let parts = crate::physics::engine::parts::Parts::of_version(&catalogue, f, v);
        let hardware = parts.hardware();
        assert!(EngineMaps::stock(&hardware).is_some(), "baked maps");
        let build = EngineBuild::new(hardware, parts.ecu_model());
        let hardware = &build.hardware;
        assert_eq!(hardware.redline(), 6000.0);
        let (mut torque, mut power) = ((0.0f32, 0.0), (0.0f32, 0.0));
        let mut curve = Vec::new();
        let mut rpm = 1500.0;
        while rpm <= build.ecu.rev_limit - 100.0 {
            let at = run_flat_out(&build, rpm, 3.0).torque(&build, rpm);
            let kilowatts = at * rpm * 2.0 * PI / 60.0 / 1000.0;
            curve.push((rpm, at.round(), kilowatts.round()));
            if at > torque.0 {
                torque = (at, rpm);
            }
            if kilowatts > power.0 {
                power = (kilowatts, rpm);
            }
            rpm += 200.0;
        }
        println!("{curve:?}");
        assert!((610.0..670.0).contains(&torque.0), "{torque:?}");
        assert!((3400.0..=4400.0).contains(&torque.1), "{torque:?}");
        assert!((285.0..315.0).contains(&power.0), "{power:?}");
        assert!((4400.0..=5600.0).contains(&power.1), "{power:?}");
        // About 630 N·m at 3600 rpm and 300 kW at 4600.
        let at = |rpm: f32| run_flat_out(&build, rpm, 3.0).torque(&build, rpm);
        assert!((at(3600.0) - 630.0).abs() < 25.0, "{}", at(3600.0));
        let kilowatts = at(4600.0) * 4600.0 * 2.0 * PI / 60.0 / 1000.0;
        assert!((kilowatts - 300.0).abs() < 12.0, "{kilowatts}");
        // Over 80 % of the peak torque from 2000 to 5200 rpm.
        assert!(
            (curve.iter())
                .filter(|(rpm, ..)| (2000.0..=5200.0).contains(rpm))
                .all(|&(_, at, _)| at > 0.8 * torque.0),
            "{curve:?}"
        );
        // The pushrods and big valves are heavy: stiff springs for a modest redline, and
        // rods for it like any other engine's.
        let valves = &hardware.valves;
        let reference = EngineHardware::default();
        assert!(valves.moving_mass() > 2.0 * reference.valves.mass);
        let float = valves.float_rpm(&hardware.cams[0]);
        assert!((6000.0..6600.0).contains(&float), "{float} rpm");
        assert!((300.0..600.0).contains(&valves.seat_force), "{valves:?}");
        assert!(hardware.rod_load(6000.0, 80e5) < 0.7);
        // Its cam's late intake closing on a wide lobe separation overlaps little: it
        // idles low.
        assert!(build.ecu.idle_rpm < 950.0, "{}", build.ecu.idle_rpm);
    }

    /// The B16R's head on the B20's block: the B16R's top end with the B20's torque, held
    /// back by the B20's long runners and its rods until short runners and forged rods go
    /// in (and an ECU that switches cams runs it).
    #[test]
    fn the_twin_cam_head_on_the_two_litre_pays() {
        use crate::physics::engine::parts::Parts;
        let catalogue = crate::catalogue::builtin();
        let measured = |name: &str, parts: &Parts| {
            let build = EngineBuild::new(parts.hardware(), ecu::EcuModel::PROGRAMMABLE);
            let (torque, power, at) = peaks(&build);
            let limit = build.ecu.rev_limit;
            println!("{name:26} {torque:4.0} N·m {power:4.0} kW at {at} rpm, limit {limit}");
            (torque, power, limit)
        };
        let factory = |code: &str| Parts::factory(code).unwrap();
        let b16 = measured("B16", &factory("B16"));
        let b20 = measured("B20", &factory("B20"));
        let b16r = measured("B16R", &factory("B16R"));
        let mut swap = factory("B20");
        let family = &catalogue.families[swap.family];
        swap.head = family
            .heads
            .iter()
            .position(|h| h.id == "b16r.head")
            .unwrap();
        let headed = measured("B20 with the B16R head", &swap);
        swap.chosen[8] = 2;
        let short = measured("… and short runners", &swap);
        swap.chosen[3] = 1;
        let forged = measured("… and forged rods", &swap);
        // The stock three: the 2.0 pulls harder than the 1.6, the 1.6R makes far more power.
        assert!(b20.0 > 1.15 * b16.0 && b20.1 > b16.1, "{b16:?} {b20:?}");
        assert!(b16r.1 > 1.5 * b20.1, "{b20:?} {b16r:?}");
        // The head alone: more torque than either, far more power than the B20.
        assert!(headed.0 > b20.0 && headed.0 > 1.15 * b16r.0, "{headed:?}");
        assert!(headed.1 > 1.4 * b20.1, "{headed:?}");
        // The B20's rods hold it under the B16R's revs; forged ones don't.
        assert!(
            short.2 < b16r.2 - 500.0 && forged.2 >= b16r.2,
            "{short:?} {forged:?}"
        );
        // All of it: more power than the B16R, and much more torque.
        assert!(
            forged.1 > 1.04 * b16r.1 && forged.0 > 1.15 * b16r.0,
            "{forged:?} {b16r:?}"
        );
    }

    /// Peak torque (N·m), peak power (kW), the speed of the peak power and the rev limit of
    /// a factory engine with its own (locked) ECU.
    fn factory_peaks(code: &str) -> (f32, f32, f32, f32) {
        let parts = crate::physics::engine::parts::Parts::factory(code).unwrap();
        let build = EngineBuild::of_parts(&parts);
        let (torque, power, at) = peaks(&build);
        let limit = build.ecu.rev_limit;
        println!("{code:5} {torque:4.0} N·m {power:4.0} kW at {at} rpm, limit {limit}");
        (torque, power, at, limit)
    }

    /// The K series as the table in `docs/k-lineup.md` has it: the K20 about 118 kW at
    /// 6500 (limit 6900), the K20R 158 kW at 8000 (8600), the K24 120 kW at 5800 (6500)
    /// with far more torque. Only the K20R's ECU switches its cams and phases the intake.
    #[test]
    fn the_k_series_is_what_the_table_says() {
        let k20 = factory_peaks("K20");
        let k20r = factory_peaks("K20R");
        let k24 = factory_peaks("K24");
        let near = |(_, power, at, limit): (f32, f32, f32, f32), kw: f32, rpm: f32, top: f32| {
            (power - kw).abs() < 0.06 * kw
                && (at - rpm).abs() <= 500.0
                && (limit - top).abs() <= 100.0
        };
        assert!(near(k20, 118.0, 6500.0, 6900.0), "{k20:?}");
        assert!(near(k20r, 158.0, 8000.0, 8600.0), "{k20r:?}");
        assert!(near(k24, 120.0, 5800.0, 6500.0), "{k24:?}");
        assert!(k24.0 > 1.12 * k20.0, "{k20:?} {k24:?}");
        let ecu = |code: &str| {
            let parts = crate::physics::engine::parts::Parts::factory(code).unwrap();
            parts.ecu_model()
        };
        assert!(ecu("K20R").switching && ecu("K20R").phasers && ecu("K20R").locked);
        assert!(!ecu("K20").cam_control() && !ecu("K24").cam_control());
        // The K20R's ECU advances the intake cam under load, and puts the wild cam in
        // high up.
        let parts = crate::physics::engine::parts::Parts::factory("K20R").unwrap();
        let build = EngineBuild::of_parts(&parts);
        assert_eq!(build.hardware.intake_phasing, 50.0);
        let phased = (3..16)
            .map(|i| i as f32 * 500.0)
            .find(|&rpm| build.ecu.cams.intake_phase.get(&[rpm, 100e3]) > 10.0)
            .expect("phased somewhere at full load");
        let engine = run_flat_out(&build, phased, 2.0);
        assert!(
            engine.actuators.intake_phase > 10.0,
            "at {phased} rpm: {:?}",
            engine.actuators
        );
        assert_eq!(engine.actuators.cam_profile, 0);
        let engine = run_flat_out(&build, 7500.0, 2.0);
        assert_eq!(engine.actuators.cam_profile, 1);
    }

    /// The K20R's head on the K24's block: more torque than either at once, and more
    /// power than the K24; its revs held by the K24's rods until forged ones go in; with
    /// those, short runners and long-tube headers more power than the K20R, and more again
    /// with pistons that give back the K20R's compression. The K20R's stock ECU runs
    /// 2-litre maps on it (lean flat out), the K24's doesn't switch the cams.
    #[test]
    fn the_k20r_head_on_the_k24_block_pays() {
        use crate::physics::engine::parts::Parts;
        let catalogue = crate::catalogue::builtin();
        let measured = |name: &str, parts: &Parts| {
            let build = EngineBuild::new(parts.hardware(), ecu::EcuModel::PROGRAMMABLE);
            let (torque, power, at) = peaks(&build);
            let limit = build.ecu.rev_limit;
            println!("{name:30} {torque:4.0} N·m {power:4.0} kW at {at} rpm, limit {limit}");
            (torque, power, limit)
        };
        let k20r = measured("K20R", &Parts::factory("K20R").unwrap());
        let k24 = measured("K24", &Parts::factory("K24").unwrap());
        let mut swap = Parts::factory("K24").unwrap();
        let family = &catalogue.families[swap.family];
        swap.head = (family.heads.iter())
            .position(|h| h.id == "k20r.head")
            .unwrap();
        let headed = measured("K24 with the K20R head", &swap);
        // The phaser comes with the head.
        assert_eq!(swap.hardware().intake_phasing, 50.0);
        let mut built = swap.clone();
        built.chosen[3] = 1;
        built.chosen[8] = 2;
        built.manifold = 2;
        let forged = measured("… forged rods, short runners,", &built);
        println!("{:30} (and long-tube headers)", "");
        built.chosen[crate::catalogue::PISTONS] = 3;
        let pistons = measured("… and race pistons", &built);
        // The head alone: more torque than either, more power than the K24.
        assert!(
            headed.0 > 1.05 * k24.0 && headed.0 > 1.1 * k20r.0,
            "{headed:?}"
        );
        assert!(headed.1 > 1.15 * k24.1, "{headed:?}");
        // The K24's rods hold it well under the K20R's revs; forged ones don't.
        assert!(
            headed.2 < k20r.2 - 500.0 && forged.2 >= k20r.2 - 100.0,
            "{headed:?} {forged:?}"
        );
        // Built: more power than the K20R, and more torque.
        assert!(
            forged.1 > 1.02 * k20r.1 && forged.0 > 1.05 * k20r.0,
            "{forged:?} {k20r:?}"
        );
        assert!(pistons.1 > 1.06 * k20r.1, "{pistons:?} {k20r:?}");
        // Stock ECUs: the K20R's fuels a 2.0 (lean on the 2.4), the K24's stays on the
        // mild cam.
        let ecu = |code: &str| {
            let mut parts = swap.clone();
            parts.ecu = (catalogue.ecus.iter())
                .position(|e| e.factory.as_deref() == Some(code))
                .unwrap();
            EngineBuild::of_parts(&parts)
        };
        let build = ecu("K20R");
        let target = build.ecu.maps_on(1).1.get(&[6000.0, 100e3]);
        let engine = run_flat_out(&build, 6000.0, 2.0);
        assert!(
            engine.exhaust_lambda > target + 0.05,
            "λ {} for {target}",
            engine.exhaust_lambda
        );
        let build = ecu("K24");
        assert_eq!(run_flat_out(&build, 6000.0, 2.0).actuators.cam_profile, 0);
    }

    #[test]
    fn every_factory_engine_idles_and_pulls() {
        let catalogue = crate::catalogue::builtin();
        for (f, family) in catalogue.families.iter().enumerate() {
            for (v, version) in family.versions.iter().enumerate() {
                let parts = crate::physics::engine::parts::Parts::of_version(&catalogue, f, v);
                let hardware = parts.hardware();
                assert!(EngineMaps::stock(&hardware).is_some(), "baked maps");
                let build = EngineBuild::new(hardware, parts.ecu_model());
                let litres = build.hardware.displacement() * 1e3;
                let (torque, power, at) = peaks(&build);
                println!(
                    "{:5} {:24} {torque:4.0} N·m {power:4.0} kW at {at} rpm, redline {}, \
                     {:.0} N·m/l {:.0} kW/l",
                    version.code,
                    version.name,
                    build.hardware.redline(),
                    torque / litres,
                    power / litres,
                );
                // Naturally aspirated engines make 75 (a big, lazy pushrod V) to 125 N·m a
                // litre, turbo engines on their stock boost a good deal more; power peaks
                // in the upper half.
                let per_litre = torque / litres;
                let expected = if build.hardware.turbo.is_some() {
                    125.0..200.0
                } else {
                    75.0..125.0
                };
                assert!(
                    expected.contains(&per_litre),
                    "{}: {per_litre} N·m/l",
                    version.code
                );
                assert!(
                    at >= 0.6 * build.hardware.redline(),
                    "{}: {at} rpm",
                    version.code
                );
                // It idles with the throttle closed.
                let mut engine = Engine::default();
                let mut rpm = 1200.0;
                for _ in 0..(6.0 * 120.0) as usize {
                    engine.step(&build, 0.0, rpm, true, 1.0 / 120.0);
                    let torque = engine.torque(&build, rpm);
                    rpm += torque / build.hardware.inertia / 120.0 * 60.0 / (2.0 * PI);
                }
                let idle = build.ecu.idle_rpm;
                assert!(
                    (rpm - idle).abs() < 60.0,
                    "{}: idles at {rpm}",
                    version.code
                );
            }
        }
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
            let mut build = EngineBuild::new(parts.hardware(), ecu::EcuModel::STOCK);
            build.octane = 110.0;
            build.ecu = Ecu::calibrated(&build.hardware, &build.maps, 110.0, parts.ecu_model());
            build
        };
        let mut parts = twin_cam_turbo();
        parts.boost = 1.5;
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
        let mut parts = twin_cam_turbo();
        parts.chosen[crate::catalogue::TURBO] = 1;
        parts.boost = 1.5;
        let build = EngineBuild::new(parts.hardware(), ecu::EcuModel::STOCK);
        let engine = run_flat_out(&build, 5800.0, 10.0);
        assert!(engine.damage.turbo_failed(), "{:?}", engine.damage);
        assert_eq!(engine.damage.failures(4), ["turbo failed"]);
        // It no longer boosts, but the engine runs on.
        assert!(engine.boost_pressure < AMBIENT_PRESSURE);
        assert!(engine.torque(&build, 5800.0) > 50.0);
    }

    /// The B16R turbocharged as `Parts::turbo` does the B16 (whose 8-valve head doesn't
    /// breathe enough to break things), on its first cam profile with a stock ECU.
    fn twin_cam_turbo() -> parts::Parts {
        let mut parts = parts::Parts::factory("B16R").unwrap();
        parts.chosen[crate::catalogue::PISTONS] = 1;
        parts.chosen[crate::catalogue::TURBO] = 2;
        parts.manifold = parts.turbo_manifold().unwrap();
        parts
    }

    /// The R80 on the family's manifold `id`, with turbo `turbo` (by its place in the
    /// slot, 0 none) on every mount and low compression pistons with a turbo.
    fn r80_on(id: &str, turbo: usize) -> EngineBuild {
        r80_boosted(id, turbo, 0.6)
    }

    /// Likewise with the wastegate's spring at `boost` (bar).
    fn r80_boosted(id: &str, turbo: usize, boost: f32) -> EngineBuild {
        let mut parts = parts::Parts::factory("R80").unwrap();
        parts.manifold = (parts.family().manifolds.iter())
            .position(|m| m.id == id)
            .unwrap();
        if turbo > 0 {
            parts.chosen[crate::catalogue::PISTONS] = 1;
            parts.chosen[crate::catalogue::TURBO] = turbo;
            parts.boost = boost;
        }
        EngineBuild::new(parts.hardware(), parts.ecu_model())
    }

    /// Boost (bar) and torque (N·m) flat out after `seconds` at each of `rpms`.
    fn boost_curve(build: &EngineBuild, rpms: &[f32], seconds: f32) -> Vec<(f32, f32, f32)> {
        (rpms.iter())
            .map(|&rpm| {
                let engine = run_flat_out(build, rpm, seconds);
                let boost = (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5;
                let torque = engine.torque(build, rpm);
                (rpm, (boost * 100.0).round() / 100.0, torque.round())
            })
            .collect()
    }

    /// Seconds from flooring it at `rpm` (after cruising at a light throttle) until the
    /// boost reaches `bar` (10 if it never does).
    fn spool_time(build: &EngineBuild, rpm: f32, bar: f32) -> f32 {
        let mut engine = Engine::default();
        for _ in 0..600 {
            engine.step(build, 0.15, rpm, true, 1.0 / 120.0);
        }
        for step in 0..1200 {
            engine.step(build, 1.0, rpm, true, 1.0 / 120.0);
            if engine.boost_pressure - AMBIENT_PRESSURE >= bar * 1e5 {
                return step as f32 / 120.0;
            }
        }
        10.0
    }

    /// Mean boost (bar) over the last second of `seconds` flat out at `rpm`.
    fn mean_boost(build: &EngineBuild, rpm: f32, seconds: f32) -> f32 {
        let mut engine = run_flat_out(build, rpm, seconds - 1.0);
        let mut sum = 0.0;
        for _ in 0..120 {
            engine.step(build, 1.0, rpm, true, 1.0 / 120.0);
            sum += engine.boost_pressure - AMBIENT_PRESSURE;
        }
        sum / 120.0 / 1e5
    }

    /// The R80 naturally aspirated: its stock cast manifolds end in tuned collectors, the
    /// turbo manifolds without turbos in the turbines' flanges, which send less of the
    /// wave back (less scavenging); long tubes tune lower in the band than the short cast
    /// primaries.
    #[test]
    fn the_r80s_manifolds_tune_it_without_turbos() {
        let rpms = [3500.0, 4000.0, 4500.0, 5000.0, 5500.0];
        let torques = |id: &str| {
            let build = r80_on(id, 0);
            assert!(build.hardware.turbo.is_none() && build.hardware.turbos == 0);
            rpms.map(|rpm| run_flat_out(&build, rpm, 3.0).torque(&build, rpm))
        };
        let (cast, empty, long) = (
            torques("r.manifolds.stock"),
            torques("r.manifolds.turbo"),
            torques("r.headers.long-tube"),
        );
        println!("cast {cast:?}\nempty turbo manifolds {empty:?}\nlong tubes {long:?}");
        let mean = |t: &[f32; 5]| t.iter().sum::<f32>() / 5.0;
        assert!(mean(&cast) > mean(&empty), "{cast:?} vs {empty:?}");
        // Long tubes ahead of the turbo manifolds low in the band, behind high up.
        assert!(
            long[0] > empty[0] && long[1] > empty[1],
            "{long:?} vs {empty:?}"
        );
        assert!(
            long[3] < empty[3] && long[4] < empty[4],
            "{long:?} vs {empty:?}"
        );
    }

    /// The same turbo kit on the R80's pair of turbo manifolds (a turbo per bank, each
    /// sized for 4 litres) and on the 10-into-1 (one sized for 8): the pair's smaller
    /// shafts spool quicker; the big one, a little more efficient, holds more boost high
    /// up where the small ones run out of breath.
    #[test]
    fn twin_turbos_spool_quicker_and_a_big_one_holds_boost_high() {
        let (pair, single) = (
            r80_on("r.manifolds.turbo", 2),
            r80_on("r.manifold.turbo-crossover", 2),
        );
        assert_eq!((pair.hardware.turbos, single.hardware.turbos), (2, 1));
        let (small, big) = (pair.hardware.turbo.unwrap(), single.hardware.turbo.unwrap());
        let ratio = big.compressor.diameter / small.compressor.diameter;
        assert!((ratio - 2f32.sqrt()).abs() < 1e-3, "{ratio}");
        for rpm in [3500.0, 4000.0] {
            let (quick, slow) = (spool_time(&pair, rpm, 0.4), spool_time(&single, rpm, 0.4));
            assert!(quick < 0.85 * slow, "{quick} s vs {slow} s at {rpm} rpm");
        }
        // On their springs both hold the wastegate's boost.
        for build in [&pair, &single] {
            let boost = mean_boost(build, 5000.0, 4.0);
            assert!((0.62..0.76).contains(&boost), "{boost} bar");
        }
        // The small turbo held to 1.5 bar at the top: the pair's spin nearer their limit,
        // and held to 1.8 bar they burst where the big one still holds.
        let share = |build: &EngineBuild, rpm| {
            let engine = run_flat_out(build, rpm, 5.0);
            engine.turbo_speed / build.hardware.turbo.unwrap().max_speed()
        };
        let (pair, single) = (
            r80_boosted("r.manifolds.turbo", 1, 1.5),
            r80_boosted("r.manifold.turbo-crossover", 1, 1.5),
        );
        let (twins, one) = (share(&pair, 5800.0), share(&single, 5800.0));
        println!("shaft speed at 1.5 bar, 5800 rpm: pair {twins}, crossover {one} of the limit");
        assert!(twins > one + 0.02, "{twins} vs {one}");
        let (pair, single) = (
            r80_boosted("r.manifolds.turbo", 1, 1.8),
            r80_boosted("r.manifold.turbo-crossover", 1, 1.8),
        );
        let (burst, held) = (
            run_flat_out(&pair, 5000.0, 5.0),
            run_flat_out(&single, 5000.0, 5.0),
        );
        assert!(burst.damage.turbo_failed(), "{:?}", burst.damage);
        assert!(!held.damage.turbo_failed(), "{:?}", held.damage);
        let boost = (held.boost_pressure - AMBIENT_PRESSURE) / 1e5;
        assert!(boost > 1.7, "{boost} bar");
    }

    /// Twin turbos on the V10 flat out for a minute: the factory tune keeps the turbine
    /// inlets clear of their limit and the cylinders clear of knock, and the oil (cooled
    /// by the coolant as a big engine's is) holds.
    #[test]
    fn twin_turbos_last_on_the_factory_tune() {
        let build = r80_on("r.manifolds.turbo", 2);
        let mut engine = Engine {
            airspeed: 40.0,
            ..Engine::default()
        };
        for _ in 0..60 * 120 {
            engine.step(&build, 1.0, 5500.0, true, 1.0 / 120.0);
        }
        let cycle = engine.cycle(&build, 5500.0);
        assert_eq!(cycle.knock, 0.0);
        assert!(engine.damage.turbo < 1e-3, "{:?}", engine.damage);
        let wear = engine.damage.worst(10);
        assert!(
            wear < 1e-3,
            "wear {wear}: {:?} {:?}",
            engine.damage,
            engine.thermal
        );
    }

    /// The B series' turbo manifold has one branch: its turbo is sized for the whole
    /// engine. On the 8-valve B16 (since 2026-11-27; the 16-valve B16 made 202 N·m and 0.40
    /// bar at 3000 rpm, 254 at 4000, 241 and 0.69 bar at 5000) its torque falls off sooner.
    #[test]
    fn the_b_series_turbo_is_as_before() {
        let build = EngineBuild::new(EngineHardware::turbo(), ecu::EcuModel::STOCK);
        assert_eq!(build.hardware.turbos, 1);
        for (rpm, torque, boost) in [
            (3000.0, 212.0, 0.47),
            (4000.0, 229.0, 0.65),
            (5000.0, 207.0, 0.68),
        ] {
            let engine = run_flat_out(&build, rpm, 4.0);
            let at = engine.torque(&build, rpm);
            let bar = (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5;
            println!("{rpm}: {at} N·m {bar} bar");
            assert!((at / torque - 1.0).abs() < 0.06, "{at} N·m at {rpm} rpm");
            assert!((bar - boost).abs() < 0.04, "{bar} bar at {rpm} rpm");
        }
    }

    /// Prints the R80's turbo layouts and the B series' turbo engine.
    #[test]
    #[ignore]
    fn turbo_layouts() {
        let rpms: Vec<f32> = [
            1500.0, 2000.0, 2500.0, 3000.0, 3500.0, 4000.0, 4500.0, 5000.0, 5500.0, 5800.0,
        ]
        .to_vec();
        let b = EngineBuild::new(EngineHardware::turbo(), ecu::EcuModel::STOCK);
        println!("B16T medium: {:?}", boost_curve(&b, &rpms, 4.0));
        for turbo in [1, 2, 3] {
            for id in ["r.manifolds.turbo", "r.manifold.turbo-crossover"] {
                let build = r80_on(id, turbo);
                let t = build.hardware.turbo.unwrap();
                println!(
                    "{id} turbo {turbo} x{}: compressor {:.0} mm, rev limit {}: {:?}",
                    build.hardware.turbos,
                    t.compressor.diameter * 1e3,
                    build.ecu.rev_limit,
                    boost_curve(&build, &rpms, 4.0)
                );
            }
        }
        for id in ["r.manifolds.turbo", "r.manifold.turbo-crossover"] {
            let build = r80_boosted(id, 1, 1.2);
            println!(
                "{id} small at 1.2 bar: {:?}",
                boost_curve(&build, &rpms, 4.0)
            );
            for bar in [1.2, 1.5, 1.8] {
                for turbo in [1, 2] {
                    let build = r80_boosted(id, turbo, bar);
                    let means: Vec<_> = [4000.0, 5000.0, 5800.0]
                        .map(|rpm| (rpm, (mean_boost(&build, rpm, 5.0) * 100.0).round() / 100.0))
                        .to_vec();
                    println!("{id} turbo {turbo} at {bar} bar: mean boost {means:?}");
                }
            }
        }
        for turbo in [1, 2, 3] {
            for id in ["r.manifolds.turbo", "r.manifold.turbo-crossover"] {
                let build = r80_on(id, turbo);
                let spool: Vec<_> = [2500.0, 3000.0, 3500.0, 4000.0]
                    .map(|rpm| (rpm, spool_time(&build, rpm, 0.4)))
                    .to_vec();
                println!("{id} turbo {turbo}: time to 0.4 bar {spool:?}");
            }
        }
        for id in [
            "r.manifolds.stock",
            "r.headers.long-tube",
            "r.manifolds.turbo",
        ] {
            println!("{id}: {:?}", boost_curve(&r80_on(id, 0), &rpms, 3.0));
        }
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
