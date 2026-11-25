//! The built-in catalogue's engines: the families and their factory versions, each
//! version's block made from a few numbers (`Block`) and scaled from the 2-litre four.

use std::f32::consts::PI;

use super::parts::{PISTONS, SLOTS, TURBO};
use crate::physics::engine::ecu::EcuModel;
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

/// A block with a mild and a wild cam profile to switch between (durations in crank
/// degrees), an intake cam phaser, and springs for the wild one.
fn switching(b: Block, mild: f32, wild: f32) -> EngineHardware {
    let redline = b.redline;
    let mut hardware = block(b);
    let stock = hardware.cams[0];
    let profile = |duration: f32, separation: f32, lift: f32| CamProfile {
        intake_duration: duration,
        exhaust_duration: duration,
        lobe_separation: separation,
        lift: round_to(stock.lift * lift, 1e-4),
    };
    hardware.cams = vec![
        profile(mild, stock.lobe_separation + 2.0, 0.95),
        profile(wild, stock.lobe_separation - 2.0, 1.2),
    ];
    hardware.intake_phasing = 40.0;
    made_for(&mut hardware, redline);
    hardware
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

/// A turbo from the catalogue (1 small, 2 medium, 3 large for the engine's size) with the
/// wastegate set to `boost` bar.
fn turbo<const SIZE: usize, const BOOST_PERCENT: u32>(parts: &mut Preset) {
    parts.chosen[TURBO] = SIZE;
    parts.boost = BOOST_PERCENT as f32 / 100.0;
}

fn cam_control(parts: &mut Preset) {
    parts.ecu = EcuModel::Programmable;
}

pub fn families() -> Vec<Family> {
    vec![
        Family {
            name: "A-series three".into(),
            about: "A light, frugal three-cylinder for small cars".into(),
            layout: Layout::Inline3,
            length: 0.47,
            width: 0.58,
            height: 0.62,
            mass: 82.0,
            versions: vec![
                version(
                    "A10",
                    "1.0 three",
                    || {
                        block(Block {
                            cylinders: 3,
                            bore: 71.0,
                            stroke: 84.0,
                            compression: 10.5,
                            duration: 232.0,
                            separation: 112.0,
                            redline: 6400.0,
                            runner: 380.0,
                            header: 600.0,
                            inertia: 0.09,
                            flow: 1.0,
                        })
                    },
                    none,
                ),
                version(
                    "A10T",
                    "1.0 turbo three",
                    || {
                        block(Block {
                            cylinders: 3,
                            bore: 71.0,
                            stroke: 84.0,
                            compression: 9.5,
                            duration: 232.0,
                            separation: 112.0,
                            redline: 6400.0,
                            runner: 380.0,
                            header: 600.0,
                            inertia: 0.09,
                            flow: 1.0,
                        })
                    },
                    turbo::<1, 70>,
                ),
            ],
        },
        Family {
            name: "B-series four".into(),
            about: "A small four: light, simple and willing".into(),
            layout: Layout::Inline4,
            length: 0.58,
            width: 0.58,
            height: 0.63,
            mass: 100.0,
            versions: vec![
                version(
                    "B12",
                    "1.2 four",
                    || {
                        block(Block {
                            cylinders: 4,
                            bore: 71.0,
                            stroke: 75.6,
                            compression: 10.5,
                            duration: 236.0,
                            separation: 112.0,
                            redline: 6600.0,
                            runner: 360.0,
                            header: 650.0,
                            inertia: 0.10,
                            flow: 1.0,
                        })
                    },
                    none,
                ),
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
                    "B16S",
                    "1.6 sport four",
                    || {
                        block(Block {
                            cylinders: 4,
                            bore: 78.0,
                            stroke: 83.6,
                            compression: 11.0,
                            duration: 252.0,
                            separation: 108.0,
                            redline: 7400.0,
                            runner: 300.0,
                            header: 750.0,
                            inertia: 0.11,
                            flow: 1.1,
                        })
                    },
                    none,
                ),
            ],
        },
        Family {
            name: "R-series four".into(),
            about: "A high-revving four with switching cams and an intake cam phaser: the one \
                to swap into everything"
                .into(),
            layout: Layout::Inline4,
            length: 0.62,
            width: 0.60,
            height: 0.64,
            mass: 128.0,
            versions: vec![
                version(
                    "R20",
                    "2.0 high-rev four",
                    || {
                        switching(
                            Block {
                                cylinders: 4,
                                bore: 86.0,
                                stroke: 86.0,
                                compression: 11.0,
                                duration: 240.0,
                                separation: 110.0,
                                redline: 8400.0,
                                runner: 280.0,
                                header: 850.0,
                                inertia: 0.13,
                                flow: 1.25,
                            },
                            236.0,
                            268.0,
                        )
                    },
                    cam_control,
                ),
                version(
                    "R24",
                    "2.4 four",
                    || {
                        switching(
                            Block {
                                cylinders: 4,
                                bore: 87.0,
                                stroke: 99.0,
                                compression: 10.5,
                                duration: 240.0,
                                separation: 110.0,
                                redline: 7200.0,
                                runner: 320.0,
                                header: 850.0,
                                inertia: 0.16,
                                flow: 1.1,
                            },
                            232.0,
                            256.0,
                        )
                    },
                    cam_control,
                ),
            ],
        },
        Family {
            name: "W-series four".into(),
            about: "The 2-litre four found in everything, with or without a turbo".into(),
            layout: Layout::Inline4,
            length: 0.64,
            width: 0.60,
            height: 0.66,
            mass: 140.0,
            versions: vec![
                version("W20", "2.0 four", EngineHardware::default, none),
                version("W20T", "2.0 turbo four", EngineHardware::default, |parts| {
                    parts.chosen[PISTONS] = 1;
                    parts.chosen[TURBO] = 2;
                }),
            ],
        },
        Family {
            name: "X-series flat four".into(),
            about: "A turbocharged boxer: short, low and wide, with a burble of its own".into(),
            layout: Layout::Flat4,
            length: 0.46,
            width: 0.80,
            height: 0.52,
            mass: 135.0,
            versions: vec![
                version(
                    "X20T",
                    "2.0 turbo flat four",
                    || {
                        block(Block {
                            cylinders: 4,
                            bore: 92.0,
                            stroke: 75.0,
                            compression: 8.5,
                            duration: 240.0,
                            separation: 110.0,
                            redline: 7000.0,
                            runner: 300.0,
                            header: 600.0,
                            inertia: 0.15,
                            flow: 1.0,
                        })
                    },
                    turbo::<2, 80>,
                ),
                version(
                    "X25T",
                    "2.5 turbo flat four",
                    || {
                        block(Block {
                            cylinders: 4,
                            bore: 99.5,
                            stroke: 79.0,
                            compression: 8.4,
                            duration: 240.0,
                            separation: 112.0,
                            redline: 6600.0,
                            runner: 320.0,
                            header: 600.0,
                            inertia: 0.17,
                            flow: 1.0,
                        })
                    },
                    turbo::<2, 80>,
                ),
            ],
        },
        Family {
            name: "S-series straight six".into(),
            about: "A smooth straight six: long, so it needs a long engine bay".into(),
            layout: Layout::Inline6,
            length: 0.84,
            width: 0.60,
            height: 0.68,
            mass: 185.0,
            versions: vec![
                version(
                    "S30",
                    "3.0 straight six",
                    || {
                        block(Block {
                            cylinders: 6,
                            bore: 84.0,
                            stroke: 90.0,
                            compression: 10.5,
                            duration: 244.0,
                            separation: 110.0,
                            redline: 7000.0,
                            runner: 380.0,
                            header: 800.0,
                            inertia: 0.22,
                            flow: 1.0,
                        })
                    },
                    none,
                ),
                version(
                    "S30T",
                    "3.0 turbo straight six",
                    || {
                        block(Block {
                            cylinders: 6,
                            bore: 84.0,
                            stroke: 90.0,
                            compression: 9.0,
                            duration: 244.0,
                            separation: 110.0,
                            redline: 7000.0,
                            runner: 380.0,
                            header: 800.0,
                            inertia: 0.22,
                            flow: 1.0,
                        })
                    },
                    turbo::<2, 70>,
                ),
            ],
        },
        Family {
            name: "V-series V6".into(),
            about: "A compact V6 that fits across a car or along it".into(),
            layout: Layout::V6,
            length: 0.52,
            width: 0.72,
            height: 0.66,
            mass: 165.0,
            versions: vec![
                version(
                    "V35",
                    "3.5 V6",
                    || {
                        block(Block {
                            cylinders: 6,
                            bore: 94.0,
                            stroke: 84.0,
                            compression: 11.0,
                            duration: 244.0,
                            separation: 110.0,
                            redline: 7000.0,
                            runner: 330.0,
                            header: 700.0,
                            inertia: 0.20,
                            flow: 1.1,
                        })
                    },
                    none,
                ),
                version(
                    "V40",
                    "4.0 V6",
                    || {
                        block(Block {
                            cylinders: 6,
                            bore: 95.5,
                            stroke: 93.0,
                            compression: 10.0,
                            duration: 228.0,
                            separation: 114.0,
                            redline: 5800.0,
                            runner: 450.0,
                            header: 650.0,
                            inertia: 0.28,
                            flow: 1.0,
                        })
                    },
                    none,
                ),
            ],
        },
        Family {
            name: "H-series flat six".into(),
            about: "A flat six: low and wide, made to hang out behind the rear axle".into(),
            layout: Layout::Flat6,
            length: 0.62,
            width: 0.82,
            height: 0.54,
            mass: 185.0,
            versions: vec![
                version(
                    "H30",
                    "3.0 flat six",
                    || {
                        block(Block {
                            cylinders: 6,
                            bore: 91.0,
                            stroke: 76.4,
                            compression: 11.3,
                            duration: 248.0,
                            separation: 110.0,
                            redline: 7600.0,
                            runner: 300.0,
                            header: 750.0,
                            inertia: 0.17,
                            flow: 1.15,
                        })
                    },
                    none,
                ),
                version(
                    "H36T",
                    "3.6 turbo flat six",
                    || {
                        block(Block {
                            cylinders: 6,
                            bore: 100.0,
                            stroke: 76.4,
                            compression: 9.0,
                            duration: 244.0,
                            separation: 112.0,
                            redline: 7000.0,
                            runner: 300.0,
                            header: 600.0,
                            inertia: 0.19,
                            flow: 1.1,
                        })
                    },
                    turbo::<2, 80>,
                ),
            ],
        },
        Family {
            name: "M-series V8".into(),
            about: "A big cross-plane V8: torque everywhere, and the burble".into(),
            layout: Layout::V8,
            length: 0.70,
            width: 0.74,
            height: 0.70,
            mass: 210.0,
            versions: vec![
                version(
                    "M50",
                    "5.0 V8",
                    || {
                        block(Block {
                            cylinders: 8,
                            bore: 94.0,
                            stroke: 90.0,
                            compression: 10.5,
                            duration: 236.0,
                            separation: 114.0,
                            redline: 6400.0,
                            runner: 400.0,
                            header: 800.0,
                            inertia: 0.30,
                            flow: 1.0,
                        })
                    },
                    none,
                ),
                version(
                    "M62",
                    "6.2 V8",
                    || {
                        block(Block {
                            cylinders: 8,
                            bore: 103.25,
                            stroke: 92.0,
                            compression: 11.0,
                            duration: 240.0,
                            separation: 114.0,
                            redline: 6500.0,
                            runner: 380.0,
                            header: 850.0,
                            inertia: 0.32,
                            flow: 1.0,
                        })
                    },
                    none,
                ),
            ],
        },
        Family {
            name: "F-series V8".into(),
            about: "A flat-plane V8 that lives at the top of its rev range".into(),
            layout: Layout::FlatPlaneV8,
            length: 0.66,
            width: 0.72,
            height: 0.62,
            mass: 195.0,
            versions: vec![version(
                "F40",
                "4.0 flat-plane V8",
                || {
                    block(Block {
                        cylinders: 8,
                        bore: 92.0,
                        stroke: 75.2,
                        compression: 12.0,
                        duration: 260.0,
                        separation: 108.0,
                        redline: 8600.0,
                        runner: 250.0,
                        header: 700.0,
                        inertia: 0.24,
                        flow: 1.3,
                    })
                },
                none,
            )],
        },
        Family {
            name: "D-series V10".into(),
            about: "A howling V10".into(),
            layout: Layout::V10,
            length: 0.80,
            width: 0.72,
            height: 0.64,
            mass: 235.0,
            versions: vec![version(
                "D52",
                "5.2 V10",
                || {
                    block(Block {
                        cylinders: 10,
                        bore: 84.5,
                        stroke: 92.8,
                        compression: 12.0,
                        duration: 256.0,
                        separation: 108.0,
                        redline: 8500.0,
                        runner: 260.0,
                        header: 750.0,
                        inertia: 0.30,
                        flow: 1.3,
                    })
                },
                none,
            )],
        },
        Family {
            name: "L-series V12".into(),
            about: "Twelve cylinders: nothing is smoother, longer or heavier".into(),
            layout: Layout::V12,
            length: 0.94,
            width: 0.74,
            height: 0.68,
            mass: 275.0,
            versions: vec![version(
                "L65",
                "6.5 V12",
                || {
                    block(Block {
                        cylinders: 12,
                        bore: 95.0,
                        stroke: 76.4,
                        compression: 11.8,
                        duration: 256.0,
                        separation: 108.0,
                        redline: 8700.0,
                        runner: 260.0,
                        header: 700.0,
                        inertia: 0.38,
                        flow: 1.25,
                    })
                },
                none,
            )],
        },
    ]
}

#[cfg(test)]
mod tests {
    use crate::catalogue::builtin;

    #[test]
    fn blocks_are_what_they_say() {
        let catalogue = builtin();
        assert_eq!(catalogue.families[3].versions[0].code, "W20");
        for family in &catalogue.families {
            for version in &family.versions {
                let hardware = &version.block;
                assert_eq!(
                    hardware.cylinders,
                    family.layout.cylinders(),
                    "{}",
                    version.code
                );
                // The code and the name give the displacement: "W20", "2.0 four".
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
