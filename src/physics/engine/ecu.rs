//! Engine control units: they read the sensors and drive the actuators, using their own
//! idea of the engine (their maps), which matches the real one only as well as the tune.

use super::damage::TURBINE_LIMIT;
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
/// A factory tune keeps the turbine inlet this far under what the turbine takes (K),
/// going richer where it would be hotter: a hot charge and heat soak on the road add to
/// what the dyno saw.
const TURBINE_MARGIN: f32 = 40.0;
/// Richest a factory tune goes to cool the exhaust.
const RICHEST: f32 = 0.68;
const OVERRUN_RESUME: f32 = 400.0;
/// The cams switch back to the first profile this far below the switch-over (rpm, Pa).
const CAM_HYSTERESIS_RPM: f32 = 200.0;
const CAM_HYSTERESIS_MAP: f32 = 5e3;
/// Manifold pressures between which the factory phase maps go from leaving the cams at
/// rest (light load and idle, for a steady idle) to the most air (Pa).
const PHASE_FROM: f32 = 45e3;
const PHASE_TO: f32 = 75e3;
/// Manifold pressure above which the factory tune switches cams (Pa).
const SWITCH_MAP: f32 = 60e3;
/// How long the lambda sensor takes to follow the exhaust (s).
const SENSOR_LAG: f32 = 0.08;
/// A narrowband sensor's closed loop: the trim's jump each time the sensor switches
/// between rich and lean, and its ramp in between (per second); target λ within
/// `STOICHIOMETRIC_BAND` of 1 is where it runs.
const NARROWBAND_JUMP: f32 = 0.02;
const NARROWBAND_RAMP: f32 = 0.08;
const STOICHIOMETRIC_BAND: f32 = 0.005;
/// A wideband sensor's closed loop: trim per second per share the mixture is off.
const WIDEBAND_GAIN: f32 = 1.5;
/// Out of closed loop the trim lets go within about this long (s).
const TRIM_RELEASE: f32 = 0.5;
/// The most a factory tune lets the closed loop trim the fuel by (share).
const TRIM_LIMIT: f32 = 0.25;
/// A narrowband sensor's output (V): about 0.1 lean, 0.8 rich, switching at λ = 1.
fn narrowband_volts(lambda: f32) -> f32 {
    0.45 + 0.4 * ((1.0 - lambda) / 0.01).tanh()
}

/// What an ECU's hardware can do: how many cylinders it fires, whether it switches cams
/// and drives phasers, reads a wideband lambda sensor, and whether its maps are locked
/// (a stock ECU's, the factory's for one engine). The catalogue names them (`EcuDef`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcuModel {
    pub cylinders: u32,
    pub switching: bool,
    pub phasers: bool,
    pub wideband: bool,
    pub locked: bool,
}

impl EcuModel {
    /// For tests and tools: fuelling, spark, rev limiter and idle control for any engine.
    pub const STOCK: Self = Self {
        cylinders: super::damage::MAX_CYLINDERS as u32,
        switching: false,
        phasers: false,
        wideband: false,
        locked: false,
    };
    /// The same, with cam switching, phasers and a wideband sensor.
    pub const PROGRAMMABLE: Self = Self {
        switching: true,
        phasers: true,
        wideband: true,
        ..Self::STOCK
    };

    /// It drives cam switching (to the second cam profile) or the cam phasers.
    pub fn cam_control(self) -> bool {
        self.switching || self.phasers
    }
}

impl Default for EcuModel {
    fn default() -> Self {
        Self::STOCK
    }
}

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
    /// The exhaust's λ where the lambda sensor sits, and whether it's a wideband sensor
    /// (which reads it; a narrowband one only says rich or lean).
    pub lambda: f32,
    pub wideband: bool,
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
    /// What the lambda sensor reads (a narrowband's volts, a wideband's λ), and the closed
    /// loop's fuel trim (share more fuel).
    pub lambda_sensor: f32,
    pub trim: f32,
}

/// An ECU: speed-density fuelling from rpm, manifold pressure and intake air temperature,
/// a spark map, a rev limiter and idle control; and, if its model can, cam switching and
/// phasing (otherwise the cams stay on the first profile, unphased).
#[derive(Clone, Debug, PartialEq)]
pub struct Ecu {
    pub model: EcuModel,
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
    /// The most the closed loop trims the fuel by (share either way).
    pub trim_limit: f32,
    /// Cam control, used if the model has it.
    pub cams: CamControl,
}

