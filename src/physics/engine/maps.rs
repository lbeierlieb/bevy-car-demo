//! The engine's maps: what the cylinders do at every operating point, baked from the
//! hardware by the cycle simulation (`cycle`), and curves that are the same for every
//! engine.
//!
//! Baking takes a few seconds, so maps are kept (see `store`), each with a fingerprint of
//! the hardware and of the simulation's code.

use super::cycle::{Conditions, Cylinder};
use super::hardware::EngineHardware;
use super::table::{self, Table, curve};
use super::{AMBIENT_PRESSURE, AMBIENT_TEMPERATURE, R_AIR};

/// Conditions the maps are baked at: intake air and coolant temperatures (K), and the
/// fuel's octane number. The knock limit's shifts (`knock_limit_shift`) start from them.
const REFERENCE_CHARGE: f64 = 300.0;
const REFERENCE_COOLANT: f64 = 363.0;
const REFERENCE_OCTANE: f64 = 95.0;
/// Exhaust pressures simulated (Pa): the ratio axis reaches pressures no engine sees.
const EXHAUST_RANGE: (f64, f64) = (10e3, 800e3);

/// Grid of the baked maps, in rpm.
fn rpm_axis() -> Vec<f32> {
    (0..20).map(|i| i as f32 * 500.0).collect()
}

/// Intake manifold pressure (Pa).
fn pressure_axis() -> Vec<f32> {
    [
        10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 130.0, 160.0, 200.0,
        250.0, 300.0,
    ]
    .map(|kpa| kpa * 1000.0)
    .to_vec()
}

/// Exhaust over intake manifold pressure; coarser with cam phasers, whose phase axes
/// multiply the grid.
fn ratio_axis(phasers: bool) -> Vec<f32> {
    if phasers {
        vec![0.5, 0.85, 1.15, 1.6, 3.0, 10.0]
    } else {
        vec![
            0.5, 0.7, 0.85, 1.0, 1.15, 1.3, 1.6, 2.0, 3.0, 4.0, 6.0, 10.0,
        ]
    }
}

/// Air per cycle relative to filling the displacement at ambient conditions.
fn load_axis() -> Vec<f32> {
    vec![
        0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.2, 1.5, 2.0, 2.5, 3.0,
    ]
}

/// A cam phaser's positions from 0 to `range` (crank degrees), or just 0 without one.
/// Three points keep the bake short: with both phasers they make the VE grid 9 times
/// bigger.
fn phase_axis(range: f32) -> Vec<f32> {
    if range > 0.0 {
        vec![0.0, range / 2.0, range]
    } else {
        vec![0.0]
    }
}

#[derive(Clone, Debug)]
pub struct EngineMaps {
    /// Volumetric efficiency per cam profile (air trapped per cycle relative to filling the
    /// displacement at intake manifold density) by rpm, intake manifold pressure, exhaust
    /// over intake pressure, intake phase and exhaust phase.
    ve: Vec<Table>,
    /// Spark advance for best torque (degrees before top dead centre) by rpm and load.
    mbt: Table,
    /// Indicated efficiency at MBT spark and λ = 1 by rpm and load.
    efficiency: Table,
    /// Spark advance where knock starts (degrees before top dead centre) by rpm and load,
    /// at the reference conditions of `knock_limit_shift`.
    knock_limit: Table,
    /// Exhaust gas temperature at MBT spark and λ = 1 (K) by rpm and load.
    exhaust_temperature: Table,
    /// Share of the fuel's energy that goes into the coolant, by rpm and load.
    heat_to_coolant: Table,
    /// Highest cylinder pressure at MBT spark (Pa) by rpm and load.
    peak_pressure: Table,
    /// Friction mean effective pressure (Pa) with the oil warm, by rpm.
    friction: Table,
}

