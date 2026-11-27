//! Superchargers: compressors the crank drives through a belt, so the boost follows the
//! engine's speed at once (no lag), at the cost of the power it takes to drive them.
//!
//! A **Roots blower** is a positive-displacement pump: its rotors carry a fixed volume of
//! air round per turn, so it delivers in proportion to its speed and makes most of its
//! boost from low down (less what leaks back between the rotors, which matters most when
//! it turns slowly). It compresses by pushing air against the manifold's pressure, which is
//! poor work: the charge comes out hot and it takes a lot of power. It sits on top of the
//! engine, through the bonnet.
//!
//! A **centrifugal** supercharger is a turbo's compressor driven by the belt through a
//! step-up gear: its head goes with the square of the impeller's speed, so the boost
//! rises with the rpm squared, little low down, most at the redline; more efficient,
//! cooler. It sits beside the engine, under the bonnet.
//!
//! Both have a bypass valve that opens when the throttle shuts on them (vacuum in the
//! manifold), so at part throttle they idle round with little to push.

use super::hardware::{Compressor, Supercharger};
use super::turbo::{self, CompressorPoint};
use super::{AMBIENT_PRESSURE, AMBIENT_TEMPERATURE, R_AIR, orifice_flow};

/// Specific heat of air (J/(kg·K)), and its isentropic exponent (γ - 1)/γ.
const AIR_HEAT: f32 = 1005.0;
const AIR_EXPONENT: f32 = 0.2857;
/// How much hotter a Roots blower's charge comes out than its work alone heats it (the air
/// leaking back is heated again), as a share of that.
const ROOTS_REHEAT: f32 = 0.15;

/// Crank speed (rad/s) per rpm.
const RAD_PER_RPM: f32 = std::f32::consts::TAU / 60.0;

/// Where the supercharger runs at `rpm` (the crank's) between `inlet` and `outlet` (Pa),
/// drawing air at `temperature` (K): the air it delivers (kg/s), the torque it takes at
/// the crank (N·m, belt drag included), and the charge's temperature.
pub fn point(
    s: &Supercharger,
    rpm: f32,
    inlet: f32,
    outlet: f32,
    temperature: f32,
) -> CompressorPoint {
    let rpm = rpm.max(0.0);
    let drag = (s.drag[0] + s.drag[1] * rpm / 1000.0) * (rpm / 30.0).min(1.0);
    let crank = rpm * RAD_PER_RPM;
    if s.roots {
        // Revolutions of the rotors a second, and the air they carry round.
        let turns = rpm / 60.0 * s.ratio;
        let density = inlet / (R_AIR * temperature);
        let carried = density * s.displacement * turns;
        // What leaks back through the clearances (or on through them, downhill).
        let leak = if outlet > inlet {
            -orifice_flow(s.leak, outlet, inlet, temperature)
        } else {
            orifice_flow(s.leak, inlet, outlet, temperature)
        };
        // Pushing each pocket against the outlet's pressure: the work (W).
        let power = (outlet - inlet).max(0.0) * s.displacement * turns;
        let heated = power / (carried.max(1e-4) * AIR_HEAT) * (1.0 + ROOTS_REHEAT);
        CompressorPoint {
            flow: (carried + leak).max(0.0),
            torque: power / crank.max(1.0) + drag,
            temperature: temperature + heated,
        }
    } else {
        let impeller = crank * s.ratio;
        let point = turbo::compressor(&s.compressor, impeller, inlet, outlet, temperature);
        CompressorPoint {
            torque: point.torque * s.ratio + drag,
            ..point
        }
    }
}

/// Charge temperature (K) expected at `boost` pressure (absolute, Pa): for tuning.
pub fn expected_charge(s: &Supercharger, boost: f32) -> f32 {
    let ratio = (boost / AMBIENT_PRESSURE).max(1.0);
    let t = AMBIENT_TEMPERATURE;
    if s.roots {
        // Pushed against the outlet's pressure: Δp over ρ·c_p.
        t + AIR_EXPONENT * t * (ratio - 1.0) * (1.0 + ROOTS_REHEAT)
    } else {
        t + t * (ratio.powf(AIR_EXPONENT) - 1.0) / s.compressor.efficiency
    }
}