/// The factory's idle speed for `hardware`: wild cams (on the profile it idles on) run too
/// roughly for a low idle, from the overlap of the intake's and exhaust's opening (crank
/// degrees: a long duration on a wide lobe separation idles smoothly).
fn idle_rpm(hardware: &EngineHardware) -> f32 {
    let cam = &hardware.cams[0];
    let overlap = (cam.intake_duration + cam.exhaust_duration) / 2.0 - 2.0 * cam.lobe_separation;
    850.0 + 12.5 * (overlap - 20.0).max(0.0)
}

/// The factory's rev limit for `hardware` with a `model` ECU: without cam control, the
/// valvetrain stays on the first profile.
fn rev_limit(hardware: &EngineHardware, model: EcuModel) -> f32 {
    hardware.safe_rpm(if model.switching { usize::MAX } else { 0 })
}

/// When to switch cam profiles, and where to put the phasers.
#[derive(Clone, Debug, PartialEq)]
pub struct CamControl {
    /// Switch to the second cam profile above `switch_rpm` with the manifold pressure above
    /// `switch_map` (Pa), and back a little below either.
    pub switch_rpm: f32,
    pub switch_map: f32,
    /// Intake cam advance and exhaust cam retard (crank degrees), by rpm and manifold
    /// pressure.
    pub intake_phase: Table,
    pub exhaust_phase: Table,
    /// The fuel (VE), target λ and spark maps on the second cam profile (the ECU's own on
    /// the first).
    pub ve: Table,
    pub lambda: Table,
    pub spark: Table,
}

/// The ECU's memory between steps.
#[derive(Clone, Copy, Debug)]
pub struct EcuState {
    /// Time left in the rev limiter's cycle: fuel cut, then fuel back on.
    limiter: f32,
    /// Integral term of the idle controller (valve opening).
    idle: f32,
    /// Fuel cut on the overrun.
    overrun: bool,
    /// On the second cam profile.
    high_cam: bool,
    /// The λ the lambda sensor has caught up with, the closed loop's fuel trim, and which
    /// side of λ = 1 a narrowband sensor last said.
    sensed: f32,
    trim: f32,
    rich: bool,
}

impl Default for EcuState {
    fn default() -> Self {
        Self {
            limiter: 0.0,
            idle: 0.0,
            overrun: false,
            high_cam: false,
            sensed: 1.0,
            trim: 0.0,
            rich: false,
        }
    }
}

impl Ecu {
    /// A `model` ECU with nothing in its tables, for an engine only to be looked at (or
    /// heard, by its idle speed and rev limit): it never runs one.
    pub fn blank(hardware: &EngineHardware, model: EcuModel) -> Self {
        let table = || Table::bake(vec![vec![0.0]; 2], |_| 0.0);
        Self {
            model,
            displacement: hardware.displacement(),
            cylinders: hardware.cylinders,
            ve: table(),
            lambda: table(),
            spark: table(),
            idle_rpm: idle_rpm(hardware),
            rev_limit: rev_limit(hardware, model),
            limiter_cut: 0.08,
            limiter_fuel: 0.04,
            trim_limit: TRIM_LIMIT,
            cams: CamControl {
                switch_rpm: f32::INFINITY,
                switch_map: f32::INFINITY,
                intake_phase: table(),
                exhaust_phase: table(),
                ve: table(),
                lambda: table(),
                spark: table(),
            },
        }
    }