impl EngineMaps {
    /// Maps with nothing in them (zero everywhere), for a car only to be looked at: its
    /// engine never runs on them.
    pub fn blank() -> Self {
        let table = |axes: usize| Table::bake(vec![vec![0.0]; axes], |_| 0.0);
        Self {
            ve: vec![table(5)],
            mbt: table(2),
            efficiency: table(2),
            knock_limit: table(2),
            exhaust_temperature: table(2),
            heat_to_coolant: table(2),
            peak_pressure: table(2),
            friction: table(1),
        }
    }

    /// Bakes the maps by simulating cycles all over the grid (on all cores, where there
    /// are threads).
    pub fn bake(hardware: &EngineHardware) -> Self {
        let mut bake = Bake::new(hardware);
        loop {
            if let Some(maps) = bake.advance(usize::MAX) {
                return maps;
            }
        }
    }

    /// The maps as text: the hardware's fingerprint, then the tables (see
    /// `table::write_table`).
    pub fn to_text(&self, hardware: &EngineHardware) -> String {
        let mut text = format!(
            "# car-sim engine maps: {}\nfingerprint {:016x}\n",
            hardware.name,
            fingerprint(hardware)
        );
        for (i, ve) in self.ve.iter().enumerate() {
            table::write_table(&mut text, &format!("ve{i}"), ve);
        }
        for (name, table) in self.named() {
            table::write_table(&mut text, name, table);
        }
        text
    }

    /// Maps read from `to_text`'s text, if they're all there and for hardware with this
    /// fingerprint.
    pub fn from_text(text: &str, fingerprint: u64) -> Option<Self> {
        let mut matches = false;
        let mut tables = Vec::new();
        table::read_text(
            text,
            |key, value| {
                if key == "fingerprint" {
                    matches = u64::from_str_radix(value, 16) == Ok(fingerprint);
                }
            },
            |name, axes, values| tables.push((name.to_string(), Table::from_parts(axes, values))),
        );
        if !matches {
            return None;
        }
        let mut take = |name: &str| {
            let i = tables.iter().position(|(n, _)| n == name)?;
            tables.swap_remove(i).1
        };
        let ve: Vec<Table> = (0..).map_while(|i| take(&format!("ve{i}"))).collect();
        if ve.is_empty() {
            return None;
        }
        Some(Self {
            ve,
            mbt: take("mbt")?,
            efficiency: take("efficiency")?,
            knock_limit: take("knock_limit")?,
            exhaust_temperature: take("exhaust_temperature")?,
            heat_to_coolant: take("heat_to_coolant")?,
            peak_pressure: take("peak_pressure")?,
            friction: take("friction")?,
        })
    }

