//! Turbocharger physics: the compressor along its speed lines, the turbine as a nozzle
//! with a blade-speed efficiency, the wastegate actuator, and the intercooled charge.
//! The maps are parametric for now; measured maps can replace these functions later.

use super::hardware::{Compressor, Turbine, Turbo, Wastegate};
use super::{AMBIENT_PRESSURE, AMBIENT_TEMPERATURE, R_AIR, orifice_flow};

/// Specific heats (J/(kg·K)) of air and exhaust gas, and their isentropic exponents
/// (γ - 1)/γ.
const AIR_HEAT: f32 = 1005.0;
const EXHAUST_HEAT: f32 = 1150.0;
const AIR_EXPONENT: f32 = 0.2857;
const EXHAUST_EXPONENT: f32 = 0.248;
/// Turbine blade speed ratio (tip speed over spouting velocity) of best efficiency.
const BEST_BLADE_SPEED: f32 = 0.65;
/// Least compressor efficiency, far from its island (also when barely turning).
const LEAST_EFFICIENCY: f32 = 0.4;

/// Where the compressor runs.
pub struct CompressorPoint {
    /// Air through it (kg/s), torque it takes from the shaft (N·m), and the temperature of
    /// the air leaving it (K).
    pub flow: f32,
    pub torque: f32,
    pub temperature: f32,
}

/// The compressor at shaft `speed` (rad/s) between `inlet` and `outlet` pressures (Pa),
/// drawing air at `temperature` (K). Above the head it can make there's no flow (surge,
/// without the backflow). Standing still, it's a restriction.
pub fn compressor(
    c: &Compressor,
    speed: f32,
    inlet: f32,
    outlet: f32,
    temperature: f32,
) -> CompressorPoint {
    let tip = speed * c.diameter / 2.0;
    let head = AIR_HEAT * temperature * ((outlet / inlet).powf(AIR_EXPONENT) - 1.0);
    let density = inlet / (R_AIR * temperature);
    let area = density * c.diameter * c.diameter;
    let reserve = c.head * tip * tip - head;
    let flow = if reserve > 0.0 {
        area * c.choke * (reserve / c.head).sqrt()
    } else {
        0.0
    };
    let coefficient = flow / (area * tip.max(1.0));
    let off = (coefficient - 0.6 * c.choke) / (0.5 * c.choke);
    let efficiency = (c.efficiency * (1.0 - off * off)).max(LEAST_EFFICIENCY);
    let work = head.max(0.0) / efficiency;
    CompressorPoint {
        flow,
        torque: flow * work / speed.max(1.0),
        temperature: temperature + work / AIR_HEAT,
    }
}

/// Exhaust through the turbine (kg/s) and the torque it gives the shaft (N·m), at shaft
/// `speed` (rad/s) between `inlet` and `outlet` pressures (Pa) with gas at `temperature`.
pub fn turbine(t: &Turbine, speed: f32, inlet: f32, outlet: f32, temperature: f32) -> (f32, f32) {
    let flow = orifice_flow(t.area, inlet, outlet, temperature);
    let expansion = 1.0 - (outlet / inlet).min(1.0).powf(EXHAUST_EXPONENT);
    let spouting = (2.0 * EXHAUST_HEAT * temperature * expansion).sqrt();
    if spouting < 1.0 {
        return (flow, 0.0);
    }
    // Efficiency η·x(2 - x) with x the blade speed ratio over the best one; the torque
    // (power over speed) stays finite with the shaft standing still.
    let radius = t.diameter / 2.0;
    let x = speed * radius / (BEST_BLADE_SPEED * spouting);
    let torque =
        flow * t.efficiency * (2.0 - x).max(0.0) * radius * spouting / (2.0 * BEST_BLADE_SPEED);
    (flow, torque)
}

/// How far the wastegate's spring lets it open (0..1) at `boost` above ambient (Pa).
pub fn wastegate_opening(w: &Wastegate, boost: f32) -> f32 {
    ((boost - w.spring) / w.span).clamp(0.0, 1.0)
}

/// Charge temperature after the compressor and intercooler (K), from the compressor's
/// outlet temperature.
pub fn intercooled(turbo: &Turbo, outlet: f32) -> f32 {
    AMBIENT_TEMPERATURE + (outlet - AMBIENT_TEMPERATURE) * (1.0 - turbo.intercooler.effectiveness)
}

/// Charge temperature (K) expected at `boost` pressure (absolute, Pa), with the
/// compressor near its best efficiency: for tuning.
pub fn expected_charge(turbo: &Turbo, boost: f32) -> f32 {
    let ratio = (boost / AMBIENT_PRESSURE).max(1.0);
    let rise = AMBIENT_TEMPERATURE * (ratio.powf(AIR_EXPONENT) - 1.0) / turbo.compressor.efficiency;
    intercooled(turbo, AMBIENT_TEMPERATURE + rise)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::hardware::EngineHardware;

    #[test]
    fn compressor_speed_lines() {
        let turbo = EngineHardware::turbo().turbo.unwrap();
        let c = &turbo.compressor;
        let t = AMBIENT_TEMPERATURE;
        let flow = |speed: f32, ratio: f32| compressor(c, speed, 1e5, 1e5 * ratio, t).flow;
        // Faster, more flow at the same pressure ratio; more pressure, less flow.
        assert!(flow(15000.0, 1.8) > flow(12000.0, 1.8));
        assert!(flow(15000.0, 2.0) < flow(15000.0, 1.5));
        // Past the head the wheel can make: surge, no flow.
        assert_eq!(flow(8000.0, 2.2), 0.0);
        // Standing still it only passes air downhill.
        assert!(flow(0.0, 0.9) > 0.0 && flow(0.0, 1.1) == 0.0);
        // Boost at about 1.75 bar with 0.15 kg/s needs around 130 000 rpm, and heats the air
        // to about 90 °C.
        let point = compressor(c, 13600.0, 1e5, 1.75e5, t);
        assert!((0.08..0.25).contains(&point.flow), "{} kg/s", point.flow);
        assert!(
            (340.0..400.0).contains(&point.temperature),
            "{} K",
            point.temperature
        );
    }

    #[test]
    fn turbine_torque_falls_with_speed() {
        let turbo = EngineHardware::turbo().turbo.unwrap();
        let torque = |speed| turbine(&turbo.turbine, speed, 2e5, 1.05e5, 1100.0).1;
        assert!(torque(0.0) > torque(10000.0));
        assert!(torque(10000.0) > 0.0);
        assert_eq!(torque(100000.0), 0.0);
    }
}