    /// A factory tune of a `model` ECU for the hardware: its VE table matches the engine,
    /// it runs λ = 1 at part load and richer towards full load, and the spark just short
    /// of MBT or of the knock limit on `octane` fuel (with the charge as hot as the turbo's
    /// or supercharger's compressor leaves it), whichever comes first. With a turbo or a
    /// supercharger, the maps reach 300 kPa and go richer still on boost; with a turbo
    /// richer again where the
    /// turbine inlet would come near its limit (`TURBINE_MARGIN`). With cam control, it switches
    /// cams where the second profile breathes better, and phases the cams for the most air
    /// under load (at rest at light load).
    pub fn calibrated(
        hardware: &EngineHardware,
        maps: &EngineMaps,
        octane: f32,
        model: EcuModel,
    ) -> Self {
        let boosted = hardware.turbo.is_some() || hardware.supercharger.is_some();
        let top = if boosted { 30 } else { 11 };
        let axes = || {
            let rpm = (0..19).map(|i| i as f32 * 500.0).collect();
            let map = (1..=top).map(|i| i as f32 * 10e3).collect();
            vec![rpm, map]
        };
        let lambda = |map: f32| {
            1.0 - 0.12 * ((map - 60e3) / 35e3).clamp(0.0, 1.0)
                - 0.1 * ((map - 100e3) / 100e3).clamp(0.0, 1.0)
        };
        // On the dyno: the charge as warm as the intercooler leaves it, the exhaust
        // pressure whatever the turbo and exhaust make with the wastegate on its spring.
        let charge = |map| match (&hardware.turbo, &hardware.supercharger) {
            (Some(turbo), _) => turbo::expected_charge(turbo, map),
            (None, Some(charger)) => super::supercharger::expected_charge(charger, map),
            (None, None) => AMBIENT_TEMPERATURE,
        };
        let wastegate = |map: f32| {
            hardware.turbo.as_ref().map_or(0.0, |turbo| {
                turbo::wastegate_opening(&turbo.wastegate, map - AMBIENT_PRESSURE)
            })
        };
        let displacement = hardware.displacement();
        let mut cams = CamControl::calibrated(hardware, maps, model.switching, axes);
        // The phasers where the ECU puts them.
        let phases = |rpm: f32, map: f32| {
            if !model.phasers {
                return (0.0, 0.0);
            }
            let point = [rpm, map];
            (
                cams.intake_phase.get(&point),
                cams.exhaust_phase.get(&point),
            )
        };
        let load = |ve: f32, map: f32| ve * map / (R_AIR * charge(map)) / super::ambient_density();
        // The fuel, spark and λ maps for cam profile `profile`.
        let tables = |profile: usize| {
            let ve = Table::bake(axes(), |p| {
                let (rpm, map) = (p[0], p[1]);
                let (intake, exhaust_phase) = phases(rpm, map);
                let mut exhaust = AMBIENT_PRESSURE;
                let mut ve = 0.0;
                for _ in 0..3 {
                    ve = maps.ve(profile, rpm, map, exhaust / map, intake, exhaust_phase);
                    let air = load(ve, map) * super::ambient_density() * displacement;
                    let flow = air * (1.0 + 1.0 / (STOICHIOMETRIC * lambda(map))) * rpm / 120.0;
                    let temperature = maps.exhaust_temperature(profile, rpm, load(ve, map));
                    exhaust = super::exhaust_pressure(hardware, flow, temperature, wastegate(map));
                }
                ve
            });
            let spark = Table::bake(axes(), |p| {
                let (rpm, map) = (p[0], p[1]);
                let load = load(ve.get(p), map);
                let shift = maps::knock_limit_shift(charge(map), 363.0, octane);
                let knock = maps.knock_limit(profile, rpm, load) + shift;
                maps.mbt(profile, rpm, load).min(knock) - 2.0
            });
            let lambda = Table::bake(axes(), |p| {
                let (rpm, map) = (p[0], p[1]);
                let lambda = lambda(map);
                if hardware.turbo.is_none() {
                    return lambda;
                }
                let load = load(ve.get(p), map);
                let offset = spark.get(p) - maps.mbt(profile, rpm, load);
                let hot = maps.exhaust_temperature(profile, rpm, load)
                    + maps::exhaust_temperature_shift(offset, lambda);
                // Each hundredth richer takes 4 K off.
                let over = hot - (TURBINE_LIMIT - TURBINE_MARGIN);
                (lambda - over.max(0.0) / 400.0).max(RICHEST)
            });
            (ve, spark, lambda)
        };
        let (ve, spark, lambda) = tables(0);
        // The second profile's maps where the ECU can switch to one (else the first's).
        (cams.ve, cams.spark, cams.lambda) = if model.switching && hardware.cams.len() > 1 {
            tables(1)
        } else {
            (ve.clone(), spark.clone(), lambda.clone())
        };
        Self {
            model,
            cams,
            displacement: hardware.displacement(),
            cylinders: hardware.cylinders,
            lambda,
            ve,
            spark,
            idle_rpm: idle_rpm(hardware),
            rev_limit: rev_limit(hardware, model),
            limiter_cut: 0.08,
            limiter_fuel: 0.04,
            trim_limit: TRIM_LIMIT,
        }
    }