    fn named(&self) -> [(&'static str, &Table); 7] {
        [
            ("mbt", &self.mbt),
            ("efficiency", &self.efficiency),
            ("knock_limit", &self.knock_limit),
            ("exhaust_temperature", &self.exhaust_temperature),
            ("heat_to_coolant", &self.heat_to_coolant),
            ("peak_pressure", &self.peak_pressure),
            ("friction", &self.friction),
        ]
    }

    pub fn ve(
        &self,
        profile: usize,
        rpm: f32,
        intake: f32,
        ratio: f32,
        intake_phase: f32,
        exhaust_phase: f32,
    ) -> f32 {
        let table = &self.ve[profile.min(self.ve.len() - 1)];
        table.get(&[rpm, intake, ratio, intake_phase, exhaust_phase])
    }

    pub fn mbt(&self, rpm: f32, load: f32) -> f32 {
        self.mbt.get(&[rpm, load])
    }

    pub fn efficiency(&self, rpm: f32, load: f32) -> f32 {
        self.efficiency.get(&[rpm, load])
    }

    /// Spark advance where knock starts at the reference conditions.
    pub fn knock_limit(&self, rpm: f32, load: f32) -> f32 {
        self.knock_limit.get(&[rpm, load])
    }

    pub fn exhaust_temperature(&self, rpm: f32, load: f32) -> f32 {
        self.exhaust_temperature.get(&[rpm, load])
    }

    pub fn heat_to_coolant(&self, rpm: f32, load: f32) -> f32 {
        self.heat_to_coolant.get(&[rpm, load])
    }

    /// Highest cylinder pressure at MBT spark (Pa).
    pub fn peak_pressure(&self, rpm: f32, load: f32) -> f32 {
        self.peak_pressure.get(&[rpm, load])
    }

    /// Friction mean effective pressure (Pa) at `rpm` with the oil at `oil` (K).
    pub fn friction(&self, rpm: f32, oil: f32) -> f32 {
        self.friction.get(&[rpm]) * friction_oil_factor(oil)
    }
}

/// Baking the maps a batch of grid points at a time, so it can be spread over frames and
/// show its progress: first the volumetric efficiency, then the maps by load (which find
/// their intake pressures in it).
pub struct Bake {
    hardware: EngineHardware,
    /// Grid points of the VE tables (cam profile and point), and their values so far.
    ve_points: Vec<(usize, Vec<f32>)>,
    ve_values: Vec<f32>,
    /// The VE tables once they're done.
    ve: Vec<Table>,
    /// Grid points of the maps by load, and their values so far (MBT, efficiency, knock
    /// limit, exhaust temperature, heat to coolant, peak pressure).
    load_points: Vec<Vec<f32>>,
    load_values: Vec<[f32; 6]>,
}

impl Bake {
    pub fn new(hardware: &EngineHardware) -> Self {
        let axes = ve_axes(hardware);
        let ve_points = (0..hardware.cams.len())
            .flat_map(|profile| grid(&axes).into_iter().map(move |point| (profile, point)))
            .collect();
        Self {
            hardware: hardware.clone(),
            ve_points,
            ve_values: Vec::new(),
            ve: Vec::new(),
            load_points: grid(&load_axes()),
            load_values: Vec::new(),
        }
    }

    /// Share of the work done (0..1).
    pub fn progress(&self) -> f32 {
        let done = self.ve_values.len() + self.load_values.len();
        done as f32 / (self.ve_points.len() + self.load_points.len()) as f32
    }

    /// Bakes up to `count` more grid points (on all cores, where there are threads), and
    /// returns the maps when it has done the last.
    pub fn advance(&mut self, count: usize) -> Option<EngineMaps> {
        let hardware = &self.hardware;
        let done = self.ve_values.len();
        if done < self.ve_points.len() {
            let batch = &self.ve_points[done..self.ve_points.len().min(done.saturating_add(count))];
            let values = parallel(batch, |(profile, point)| {
                ve_point(hardware, *profile, point)
            });
            self.ve_values.extend(values);
            if self.ve_values.len() == self.ve_points.len() {
                let size = self.ve_points.len() / hardware.cams.len();
                self.ve = self
                    .ve_values
                    .chunks(size)
                    .map(|values| {
                        let axes = ve_axes(hardware);
                        Table::from_parts(axes, values.to_vec()).expect("finite maps")
                    })
                    .collect();
            }
            return None;
        }
        let done = self.load_values.len();
        let end = self.load_points.len().min(done.saturating_add(count));
        let ve = &self.ve[0];
        let values = parallel(&self.load_points[done..end], |point| {
            load_point(hardware, ve, point)
        });
        self.load_values.extend(values);
        (self.load_values.len() == self.load_points.len()).then(|| self.finish())
    }

