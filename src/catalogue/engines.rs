//! The built-in catalogue's engines: the families and their factory versions, each
//! version's block made from a few numbers (`Block`) and scaled from the 2-litre four.

use std::f32::consts::PI;

use super::parts::SLOTS;
use crate::physics::engine::families::{Family, Layout, Version, relative_size};
use crate::physics::engine::hardware::{
    BottomEnd, CamProfile, Cooling, EngineHardware, Exhaust, FLOAT_MARGIN, Intake, ROD_MARGIN,
    ValveTrain,
};
use crate::physics::engine::parts::{Preset, round_to};

/// What sets one block apart from another; the rest follows from its size.
struct Block {
    cylinders: u32,
    /// Bore and stroke (mm).
    bore: f32,
    stroke: f32,
    compression: f32,
    /// Cam duration (crank degrees) and lobe separation (cam degrees).
    duration: f32,
    separation: f32,
    /// The speed its valve springs and rods are made for (rpm).
    redline: f32,
    /// Intake runner and header primary lengths (mm).
    runner: f32,
    header: f32,
    /// Flywheel and crank inertia (kg·m²).
    inertia: f32,
    /// How well the head flows (see `EngineHardware::port_flow`).
    flow: f32,
}

/// The hardware of a block: the 2-litre four's cylinder scaled to the bore (masses with
/// its cube, forces with its square), the parts around the cylinders sized by the
/// displacement, and springs and rods for the redline.
fn block(b: Block) -> EngineHardware {
    let reference = EngineHardware::default();
    let (bore, stroke) = (b.bore / 1000.0, b.stroke / 1000.0);
    let scale = bore / reference.bore;
    let cam = CamProfile {
        intake_duration: b.duration,
        exhaust_duration: b.duration,
        lobe_separation: b.separation,
        lift: round_to(0.11 * bore, 1e-4),
    };
    let mut hardware = EngineHardware {
        name: String::new(),
        cylinders: b.cylinders,
        bore,
        stroke,
        rod_length: round_to(1.66 * stroke, 1e-3),
        compression_ratio: b.compression,
        bottom_end: BottomEnd {
            piston_mass: round_to(0.42 * scale.powi(3), 1e-3),
            rod_mass: round_to(0.6 * scale * scale * stroke / reference.stroke, 1e-3),
            rod_strength: 0.0,
            piston_toughness: 1.0,
        },
        head_gasket: reference.head_gasket,
        cams: vec![cam],
        intake_phasing: 0.0,
        exhaust_phasing: 0.0,
        intake_timing: 0.0,
        exhaust_timing: 0.0,
        valves: ValveTrain {
            seat_force: reference.valves.seat_force * scale * scale,
            rate: reference.valves.rate * scale,
            mass: round_to(reference.valves.mass * scale.powi(3), 1e-3),
        },
        port_flow: b.flow,
        intake: reference.intake,
        turbo: None,
        supercharger: None,
        exhaust: reference.exhaust,
        cooling: reference.cooling,
        inertia: b.inertia,
    };
    let size = relative_size(&hardware);
    let (intake, exhaust, cooling) = (reference.intake, reference.exhaust, reference.cooling);
    hardware.intake = Intake {
        filter_area: intake.filter_area * size,
        throttle_diameter: round_to(intake.throttle_diameter * size.sqrt(), 1e-3),
        idle_valve_area: intake.idle_valve_area * size,
        runner_length: b.runner / 1000.0,
        plenum_volume: intake.plenum_volume * size,
        tract_volume: intake.tract_volume * size,
    };
    hardware.exhaust = Exhaust {
        header_length: b.header / 1000.0,
        pipe_area: exhaust.pipe_area * size,
        cat_area: exhaust.cat_area.map(|area| area * size),
        silencer_area: exhaust.silencer_area.map(|area| area * size),
    };
    hardware.cooling = Cooling {
        radiator_area: cooling.radiator_area * size,
        coolant_capacity: cooling.coolant_capacity * (0.4 + 0.6 * size),
        oil_capacity: cooling.oil_capacity * (0.4 + 0.6 * size),
        ..cooling
    };
    made_for(&mut hardware, b.redline);
    hardware
}