    /// The fuel (VE), target λ and spark maps on cam profile `profile`.
    pub fn maps_on(&self, profile: usize) -> (&Table, &Table, &Table) {
        if profile > 0 {
            (&self.cams.ve, &self.cams.lambda, &self.cams.spark)
        } else {
            (&self.ve, &self.lambda, &self.spark)
        }
    }

    pub fn step(&self, state: &mut EcuState, sensors: &Sensors, dt: f32) -> Actuators {
        let Sensors {
            rpm,
            map,
            throttle,
            intake_temperature,
            lambda: exhaust_lambda,
            wideband,
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

        // Cams: switched with some hysteresis, phased by the maps.
        let cams = &self.cams;
        let margin = if state.high_cam { 1.0 } else { 0.0 };
        state.high_cam = self.model.switching
            && rpm > cams.switch_rpm - margin * CAM_HYSTERESIS_RPM
            && map > cams.switch_map - margin * CAM_HYSTERESIS_MAP;
        let point = [rpm, map];
        let (intake_phase, exhaust_phase) = if self.model.phasers {
            (
                cams.intake_phase.get(&point),
                cams.exhaust_phase.get(&point),
            )
        } else {
            (0.0, 0.0)
        };
        let cam_profile = usize::from(state.high_cam);

        let point = [rpm, map];
        let (ve, lambda, spark) = self.maps_on(cam_profile);
        let air = ve.get(&point) * map * self.displacement
            / (R_AIR * intake_temperature * self.cylinders as f32);
        let target = lambda.get(&point);

        // The lambda sensor follows the exhaust. With a wideband sensor (and an ECU that
        // reads one) the loop closes to the target everywhere; with a narrowband one only
        // where the target is λ = 1, the trim jumping at each switch and ramping between
        // (so the mixture hunts round it); elsewhere the trim lets go.
        let seen = if exhaust_lambda.is_finite() {
            exhaust_lambda.clamp(0.5, 2.0)
        } else {
            2.0
        };
        state.sensed += (seen - state.sensed) * (dt / SENSOR_LAG).min(1.0);
        let wideband = wideband && self.model.wideband;
        let running = rpm > RUNNING_RPM && !cut;
        let limit = self.trim_limit.max(0.0);
        if running && wideband {
            let lean = state.sensed / target - 1.0;
            state.trim += WIDEBAND_GAIN * lean * dt;
        } else if running && (target - 1.0).abs() < STOICHIOMETRIC_BAND {
            let rich = state.sensed < 1.0;
            if rich != state.rich {
                state.trim += if rich {
                    -NARROWBAND_JUMP
                } else {
                    NARROWBAND_JUMP
                };
                state.rich = rich;
            }
            state.trim += if rich {
                -NARROWBAND_RAMP
            } else {
                NARROWBAND_RAMP
            } * dt;
        } else {
            state.trim *= (1.0 - dt / TRIM_RELEASE).max(0.0);
        }
        state.trim = state.trim.clamp(-limit, limit);
        let fuel = if cut {
            0.0
        } else {
            air / (STOICHIOMETRIC * target) * (1.0 + state.trim)
        };
        let lambda_sensor = if wideband {
            state.sensed
        } else {
            narrowband_volts(state.sensed)
        };
        Actuators {
            fuel,
            spark: spark.get(&point),
            idle_valve,
            cam_profile,
            intake_phase,
            exhaust_phase,
            lambda_sensor,
            trim: state.trim,
        }
    }
}

impl CamControl {
    /// The factory's cam control over the ECU's map `axes`: switching to the second
    /// profile (if there is one) where it gives more air at full load up to the redline,
    /// and phasing
    /// for the most air from the middle of the load range up.
    fn calibrated(
        hardware: &EngineHardware,
        maps: &EngineMaps,
        switching: bool,
        axes: impl Fn() -> Vec<Vec<f32>>,
    ) -> Self {
        let positions = |range: f32| [0.0, range / 2.0, range];
        let ratio = |map: f32| (1.05 * AMBIENT_PRESSURE).max(1.1 * map) / map;
        // The phasing with the most air at a point on a profile, and that air.
        let best = |profile: usize, rpm: f32, map: f32| {
            let mut best = (0.0, 0.0, 0.0);
            for intake in positions(hardware.intake_phasing) {
                for exhaust in positions(hardware.exhaust_phasing) {
                    let ve = maps.ve(profile, rpm, map, ratio(map), intake, exhaust);
                    if ve > best.2 {
                        best = (intake, exhaust, ve);
                    }
                }
            }
            best
        };
        // From where the second profile breathes better all the way to the redline.
        let mut switch_rpm = 9500.0;
        if switching && hardware.cams.len() > 1 {
            let mut rpm = hardware.redline();
            while rpm >= 2000.0 && best(1, rpm, 100e3).2 > best(0, rpm, 100e3).2 {
                switch_rpm = rpm;
                rpm -= 100.0;
            }
        }
        let phase = |p: &[f32], exhaust: bool| {
            let (rpm, map) = (p[0], p[1]);
            let profile = usize::from(rpm > switch_rpm && map > SWITCH_MAP);
            let (intake_phase, exhaust_phase, _) = best(profile, rpm, map);
            let share = ((map - PHASE_FROM) / (PHASE_TO - PHASE_FROM)).clamp(0.0, 1.0);
            share * if exhaust { exhaust_phase } else { intake_phase }
        };
        let none = || Table::bake(vec![vec![0.0]; 2], |_| 0.0);
        Self {
            switch_rpm,
            switch_map: SWITCH_MAP,
            intake_phase: Table::bake(axes(), |p| phase(p, false)),
            exhaust_phase: Table::bake(axes(), |p| phase(p, true)),
            // Made by `Ecu::calibrated`.
            ve: none(),
            lambda: none(),
            spark: none(),
        }
    }
}

/// What a tune file says it was made for: an engine build and an ECU (older tunes don't
/// say which ECU).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TuneOrigin {
    pub engine: Option<String>,
    pub ecu: Option<String>,
}