    fn finish(&mut self) -> EngineMaps {
        let hardware = &self.hardware;
        let column = |i: usize| {
            let values = self.load_values.iter().map(|values| values[i]).collect();
            Table::from_parts(load_axes(), values).expect("finite maps")
        };
        // Rings and skirts by mean piston speed, more side thrust with shorter rods.
        let rod = 1.0 + 0.5 * (hardware.stroke / (2.0 * hardware.rod_length) - 0.3);
        let friction = Table::bake(vec![rpm_axis()], |p| {
            let speed = 2.0 * hardware.stroke * p[0] / 60.0;
            (60e3 + 4.2e3 * speed + 0.4e3 * speed * speed) * rod
        });
        EngineMaps {
            ve: std::mem::take(&mut self.ve),
            mbt: column(0),
            efficiency: column(1),
            knock_limit: column(2),
            exhaust_temperature: column(3),
            heat_to_coolant: column(4),
            peak_pressure: column(5),
            friction,
        }
    }
}

fn ve_axes(hardware: &EngineHardware) -> Vec<Vec<f32>> {
    vec![
        rpm_axis(),
        pressure_axis(),
        ratio_axis(hardware.intake_phasing > 0.0 || hardware.exhaust_phasing > 0.0),
        phase_axis(hardware.intake_phasing),
        phase_axis(hardware.exhaust_phasing),
    ]
}

fn load_axes() -> Vec<Vec<f32>> {
    vec![rpm_axis(), load_axis()]
}

/// The cycle simulation's conditions at the reference temperatures and fuel.
fn conditions(rpm: f32, intake: f64, exhaust: f64) -> Conditions {
    Conditions {
        rpm: f64::from(rpm),
        intake_pressure: intake,
        exhaust_pressure: exhaust.clamp(EXHAUST_RANGE.0, EXHAUST_RANGE.1),
        intake_temperature: REFERENCE_CHARGE,
        coolant_temperature: REFERENCE_COOLANT,
        octane: REFERENCE_OCTANE,
    }
}

/// Volumetric efficiency on cam profile `profile` at a point of `ve_axes`.
fn ve_point(hardware: &EngineHardware, profile: usize, point: &[f32]) -> f32 {
    let cam = &hardware.cams[profile];
    let cylinder = Cylinder::new(hardware, cam, point[3], point[4]);
    let intake = f64::from(point[1]);
    let conditions = conditions(point[0], intake, intake * f64::from(point[2]));
    let result = cylinder.run(&conditions, None);
    result.volumetric_efficiency(&cylinder, &conditions) as f32
}

/// The maps by load at a point of `load_axes` (MBT, efficiency, knock limit, exhaust
/// temperature, heat to coolant, peak pressure), on the first cam profile, unphased, finding the intake
/// pressure for the load in `ve`, the first profile's VE.
fn load_point(hardware: &EngineHardware, ve: &Table, point: &[f32]) -> [f32; 6] {
    let (rpm, load) = (point[0], point[1]);
    // The exhaust pressure the engine would about have: a little over ambient, or over
    // the boost.
    let exhaust = |intake: f64| {
        let over = if hardware.turbo.is_some() { 1.3 } else { 1.1 };
        (1.05 * f64::from(AMBIENT_PRESSURE)).max(over * intake)
    };
    let reference = AMBIENT_PRESSURE / (R_AIR * AMBIENT_TEMPERATURE);
    let load_at = |intake: f32| {
        let ratio = exhaust(f64::from(intake)) as f32 / intake;
        let ve = ve.get(&[rpm, intake, ratio, 0.0, 0.0]);
        ve * intake / (R_AIR * REFERENCE_CHARGE as f32) / reference
    };
    // The intake pressure that traps this much air.
    let (mut low, mut high) = (2e3f32, 1e6f32);
    for _ in 0..40 {
        let mid = (low + high) / 2.0;
        if load_at(mid) < load {
            low = mid;
        } else {
            high = mid;
        }
    }
    let intake = f64::from(low + high) / 2.0;
    let conditions = conditions(rpm, intake, exhaust(intake));
    let cylinder = Cylinder::new(hardware, &hardware.cams[0], 0.0, 0.0);
    let result = cylinder.run(&conditions, None);
    let mbt = cylinder.mbt(&result.trapped, &conditions);
    let knock_limit = cylinder.knock_limit(&result.trapped, &conditions);
    // The work at MBT: the closed part at MBT, the gas exchange as it ran. The runtime
    // model takes the pumping as (exhaust − intake pressure) × the swept volume, so the
    // efficiency is of the work before that.
    let burn = cylinder.burn(&result.trapped, &conditions, mbt);
    let closed = burn.work;
    let swept = f64::from(hardware.displacement()) / f64::from(hardware.cylinders);
    let pumping = (conditions.exhaust_pressure - intake) * swept;
    let work = result.work - result.closed_work + closed + pumping;
    // Without any fresh charge trapped, there's nothing to burn.
    let per_fuel = |energy: f64| {
        if result.fuel_energy > 0.0 {
            energy / result.fuel_energy
        } else {
            0.0
        }
    };
    [
        mbt,
        per_fuel(work),
        knock_limit,
        result.exhaust_temperature,
        per_fuel(result.heat),
        burn.peak,
    ]
    .map(|value| value as f32)
}

/// A fingerprint of what the maps depend on: the cylinder side of the hardware, whether
/// it has a turbo (for the exhaust pressures by load), and the code that bakes them.
pub fn fingerprint(hardware: &EngineHardware) -> u64 {
    let h = hardware;
    let key = format!(
        "{} {} {} {} {} {:?} {} {} {} {} {} {} {} {}",
        h.cylinders,
        h.bore,
        h.stroke,
        h.rod_length,
        h.compression_ratio,
        h.cams,
        h.intake_phasing,
        h.exhaust_phasing,
        h.intake_timing,
        h.exhaust_timing,
        h.intake.runner_length,
        h.exhaust.header_length,
        h.turbo.is_some(),
        h.port_flow,
    );
    // FNV-1a over the code without its tests, and without line ends so a checkout's
    // don't matter.
    let code = [include_str!("cycle.rs"), include_str!("maps.rs")]
        .map(|code| code.split("#[cfg(test)]\nmod tests").next().unwrap_or(code));
    let bytes = key
        .bytes()
        .chain(code.iter().flat_map(|c| c.bytes()))
        .filter(|&b| b != b'\r');
    bytes.fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Every point of a grid on `axes`, the last axis varying fastest (as in `Table`).
fn grid(axes: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let mut points = vec![Vec::new()];
    for axis in axes {
        points = points
            .into_iter()
            .flat_map(|point| {
                axis.iter().map(move |&x| {
                    let mut point = point.clone();
                    point.push(x);
                    point
                })
            })
            .collect();
    }
    points
}

/// `f` of each item, in order: spread over the cores, or on this thread on the web.
fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let next = AtomicUsize::new(0);
        let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut done = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(item) = items.get(i) else {
                                return done;
                            };
                            done.push((i, f(item)));
                        }
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|worker| worker.join().expect("baking thread"))
                .collect()
        });
        done.sort_by_key(|(i, _)| *i);
        done.into_iter().map(|(_, result)| result).collect()
    }
    #[cfg(target_arch = "wasm32")]
    items.iter().map(f).collect()
}

