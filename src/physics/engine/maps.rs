//! The engine's maps: what the cylinders do at every operating point, baked from the
//! hardware, and curves that are the same for every engine.
//!
//! For now the maps are baked from placeholder formulas that respond to the hardware in
//! the right direction: longer cam durations breathe best at higher rpm, runner and
//! header lengths tune a band, overlap costs breathing at low manifold pressure, and
//! compression raises efficiency. A cycle simulation will fill the same tables later.

use super::hardware::{CamProfile, EngineHardware};
use super::table::{Table, curve};

/// Grid of the baked maps, in rpm.
fn rpm_axis() -> Vec<f32> {
    (0..20).map(|i| i as f32 * 500.0).collect()
}

/// Intake manifold pressure (Pa).
fn pressure_axis() -> Vec<f32> {
    [
        10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 130.0, 160.0,
        200.0, 250.0, 300.0,
    ]
    .map(|kpa| kpa * 1000.0)
    .to_vec()
}

/// Exhaust over intake manifold pressure.
fn ratio_axis() -> Vec<f32> {
    vec![0.5, 0.7, 0.85, 1.0, 1.15, 1.3, 1.6, 2.0, 3.0, 4.0, 6.0, 10.0]
}

/// Air per cycle relative to filling the displacement at ambient conditions.
fn load_axis() -> Vec<f32> {
    vec![0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.2, 1.5, 2.0, 2.5, 3.0]
}