/// Tunes as text: a comment, `engine <name>` (the build it was made for), `ecu <name>` (the
/// ECU it was made on), the settings as `<key> <value>`, and the maps (see
/// `table::write_table`). The cam control's settings and maps are only written for ECUs
/// that have it.
impl Ecu {
    pub fn to_text(&self, engine: &str, ecu: &str) -> String {
        self.write_text(engine, ecu, self.model.cam_control())
    }

    /// As `to_text`, with the cam control's settings and maps whatever the ECU (a saved
    /// car's ECU comes back exactly as it was).
    pub fn to_text_whole(&self, engine: &str) -> String {
        self.write_text(engine, "", true)
    }

    fn write_text(&self, engine: &str, ecu: &str, cams: bool) -> String {
        let mut text = format!("# car-sim ECU tune\nengine {engine}\n");
        if !ecu.is_empty() {
            text += &format!("ecu {ecu}\n");
        }
        text += &format!("cylinders {}\n", self.cylinders);
        for (key, value) in self.settings() {
            if cams || !key.starts_with("cam_") {
                text += &format!("{key} {value}\n");
            }
        }
        for (name, table) in self.tables() {
            if cams || !(name.ends_with("_phase") || name.ends_with("_high")) {
                table::write_table(&mut text, name, table);
            }
        }
        text
    }