/// Shift of the knock limit (degrees) from its reference conditions: charge at 300 K,
/// coolant at 363 K, 95 octane (RON). Hot charge and hot coolant knock earlier, better
/// fuel later.
pub fn knock_limit_shift(charge: f32, coolant: f32, octane: f32) -> f32 {
    -0.08 * (charge - 300.0) - 0.1 * (coolant - 363.0) + 1.5 * (octane - 95.0)
}

/// Friction relative to warm oil: thick cold oil almost doubles it.
pub fn friction_oil_factor(oil: f32) -> f32 {
    1.0 + 0.9 * ((363.0 - oil) / 65.0).clamp(0.0, 1.0)
}

/// Fraction of the best torque at a spark advance `offset` degrees from MBT.
pub fn spark_efficiency(offset: f32) -> f32 {
    (1.0 - 4.5e-4 * offset * offset).max(0.0)
}

/// Torque relative to λ = 1 for the same air: best a little rich, misfiring far from it.
const LAMBDA: [(f32, f32); 12] = [
    (0.5, 0.0),
    (0.6, 0.8),
    (0.7, 0.95),
    (0.8, 1.02),
    (0.88, 1.04),
    (0.95, 1.025),
    (1.0, 1.0),
    (1.1, 0.93),
    (1.2, 0.86),
    (1.3, 0.78),
    (1.45, 0.55),
    (1.6, 0.0),
];