/// Fits valve springs that keep the valves on the (wildest) cam a little past `redline`,
/// and rods that take the pull there with their margin (and the push of the gas, like the
/// 2-litre four's, if that needs more).
fn made_for(hardware: &mut EngineHardware, redline: f32) {
    let cam = hardware.cams[hardware.cams.len() - 1];
    let float = (redline + 60.0) / FLOAT_MARGIN;
    let stiffer = (float / hardware.valves.float_rpm(&cam)).powi(2);
    hardware.valves.seat_force = (hardware.valves.seat_force * stiffer).round();
    hardware.valves.rate = round_to(hardware.valves.rate * stiffer, 100.0);
    let omega = 1.09 * redline * 2.0 * PI / 60.0;
    let pull = hardware.rod_pull(omega) / ROD_MARGIN;
    let reference = EngineHardware::default();
    let push = reference.bottom_end.rod_strength * (hardware.bore / reference.bore).powi(2);
    hardware.bottom_end.rod_strength = round_to(pull.max(push), 100.0);
}

fn version(
    code: &str,
    name: &str,
    block: fn() -> EngineHardware,
    preset: fn(&mut Preset),
) -> Version {
    let mut parts = Preset::stock(SLOTS);
    preset(&mut parts);
    Version {
        code: code.into(),
        name: name.into(),
        block: block(),
        preset: parts,
    }
}

fn none(_: &mut Preset) {}

pub fn families() -> Vec<Family> {
    vec![Family {
        name: "B-series four".into(),
        about: "A small four: light, simple and willing".into(),
        layout: Layout::Inline4,
        length: 0.58,
        width: 0.58,
        height: 0.63,
        mass: 100.0,
        versions: vec![
            version(
                "B16",
                "1.6 four",
                || {
                    block(Block {
                        cylinders: 4,
                        bore: 78.0,
                        stroke: 83.6,
                        compression: 10.5,
                        duration: 240.0,
                        separation: 110.0,
                        redline: 6800.0,
                        runner: 350.0,
                        header: 700.0,
                        inertia: 0.13,
                        flow: 1.0,
                    })
                },
                none,
            ),
            version(
                "B18",
                "1.8 four",
                || {
                    block(Block {
                        cylinders: 4,
                        bore: 81.0,
                        stroke: 87.2,
                        compression: 10.0,
                        duration: 244.0,
                        separation: 110.0,
                        redline: 6700.0,
                        runner: 360.0,
                        header: 700.0,
                        inertia: 0.14,
                        flow: 1.0,
                    })
                },
                none,
            ),
        ],
    }]
}

#[cfg(test)]
mod tests {
    use crate::catalogue::builtin;

    #[test]
    fn blocks_are_what_they_say() {
        let catalogue = builtin();
        assert_eq!(catalogue.families[0].versions[0].code, "B16");
        for family in &catalogue.families {
            for version in &family.versions {
                let hardware = &version.block;
                assert_eq!(
                    hardware.cylinders,
                    family.layout.cylinders(),
                    "{}",
                    version.code
                );
                // The code and the name give the displacement: "B16", "1.6 four".
                let litres = hardware.displacement() * 1e3;
                let digits: String = version.code.chars().filter(char::is_ascii_digit).collect();
                let coded = digits.parse::<f32>().unwrap() / 10.0;
                assert!(
                    (litres - coded).abs() < 0.06,
                    "{}: {litres} l",
                    version.code
                );
                assert!(
                    version.name.starts_with(&format!("{coded:.1} ")),
                    "{}",
                    version.name
                );
                let found = catalogue.find_engine(&version.code);
                assert_eq!(
                    found.map(|(f, _)| &catalogue.families[f].name),
                    Some(&family.name)
                );
            }
        }
    }

    #[test]
    fn springs_and_rods_are_made_for_the_redline() {
        for family in &builtin().families {
            for version in &family.versions {
                let hardware = &version.block;
                let redline = hardware.redline();
                assert!(
                    (5800.0..=8700.0).contains(&redline) && redline % 100.0 == 0.0,
                    "{}: {redline} rpm",
                    version.code
                );
                // The valves float before the rods are in danger, and a naturally
                // aspirated full load is far from buckling them.
                let load = hardware.rod_load(redline, 80e5);
                assert!(load < 0.7, "{}: rod load {load}", version.code);
            }
        }
    }
}