    /// Reads a tune written by `to_text` into this ECU; whatever is missing or broken keeps
    /// its value, and the ECU stays the model it is. Returns what the tune was made for.
    pub fn read_text(&mut self, text: &str) -> TuneOrigin {
        let mut origin = TuneOrigin::default();
        let mut tables = Vec::new();
        table::read_text(
            text,
            |key, value| match key {
                "engine" => origin.engine = Some(value.to_string()),
                "ecu" => origin.ecu = Some(value.to_string()),
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
            if let (Some(target), Some(table)) =
                (self.table(&name), Table::from_parts(axes, values))
            {
                *target = table;
            }
        }
        origin
    }

    fn settings(&self) -> [(&'static str, f32); 8] {
        [
            ("displacement", self.displacement),
            ("idle_rpm", self.idle_rpm),
            ("rev_limit", self.rev_limit),
            ("limiter_cut", self.limiter_cut),
            ("limiter_fuel", self.limiter_fuel),
            ("trim_limit", self.trim_limit),
            ("cam_switch_rpm", self.cams.switch_rpm),
            ("cam_switch_map", self.cams.switch_map),
        ]
    }

    fn setting(&mut self, key: &str) -> Option<&mut f32> {
        match key {
            "displacement" => Some(&mut self.displacement),
            "idle_rpm" => Some(&mut self.idle_rpm),
            "rev_limit" => Some(&mut self.rev_limit),
            "limiter_cut" => Some(&mut self.limiter_cut),
            "limiter_fuel" => Some(&mut self.limiter_fuel),
            "trim_limit" => Some(&mut self.trim_limit),
            "cam_switch_rpm" => Some(&mut self.cams.switch_rpm),
            "cam_switch_map" => Some(&mut self.cams.switch_map),
            _ => None,
        }
    }

    fn tables(&self) -> [(&'static str, &Table); 8] {
        [
            ("ve", &self.ve),
            ("lambda", &self.lambda),
            ("spark", &self.spark),
            ("intake_phase", &self.cams.intake_phase),
            ("exhaust_phase", &self.cams.exhaust_phase),
            ("ve_high", &self.cams.ve),
            ("lambda_high", &self.cams.lambda),
            ("spark_high", &self.cams.spark),
        ]
    }

    fn table(&mut self, name: &str) -> Option<&mut Table> {
        match name {
            "ve" => Some(&mut self.ve),
            "lambda" => Some(&mut self.lambda),
            "spark" => Some(&mut self.spark),
            "intake_phase" => Some(&mut self.cams.intake_phase),
            "exhaust_phase" => Some(&mut self.cams.exhaust_phase),
            "ve_high" => Some(&mut self.cams.ve),
            "lambda_high" => Some(&mut self.cams.lambda),
            "spark_high" => Some(&mut self.cams.spark),
            _ => None,
        }
    }
}

crate::snapshot::snapshot!(EcuState {
    limiter,
    idle,
    overrun,
    high_cam,
    sensed,
    trim,
    rich
});
crate::snapshot::snapshot!(Actuators {
    fuel,
    spark,
    idle_valve,
    cam_profile,
    intake_phase,
    exhaust_phase,
    lambda_sensor,
    trim,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::EngineBuild;
    use crate::physics::engine::hardware::EngineHardware;

    /// Each cam profile has its own maps by load, and an ECU that switches its own fuel,
    /// λ and spark maps for each.
    #[test]
    fn switching_cams_have_maps_of_their_own() {
        let mut parts = crate::physics::engine::parts::Parts::stock();
        parts.chosen[5] = 3;
        let hardware = parts.hardware();
        assert_eq!(hardware.cams.len(), 2);
        let build = EngineBuild::new(hardware.clone(), EcuModel::PROGRAMMABLE);
        let maps = &build.maps;
        let (rpm, load) = (7000.0, 0.9);
        let differs = |a: f32, b: f32| (a - b).abs() > 1e-3 * a.abs().max(1.0);
        assert!(differs(maps.mbt(0, rpm, load), maps.mbt(1, rpm, load)));
        assert!(differs(
            maps.exhaust_temperature(0, rpm, load),
            maps.exhaust_temperature(1, rpm, load)
        ));
        let ecu = &build.ecu;
        let point = [rpm, 100e3];
        assert!(differs(ecu.ve.get(&point), ecu.cams.ve.get(&point)));
        assert!(differs(ecu.spark.get(&point), ecu.cams.spark.get(&point)));
        // Without cam control the cams stay on the first profile: both the same.
        let stock = Ecu::calibrated(&hardware, maps, 95.0, EcuModel::STOCK);
        assert_eq!(stock.ve, stock.cams.ve);
        assert_eq!(stock.spark, stock.cams.spark);
    }

    #[test]
    fn tunes_survive_text() {
        let build = EngineBuild::default();
        let mut tuned = build.ecu.clone();
        tuned.rev_limit = 7234.5;
        tuned.spark.set_cell(&[10, 8], 31.25);
        tuned.ve.scale_at(&[3300.0, 71e3], 1.07);
        let text = tuned.to_text("test engine", "Stock");
        // Read into a different ECU (the turbo's, with longer maps): all of it comes over.
        let turbo = EngineBuild::new(EngineHardware::turbo(), EcuModel::STOCK);
        let mut read = turbo.ecu.clone();
        let origin = read.read_text(&text);
        assert_eq!(origin.engine.as_deref(), Some("test engine"));
        assert_eq!(origin.ecu.as_deref(), Some("Stock"));
        assert_eq!(read.to_text("test engine", "Stock"), text);
        assert_eq!(read.spark.cell(&[10, 8]), 31.25);
        assert_eq!(read.ve.values(), tuned.ve.values());
    }

    #[test]
    fn cam_control_switches_with_hysteresis_and_phases() {
        let mut ecu = EngineBuild::default().ecu;
        ecu.cams.switch_rpm = 5000.0;
        ecu.cams.switch_map = 60e3;
        ecu.cams.intake_phase = Table::bake(vec![vec![0.0], vec![0.0]], |_| 25.0);
        let mut state = EcuState::default();
        let mut step = |ecu: &Ecu, rpm, map| {
            let sensors = Sensors {
                rpm,
                map,
                throttle: 1.0,
                intake_temperature: 300.0,
                lambda: 1.0,
                wideband: false,
            };
            ecu.step(&mut state, &sensors, 0.01)
        };
        // The stock ECU leaves the cams alone.
        let stock = step(&ecu, 6000.0, 90e3);
        assert_eq!((stock.cam_profile, stock.intake_phase), (0, 0.0));
        ecu.model = EcuModel::PROGRAMMABLE;
        assert_eq!(step(&ecu, 4900.0, 90e3).cam_profile, 0);
        let high = step(&ecu, 5100.0, 90e3);
        assert_eq!((high.cam_profile, high.intake_phase), (1, 25.0));
        assert_eq!(step(&ecu, 4900.0, 90e3).cam_profile, 1);
        assert_eq!(step(&ecu, 4700.0, 90e3).cam_profile, 0);
        assert_eq!(step(&ecu, 6000.0, 40e3).cam_profile, 0);
    }

    #[test]
    fn cam_control_is_in_the_tune_only_with_a_programmable_ecu() {
        let mut ecu = EngineBuild::default().ecu;
        assert!(!ecu.to_text("e", "Stock").contains("intake_phase"));
        ecu.model = EcuModel::PROGRAMMABLE;
        ecu.cams.switch_rpm = 6123.0;
        ecu.cams.exhaust_phase.set_cell(&[12, 8], 17.0);
        let text = ecu.to_text("e", "Programmable");
        let mut read = EngineBuild::default().ecu;
        let origin = read.read_text(&text);
        assert_eq!(origin.ecu.as_deref(), Some("Programmable"));
        // It stays the ECU it is, but keeps what the tune says about the cams.
        assert_eq!(read.model, EcuModel::STOCK);
        assert_eq!(read.cams.switch_rpm, 6123.0);
        assert_eq!(read.cams.exhaust_phase.cell(&[12, 8]), 17.0);
    }

    #[test]
    fn broken_tunes_keep_what_they_cannot_replace() {
        let factory = EngineBuild::default().ecu;
        let mut ecu = factory.clone();
        let text = "rev_limit lots\nidle_rpm 900\ntable spark\naxis 0 1000\naxis 1 2\nvalues 1 2\n\
                    table lambda\naxis 0 500\nvalues 1 1\nvalues 1 1\n";
        assert_eq!(ecu.read_text(text), TuneOrigin::default());
        assert_eq!(ecu.rev_limit, factory.rev_limit);
        assert_eq!(ecu.idle_rpm, 900.0);
        // A spark map short of values and a λ map with the wrong axes stay as they were.
        assert_eq!(ecu.spark.values(), factory.spark.values());
        assert_eq!(ecu.lambda.values(), factory.lambda.values());
    }
}