/// A cam phaser's positions from 0 to `range` (crank degrees), or just 0 without one.
fn phase_axis(range: f32) -> Vec<f32> {
    if range > 0.0 {
        (0..5).map(|i| range * i as f32 / 4.0).collect()
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
    /// Friction mean effective pressure (Pa) with the oil warm, by rpm.
    friction: Table,
}

impl EngineMaps {
    pub fn bake(hardware: &EngineHardware) -> Self {
        let ve = hardware
            .cams
            .iter()
            .map(|cam| {
                let axes = vec![
                    rpm_axis(),
                    pressure_axis(),
                    ratio_axis(),
                    phase_axis(hardware.intake_phasing),
                    phase_axis(hardware.exhaust_phasing),
                ];
                Table::bake(axes, |p| ve_placeholder(hardware, cam, p[0], p[2], p[3], p[4]))
            })
            .collect();
        let by_load = |f: &dyn Fn(f32, f32) -> f32| {
            Table::bake(vec![rpm_axis(), load_axis()], |p| f(p[0], p[1]))
        };
        let cr = hardware.compression_ratio;
        // Faster burning at higher compression and load needs less advance.
        let mbt = by_load(&|rpm, load| {
            10.0 + 20.0 * (1.0 - (-rpm / 2500.0).exp()) + 12.0 * (1.0 - load.min(1.2))
                - 0.8 * (cr - 10.0)
        });
        // Knock needs time and pressure: high load, high compression and big bores knock
        // earlier, high rpm leaves the end gas less time.
        let bore = hardware.bore;
        let knock_limit = by_load(&|rpm, load| {
            30.0 + 2.0 * (rpm / 1000.0).min(7.0) - 14.0 * load - 3.0 * (cr - 10.0)
                - 2.0 * (bore - 0.086) / 0.01
        });
        // The ideal Otto cycle, scaled down; more heat is lost at low rpm and light load.
        let otto = 1.0 - cr.powf(-0.3);
        let efficiency = by_load(&|rpm, load| {
            0.82 * otto * (1.0 - 0.1 * (-rpm / 1500.0).exp()) * (0.92 + 0.08 * load.min(1.0))
        });
        let exhaust_temperature = by_load(&|rpm, load| 880.0 + 150.0 * load.min(1.5) + 0.03 * rpm);
        // More of the heat goes into the walls at low rpm (more time) and light load.
        let heat_to_coolant = by_load(&|rpm, load| {
            0.32 * (1.0 - 0.25 * load.min(1.5)) * (1.0 + 0.15 * (-rpm / 2000.0).exp())
        });
        // Rings and skirts by mean piston speed, more side thrust with shorter rods.
        let rod = 1.0 + 0.5 * (hardware.stroke / (2.0 * hardware.rod_length) - 0.3);
        let friction = Table::bake(vec![rpm_axis()], |p| {
            let speed = 2.0 * hardware.stroke * p[0] / 60.0;
            (60e3 + 4.2e3 * speed + 0.4e3 * speed * speed) * rod
        });
        Self {
            ve,
            mbt,
            efficiency,
            knock_limit,
            exhaust_temperature,
            heat_to_coolant,
            friction,
        }
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

    /// Friction mean effective pressure (Pa) at `rpm` with the oil at `oil` (K).
    pub fn friction(&self, rpm: f32, oil: f32) -> f32 {
        self.friction.get(&[rpm]) * friction_oil_factor(oil)
    }
}

/// Volumetric efficiency for a cam profile: a breathing curve peaking at an rpm set by the
/// cam duration (and moved down by advancing the intake cam), bumps where the runners and
/// headers are tuned, less filling from the residual gas left at high exhaust over intake
/// pressure, and backflow through the valve overlap at low intake pressure.
fn ve_placeholder(
    hardware: &EngineHardware,
    cam: &CamProfile,
    rpm: f32,
    ratio: f32,
    intake_phase: f32,
    exhaust_phase: f32,
) -> f32 {
    let duration = (cam.intake_duration + cam.exhaust_duration) / 2.0;
    let peak_rpm = (4600.0 + 60.0 * (duration - 240.0) - 25.0 * intake_phase) * 0.086
        / hardware.stroke;
    let width = if rpm < peak_rpm { 3200.0 } else { 3000.0 };
    let x = (rpm - peak_rpm) / width;
    // Flow through the valve stops growing at a lift of about a quarter of its diameter.
    let lift = (cam.lift / (0.4 * hardware.bore) / 0.25).min(1.2).powf(0.25);
    let breathing = 0.9 * lift * (1.0 - 0.18 * x * x).max(0.2);

    let bump = |tuned: f32, width: f32| (-((rpm - tuned) / width).powi(2)).exp();
    let runner = 0.06 * bump(1700.0 / hardware.intake.runner_length, 1200.0);
    let overlap = cam.overlap(intake_phase, exhaust_phase);
    // Scavenging needs the overlap, and an exhaust pressure not far above the intake.
    let scavenging = (overlap / 20.0).min(1.5) * ((1.5 - ratio) / 0.5).clamp(0.0, 1.0);
    let header = 0.04 * scavenging * bump(4500.0 / hardware.exhaust.header_length, 1500.0);

    let cr = hardware.compression_ratio;
    let residual = ((cr - ratio.powf(1.0 / 1.3)) / (cr - 1.0)).max(0.0);
    let backflow = 0.0015 * overlap * (ratio - 1.0).max(0.0) * (2000.0 / rpm.max(600.0)).min(3.0);
    ((breathing + runner + header) * residual * (1.0 - backflow).max(0.0)).max(0.0)
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

    fn wide_open_ve(hardware: &EngineHardware, profile: usize, rpm: f32) -> f32 {
        EngineMaps::bake(hardware).ve(profile, rpm, 100e3, 1.05, 0.0, 0.0)
    }

    #[test]
    fn runners_tune_the_band() {
        let long = EngineHardware::default();
        let mut short = long.clone();
        short.intake.runner_length = 0.22;
        assert!(wide_open_ve(&short, 0, 7500.0) > wide_open_ve(&long, 0, 7500.0));
        assert!(wide_open_ve(&short, 0, 4500.0) < wide_open_ve(&long, 0, 4500.0));
    }

    #[test]
    fn a_wild_cam_breathes_high_and_idles_poorly() {
        let mut hardware = EngineHardware::default();
        let stock = hardware.cams[0];
        hardware.cams.push(CamProfile {
            intake_duration: 280.0,
            exhaust_duration: 280.0,
            lobe_separation: 106.0,
            lift: 0.012,
        });
        let ve = |profile, rpm| wide_open_ve(&hardware, profile, rpm);
        assert!(ve(1, 7500.0) > ve(0, 7500.0));
        assert!(ve(1, 2000.0) < ve(0, 2000.0));
        // At idle (low manifold pressure), the overlap lets exhaust back in.
        let maps = EngineMaps::bake(&hardware);
        let idle = |profile| maps.ve(profile, 850.0, 30e3, 3.4, 0.0, 0.0);
        assert!(idle(1) < idle(0) * 0.9, "{} vs {}", idle(1), idle(0));
        assert!(stock.overlap(0.0, 0.0) < hardware.cams[1].overlap(0.0, 0.0));
    }

    #[test]
    fn cold_oil_adds_friction() {
        let maps = EngineMaps::bake(&EngineHardware::default());
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
    fn knock_limits_full_load_at_low_rpm() {
        let hardware = EngineHardware::default();
        let maps = EngineMaps::bake(&hardware);
        let margin = |rpm, load| maps.knock_limit(rpm, load) - maps.mbt(rpm, load);
        assert!(margin(2000.0, 1.0) < 0.0);
        assert!(margin(3000.0, 0.4) > -2.0);
        // More compression knocks earlier.
        let mut high = hardware.clone();
        high.compression_ratio = 12.0;
        let high = EngineMaps::bake(&high);
        assert!(high.knock_limit(2000.0, 1.0) < maps.knock_limit(2000.0, 1.0) - 3.0);
    }
}
