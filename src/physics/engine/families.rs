//! The engines there are: families of engines that share a design (and so their parts,
//! their size and what bolts to them), each in a few factory versions. A version is a
//! block (bore, stroke, cams and the rest of the stock hardware) and the parts the factory
//! fits to it, such as a turbo. They're independent of the cars: any engine goes into any
//! car it fits into.

use std::f32::consts::PI;

use super::ecu::EcuModel;
use super::hardware::{
    BottomEnd, CamProfile, Cooling, EngineHardware, Exhaust, FLOAT_MARGIN, Intake, ROD_MARGIN,
    ValveTrain,
};
use super::parts::{PISTONS, Parts, TURBO, round_to};

/// How the cylinders are arranged, which is what an engine sounds like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Inline3,
    Inline4,
    Flat4,
    Inline6,
    Flat6,
    V6,
    /// A V8 with a cross-plane crank: each bank fires unevenly.
    V8,
    /// A V8 with a flat-plane crank: each bank fires like a four.
    FlatPlaneV8,
    V10,
    V12,
}

impl Layout {
    /// Every layout (the order is their number on the network).
    pub const ALL: [Layout; 10] = [
        Layout::Inline3,
        Layout::Inline4,
        Layout::Flat4,
        Layout::Inline6,
        Layout::Flat6,
        Layout::V6,
        Layout::V8,
        Layout::FlatPlaneV8,
        Layout::V10,
        Layout::V12,
    ];

    #[cfg(test)]
    fn cylinders(self) -> u32 {
        match self {
            Layout::Inline3 => 3,
            Layout::Inline4 | Layout::Flat4 => 4,
            Layout::Inline6 | Layout::Flat6 | Layout::V6 => 6,
            Layout::V8 | Layout::FlatPlaneV8 => 8,
            Layout::V10 => 10,
            Layout::V12 => 12,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Layout::Inline3 => "inline three",
            Layout::Inline4 => "inline four",
            Layout::Flat4 => "flat four",
            Layout::Inline6 => "straight six",
            Layout::Flat6 => "flat six",
            Layout::V6 => "V6",
            Layout::V8 => "cross-plane V8",
            Layout::FlatPlaneV8 => "flat-plane V8",
            Layout::V10 => "V10",
            Layout::V12 => "V12",
        }
    }
}

pub struct Family {
    pub name: &'static str,
    pub about: &'static str,
    pub layout: Layout,
    /// Length along the crank, width and height with its ancillaries (m), and its mass
    /// without a turbo (kg).
    pub length: f32,
    pub width: f32,
    pub height: f32,
    pub mass: f32,
    pub versions: &'static [Version],
}

pub struct Version {
    /// The factory's code, e.g. "W20T"; its maps are baked into `baked/<code>.maps`.
    pub code: &'static str,
    pub name: &'static str,
    block: fn() -> EngineHardware,
    /// The parts the factory fits, on a selection of all stock parts.
    preset: fn(&mut Parts),
    /// The maps baked with the game.
    pub maps: &'static str,
}

impl Family {
    /// The family and version with the factory code `code`.
    pub fn find(code: &str) -> Option<(usize, usize)> {
        FAMILIES.iter().enumerate().find_map(|(f, family)| {
            let version = family.versions.iter().position(|v| v.code == code)?;
            Some((f, version))
        })
    }

    /// Mass of an engine of this family as built (kg): a turbo with its plumbing and
    /// intercooler adds to it.
    pub fn built_mass(&self, hardware: &EngineHardware) -> f32 {
        let turbo = if hardware.turbo.is_some() {
            12.0 + 6.0 * relative_size(hardware)
        } else {
            0.0
        };
        self.mass + turbo
    }
}

impl Version {
    /// The block as the factory builds it, before any parts.
    pub fn block(&self) -> EngineHardware {
        (self.block)()
    }

    /// The parts the factory fits to version `version` of family `family`.
    pub fn parts(family: usize, version: usize) -> Parts {
        let mut parts = Parts::all_stock(family, version);
        (FAMILIES[family].versions[version].preset)(&mut parts);
        parts
    }
}

/// Displacement relative to the 2-litre four's, which the parts around the cylinders (and
/// turbos) are sized by.
pub fn relative_size(hardware: &EngineHardware) -> f32 {
    hardware.displacement() / EngineHardware::default().displacement()
}

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

const fn version(
    code: &'static str,
    name: &'static str,
    block: fn() -> EngineHardware,
    preset: fn(&mut Parts),
    maps: &'static str,
) -> Version {
    Version {
        code,
        name,
        block,
        preset,
        maps,
    }
}

fn none(_: &mut Parts) {}

/// A turbo from the catalogue (1 small, 2 medium, 3 large for the engine's size) with the
/// wastegate set to `boost` bar.
fn turbo<const SIZE: usize, const BOOST_PERCENT: u32>(parts: &mut Parts) {
    parts.chosen[TURBO] = SIZE;
    parts.boost = BOOST_PERCENT as f32 / 100.0;
}

fn cam_control(parts: &mut Parts) {
    parts.ecu = EcuModel::Programmable;
}

/// The family of the 2-litre four that the others are scaled from.
pub const WORKHORSE: usize = 3;