pub fn lambda_efficiency(lambda: f32) -> f32 {
    curve(&LAMBDA, lambda)
}

/// Change of exhaust temperature (K) from a spark `offset` from MBT (retarded is hotter)
/// and from λ (rich is cooler, slightly lean hotter).
pub fn exhaust_temperature_shift(offset: f32, lambda: f32) -> f32 {
    8.0 * (-offset).max(0.0) - 3.0 * offset.max(0.0) - 400.0 * (1.0 - lambda).max(0.0)
        + 300.0 * (lambda - 1.0).clamp(0.0, 0.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_for_other_hardware_are_not_taken_from_the_stock() {
        let stock = EngineHardware::default();
        let text = EngineMaps::for_hardware(&stock).to_text(&stock);
        let mut ported = stock.clone();
        ported.cams[0].lift += 0.001;
        assert!(EngineMaps::from_text(&text, fingerprint(&ported)).is_none());
        // Parts that act live don't change the maps.
        let mut freer = stock.clone();
        freer.exhaust.cat_area = None;
        assert_eq!(fingerprint(&freer), fingerprint(&stock));
    }

    #[test]
    fn cold_oil_adds_friction() {
        let maps = EngineMaps::for_hardware(&EngineHardware::default());
        let (cold, warm) = (maps.friction(3000.0, 298.0), maps.friction(3000.0, 368.0));
        assert!(cold > warm * 1.5, "{cold} vs {warm}");
    }

    #[test]
    fn spark_and_mixture_cost_torque_away_from_best() {
        assert_eq!(spark_efficiency(0.0), 1.0);
        assert!(spark_efficiency(-10.0) > 0.9 && spark_efficiency(-30.0) < 0.7);
        assert!(lambda_efficiency(0.88) > 1.0);
        assert!(lambda_efficiency(1.2) < 0.9 && lambda_efficiency(2.0) == 0.0);
        assert!(exhaust_temperature_shift(-15.0, 1.0) > 100.0);
        assert!(exhaust_temperature_shift(0.0, 0.85) < 0.0);
        assert!(knock_limit_shift(330.0, 363.0, 95.0) < -2.0);
        assert!(knock_limit_shift(300.0, 363.0, 98.0) > 4.0);
    }

    #[test]
    fn the_stock_maps_look_like_an_engine() {
        let maps = EngineMaps::for_hardware(&EngineHardware::default());
        let ve = |rpm, intake, ratio| maps.ve(0, rpm, intake, ratio, 0.0, 0.0);
        assert!((0.85..1.05).contains(&ve(4500.0, 100e3, 1.05)));
        assert!(ve(850.0, 30e3, 3.4) < ve(850.0, 100e3, 1.05));
        let efficiency = maps.efficiency(4000.0, 1.0);
        assert!((0.35..0.45).contains(&efficiency), "{efficiency}");
        assert!(maps.efficiency(2000.0, 0.2) < efficiency);
        let exhaust = maps.exhaust_temperature(5000.0, 1.0);
        assert!((1050.0..1350.0).contains(&exhaust), "{exhaust} K");
        let heat = maps.heat_to_coolant(3000.0, 0.8);
        assert!((0.15..0.35).contains(&heat), "{heat}");
        // Full load knocks short of MBT at low rpm; the spark has room at high rpm.
        let margin = |rpm, load| maps.knock_limit(rpm, load) - maps.mbt(rpm, load);
        assert!(margin(1500.0, 0.9) < -1.0);
        assert!(margin(6500.0, 0.9) > 0.0);
        assert!(margin(3000.0, 0.4) > 0.0);
        assert!((15.0..40.0).contains(&maps.mbt(4000.0, 0.9)));
    }
}