/// A compressor for a centrifugal supercharger's impeller of `diameter` (m).
pub fn impeller(diameter: f32) -> Compressor {
    Compressor {
        diameter,
        head: 0.7,
        choke: 0.29,
        efficiency: 0.72,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::ecu::EcuModel;
    use crate::physics::engine::hardware::EngineHardware;
    use crate::physics::engine::parts::Parts;
    use crate::physics::engine::{Engine, EngineBuild};

    /// The stock 1.6 with the catalogue's supercharger `name` in its slot.
    pub fn charged(name: &str) -> EngineHardware {
        let mut parts = Parts::stock();
        let slot = &parts.catalogue.slots[crate::catalogue::SUPERCHARGER];
        let part = (slot.parts.iter())
            .position(|part| part.name == name)
            .expect("a supercharger");
        parts.chosen[crate::catalogue::SUPERCHARGER] = part;
        parts.hardware()
    }

    #[test]
    fn a_roots_blower_delivers_by_its_speed_and_heats_the_charge() {
        let hardware = charged("Roots blower");
        let s = hardware.supercharger.expect("a supercharger");
        assert!(s.roots && hardware.turbo.is_none());
        let flow = |rpm, ratio: f32| point(&s, rpm, 1e5, 1e5 * ratio, 298.0).flow;
        // Twice as fast, about twice the air; slow, more of it leaks back.
        assert!(flow(4000.0, 1.5) > 1.9 * flow(2000.0, 1.5));
        assert!(flow(1000.0, 1.5) < 0.5 * flow(2000.0, 1.5));
        // Against more pressure, more work and a hotter charge.
        let low = point(&s, 4000.0, 1e5, 1.2e5, 298.0);
        let high = point(&s, 4000.0, 1e5, 1.6e5, 298.0);
        assert!(high.torque > low.torque && high.temperature > low.temperature + 20.0);
        assert!((330.0..420.0).contains(&expected_charge(&s, 1.6e5)));
    }

    /// The engine after `seconds` flat out at `rpm`, tuned for what it is.
    fn flat_out(build: &EngineBuild, rpm: f32, seconds: f32) -> Engine {
        let mut engine = Engine {
            airspeed: 30.0,
            ..Engine::default()
        };
        for _ in 0..(seconds * 120.0) as usize {
            engine.step(build, 1.0, rpm, true, 1.0 / 120.0);
        }
        engine
    }

    fn boost(engine: &Engine) -> f32 {
        (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5
    }

    #[test]
    fn the_superchargers_boost_curves() {
        let na = EngineBuild::new(Parts::stock().hardware(), EcuModel::Stock);
        let roots = EngineBuild::new(charged("Roots blower"), EcuModel::Stock);
        let centrifugal = EngineBuild::new(charged("Centrifugal supercharger"), EcuModel::Stock);
        let curve = |build: &EngineBuild| {
            [2000.0, 3000.0, 6000.0].map(|rpm| boost(&flat_out(build, rpm, 3.0)))
        };
        let [r2, r3, r6] = curve(&roots);
        let [c2, c3, c6] = curve(&centrifugal);
        println!("Roots {r2:.2} {r3:.2} {r6:.2} bar, centrifugal {c2:.2} {c3:.2} {c6:.2} bar");
        // The Roots blower boosts from low down, about as much all the way up.
        assert!(r6 > 0.25 && r2 > 0.5 * r6, "{r2} {r6}");
        // The centrifugal one's boost grows with the revs squared: little low down.
        assert!(c6 > 0.25 && c3 < 0.4 * c6 && c2 < c3, "{c2} {c3} {c6}");
        // More torque than without, where each boosts; the Roots blower's hot charge.
        let torque = |build: &EngineBuild, rpm: f32| flat_out(build, rpm, 3.0).torque(build, rpm);
        assert!(torque(&roots, 2500.0) > 1.15 * torque(&na, 2500.0));
        assert!(torque(&centrifugal, 6500.0) > 1.15 * torque(&na, 6500.0));
        let hot = flat_out(&roots, 5000.0, 6.0).charge_temperature - 273.15;
        assert!((50.0..130.0).contains(&hot), "charge {hot} °C");
        // Off the throttle the bypass lets it idle round: next to no boost.
        let mut engine = flat_out(&roots, 3000.0, 2.0);
        for _ in 0..240 {
            engine.step(&roots, 0.1, 3000.0, true, 1.0 / 120.0);
        }
        assert!(boost(&engine) < 0.1, "{} bar", boost(&engine));
    }

    #[test]
    fn a_supercharged_engines_factory_tune_doesnt_knock() {
        for name in ["Roots blower", "Centrifugal supercharger"] {
            let build = EngineBuild::new(charged(name), EcuModel::Stock);
            for rpm in [2000.0, 3500.0, 5000.0, 6500.0] {
                let engine = flat_out(&build, rpm, 4.0);
                let cycle = engine.cycle(&build, rpm);
                assert_eq!(cycle.knock, 0.0, "{name} at {rpm} rpm");
                assert!(
                    cycle.lambda < 0.9,
                    "{name} at {rpm} rpm: λ {}",
                    cycle.lambda
                );
            }
            // A while flat out wears nothing.
            let engine = flat_out(&build, 6000.0, 60.0);
            let wear = engine.damage.worst(4);
            assert!(wear < 1e-3, "{name}: wear {wear}");
        }
    }
}