pub static FAMILIES: [Family; 12] = [
    Family {
        name: "A-series three",
        about: "A light, frugal three-cylinder for small cars",
        layout: Layout::Inline3,
        length: 0.47,
        width: 0.58,
        height: 0.62,
        mass: 82.0,
        versions: &[
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
                include_str!("baked/a10.maps"),
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
                include_str!("baked/a10t.maps"),
            ),
        ],
    },
    Family {
        name: "B-series four",
        about: "A small four: light, simple and willing",
        layout: Layout::Inline4,
        length: 0.58,
        width: 0.58,
        height: 0.63,
        mass: 100.0,
        versions: &[
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
                include_str!("baked/b12.maps"),
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
                include_str!("baked/b16.maps"),
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
                include_str!("baked/b16s.maps"),
            ),
        ],
    },
    Family {
        name: "R-series four",
        about: "A high-revving four with switching cams and an intake cam phaser: the one \
                to swap into everything",
        layout: Layout::Inline4,
        length: 0.62,
        width: 0.60,
        height: 0.64,
        mass: 128.0,
        versions: &[
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
                include_str!("baked/r20.maps"),
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
                include_str!("baked/r24.maps"),
            ),
        ],
    },
    Family {
        name: "W-series four",
        about: "The 2-litre four found in everything, with or without a turbo",
        layout: Layout::Inline4,
        length: 0.64,
        width: 0.60,
        height: 0.66,
        mass: 140.0,
        versions: &[
            version(
                "W20",
                "2.0 four",
                EngineHardware::default,
                none,
                include_str!("baked/w20.maps"),
            ),
            version(
                "W20T",
                "2.0 turbo four",
                EngineHardware::default,
                |parts| {
                    parts.chosen[PISTONS] = 1;
                    parts.chosen[TURBO] = 2;
                },
                include_str!("baked/w20t.maps"),
            ),
        ],
    },
    Family {
        name: "X-series flat four",
        about: "A turbocharged boxer: short, low and wide, with a burble of its own",
        layout: Layout::Flat4,
        length: 0.46,
        width: 0.80,
        height: 0.52,
        mass: 135.0,
        versions: &[
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
                include_str!("baked/x20t.maps"),
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
                include_str!("baked/x25t.maps"),
            ),
        ],
    },
    Family {
        name: "S-series straight six",
        about: "A smooth straight six: long, so it needs a long engine bay",
        layout: Layout::Inline6,
        length: 0.84,
        width: 0.60,
        height: 0.68,
        mass: 185.0,
        versions: &[
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
                include_str!("baked/s30.maps"),
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
                include_str!("baked/s30t.maps"),
            ),
        ],
    },
    Family {
        name: "V-series V6",
        about: "A compact V6 that fits across a car or along it",
        layout: Layout::V6,
        length: 0.52,
        width: 0.72,
        height: 0.66,
        mass: 165.0,
        versions: &[
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
                include_str!("baked/v35.maps"),
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
                include_str!("baked/v40.maps"),
            ),
        ],
    },
    Family {
        name: "H-series flat six",
        about: "A flat six: low and wide, made to hang out behind the rear axle",
        layout: Layout::Flat6,
        length: 0.62,
        width: 0.82,
        height: 0.54,
        mass: 185.0,
        versions: &[
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
                include_str!("baked/h30.maps"),
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
                include_str!("baked/h36t.maps"),
            ),
        ],
    },
    Family {
        name: "M-series V8",
        about: "A big cross-plane V8: torque everywhere, and the burble",
        layout: Layout::V8,
        length: 0.70,
        width: 0.74,
        height: 0.70,
        mass: 210.0,
        versions: &[
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
                include_str!("baked/m50.maps"),
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
                include_str!("baked/m62.maps"),
            ),
        ],
    },
    Family {
        name: "F-series V8",
        about: "A flat-plane V8 that lives at the top of its rev range",
        layout: Layout::FlatPlaneV8,
        length: 0.66,
        width: 0.72,
        height: 0.62,
        mass: 195.0,
        versions: &[version(
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
            include_str!("baked/f40.maps"),
        )],
    },
    Family {
        name: "D-series V10",
        about: "A howling V10",
        layout: Layout::V10,
        length: 0.80,
        width: 0.72,
        height: 0.64,
        mass: 235.0,
        versions: &[version(
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
            include_str!("baked/d52.maps"),
        )],
    },
    Family {
        name: "L-series V12",
        about: "Twelve cylinders: nothing is smoother, longer or heavier",
        layout: Layout::V12,
        length: 0.94,
        width: 0.74,
        height: 0.68,
        mass: 275.0,
        versions: &[version(
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
            include_str!("baked/l65.maps"),
        )],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_what_they_say() {
        assert_eq!(FAMILIES[WORKHORSE].versions[0].code, "W20");
        for family in &FAMILIES {
            for version in family.versions {
                let hardware = version.block();
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
                assert_eq!(
                    Family::find(version.code).map(|(f, _)| FAMILIES[f].name),
                    Some(family.name)
                );
            }
        }
    }

    #[test]
    fn springs_and_rods_are_made_for_the_redline() {
        for family in &FAMILIES {
            for version in family.versions {
                let hardware = version.block();
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
