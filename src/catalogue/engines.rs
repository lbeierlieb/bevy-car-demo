//! The built-in catalogue's engines: the families and their factory versions, each
//! version's block made from a few numbers (`Block`) and scaled from the 2-litre four.

use std::f32::consts::PI;

use super::EcuDef;
use super::parts::SLOTS;
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::families::{
    Family, HeadDef, Layout, ManifoldDef, Version, relative_size,
};
use crate::physics::engine::hardware::{
    BottomEnd, CamDrive, CamProfile, Collector, Cooling, EngineHardware, Exhaust, FLOAT_MARGIN,
    Intake, Manifold, ROD_MARGIN, ValveLayout, ValveTrain,
};
use crate::physics::engine::maps::EngineMaps;
use crate::physics::engine::parts::{Preset, round_to};
use crate::physics::fuel::PUMP_OCTANE;

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
    /// A second cam profile to switch to, if it has one: its duration, lobe separation
    /// and lift over the first's.
    high: Option<(f32, f32, f32)>,
    /// The range of an intake cam phaser in its head (crank degrees), 0 without one.
    phaser: f32,
    /// The speed its valve springs and rods are made for (rpm).
    redline: f32,
    /// Intake runner and header primary lengths (mm).
    runner: f32,
    header: f32,
    /// Flywheel and crank inertia (kg·m²).
    inertia: f32,
    /// How well the head flows (see `EngineHardware::port_flow`).
    flow: f32,
    /// Its valvetrain.
    head: Head,
}

/// Where a block's cams are and what they move.
#[derive(Clone, Copy)]
struct Head {
    layout: ValveLayout,
    /// Valve lift over the lifter's, and the lifter's and pushrod's mass (kg; see
    /// `ValveTrain`).
    rocker_ratio: f32,
    cam_side_mass: f32,
    rollers: bool,
    /// Valve lift relative to the bore.
    lift: f32,
}

impl Head {
    /// One cam over two valves per cylinder, working them through rockers.
    const SINGLE_CAM: Self = Self {
        layout: ValveLayout {
            drive: CamDrive::Sohc,
            valves: 2,
            intake_diameter: 0.46,
            exhaust_diameter: 0.38,
        },
        rocker_ratio: 1.0,
        cam_side_mass: 0.0,
        rollers: false,
        lift: 0.11,
    };
    /// Two cams over four big valves.
    const BIG_VALVES: Self = Self {
        layout: ValveLayout {
            drive: CamDrive::Dohc,
            valves: 4,
            intake_diameter: 0.41,
            exhaust_diameter: 0.35,
        },
        lift: 0.105,
        ..Self::TWIN_CAM
    };
    /// Big valves under two switching cams, the mild profile lifting little (the wild
    /// one makes up for it).
    const K_R: Self = Self {
        lift: 0.095,
        ..Self::BIG_VALVES
    };
    /// Two cams over four valves, as on the 2-litre four.
    const TWIN_CAM: Self = Self {
        layout: ValveLayout::FOUR_VALVE,
        rocker_ratio: 1.0,
        cam_side_mass: 0.0,
        rollers: false,
        lift: 0.11,
    };
    /// One cam in the block, hydraulic roller lifters and long pushrods to 1.6:1 rockers
    /// over two big valves.
    const PUSHROD: Self = Self {
        layout: ValveLayout::PUSHROD,
        rocker_ratio: 1.6,
        cam_side_mass: 0.2,
        rollers: true,
        lift: 0.125,
    };
}

/// The hardware of a block: the 2-litre four's cylinder scaled to the bore (masses with
/// its cube, forces with its square; a valve also with its diameter), the parts around
/// the cylinders sized by the displacement, and springs and rods for the redline.
fn block(b: Block) -> EngineHardware {
    let reference = EngineHardware::default();
    let (bore, stroke) = (b.bore / 1000.0, b.stroke / 1000.0);
    let scale = bore / reference.bore;
    let head = b.head;
    let valve = head.layout.intake_diameter / reference.valvetrain.intake_diameter;
    let cam = CamProfile {
        intake_duration: b.duration,
        exhaust_duration: b.duration,
        lobe_separation: b.separation,
        lift: round_to(head.lift * bore, 1e-4),
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
        cams: std::iter::once(cam)
            .chain(b.high.map(|(duration, separation, lift)| CamProfile {
                intake_duration: duration,
                exhaust_duration: duration,
                lobe_separation: separation,
                lift: round_to(cam.lift * lift, 1e-4),
            }))
            .collect(),
        intake_phasing: b.phaser,
        exhaust_phasing: 0.0,
        intake_timing: 0.0,
        exhaust_timing: 0.0,
        valves: ValveTrain {
            seat_force: reference.valves.seat_force * scale * scale,
            rate: reference.valves.rate * scale,
            mass: round_to(reference.valves.mass * scale.powi(3) * valve, 1e-3),
            rocker_ratio: head.rocker_ratio,
            cam_side_mass: head.cam_side_mass,
            rollers: head.rollers,
        },
        valvetrain: head.layout,
        port_flow: b.flow,
        intake: reference.intake,
        turbo: None,
        turbos: 0,
        supercharger: None,
        exhaust: reference.exhaust,
        wideband: false,
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
        manifold: Manifold {
            primary_length: b.header / 1000.0,
            collector: Collector::Tuned,
            branches: 1,
        },
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

/// How a manifold is made, for how it looks: cast in iron, or bent of tubes.
#[derive(Clone, Copy)]
enum Made {
    Cast,
    Tubes,
}

/// A manifold: its id and name, primaries (mm) into `branches` collectors, or into as many
/// turbine inlets (`mounts`), cast or of tubes.
fn manifold(
    id: &str,
    name: &str,
    primary: f32,
    branches: u32,
    mounts: bool,
    made: Made,
) -> ManifoldDef {
    ManifoldDef {
        id: id.into(),
        name: name.into(),
        manifold: Manifold {
            primary_length: primary / 1000.0,
            collector: if mounts {
                Collector::Turbine
            } else {
                Collector::Tuned
            },
            branches,
        },
        mounts,
        cast: matches!(made, Made::Cast),
    }
}

/// The B series' manifolds: the cast 4-into-1 (its primaries the blocks' headers), short
/// and long-tube headers, and a turbo manifold (the turbo where the cast one's collector is).
fn b_manifolds() -> Vec<ManifoldDef> {
    use Made::*;
    vec![
        manifold(
            "b.manifold.stock",
            "Stock cast 4-into-1 manifold",
            700.0,
            1,
            false,
            Cast,
        ),
        manifold(
            "b.headers.short",
            "Short 4-into-1 headers",
            390.0,
            1,
            false,
            Tubes,
        ),
        manifold(
            "b.headers.long-tube",
            "Long-tube 4-2-1 headers",
            960.0,
            1,
            false,
            Tubes,
        ),
        manifold("b.manifold.turbo", "Turbo manifold", 700.0, 1, true, Cast),
    ]
}

/// The K series' cast manifold's primaries (mm), its fixed mild cams (duration, lobe
/// separation) and its plain head's flow.
const K_HEADER: f32 = 650.0;
const K_DURATION: f32 = 236.0;
const K_SEPARATION: f32 = 112.0;
const K_FLOW: f32 = 1.1;

/// The K series' manifolds, as the B series': the cast 4-into-1, short and long-tube
/// headers, and a turbo manifold.
fn k_manifolds() -> Vec<ManifoldDef> {
    use Made::*;
    vec![
        manifold(
            "k.manifold.stock",
            "Stock cast 4-into-1 manifold",
            K_HEADER,
            1,
            false,
            Cast,
        ),
        manifold(
            "k.headers.short",
            "Short 4-into-1 headers",
            390.0,
            1,
            false,
            Tubes,
        ),
        manifold(
            "k.headers.long-tube",
            "Long-tube 4-2-1 headers",
            960.0,
            1,
            false,
            Tubes,
        ),
        manifold(
            "k.manifold.turbo",
            "Turbo manifold",
            K_HEADER,
            1,
            true,
            Cast,
        ),
    ]
}

/// The R series' manifolds, a pair (one per bank) but for the 10-into-1, whose primaries
/// cross over the valley from the far bank to one big turbo.
fn r_manifolds() -> Vec<ManifoldDef> {
    use Made::*;
    vec![
        manifold(
            "r.manifolds.stock",
            "Stock cast manifolds (pair)",
            500.0,
            2,
            false,
            Cast,
        ),
        manifold(
            "r.headers.long-tube",
            "Long-tube headers (pair)",
            690.0,
            2,
            false,
            Tubes,
        ),
        manifold(
            "r.manifolds.turbo",
            "Turbo manifolds (pair)",
            500.0,
            2,
            true,
            Cast,
        ),
        manifold(
            "r.manifold.turbo-crossover",
            "10-into-1 turbo manifold (crossover)",
            900.0,
            1,
            true,
            Tubes,
        ),
    ]
}

/// The ECUs: a stock one for each factory version (locked on its factory tune, firing its
/// cylinders and switching or phasing its cams if it has them), each version's preset
/// fitting its own; and the programmable ones, for up to four cylinders with cam control
/// and for up to ten without.
pub fn ecus(families: &mut [Family]) -> Vec<EcuDef> {
    let mut ecus = Vec::new();
    for family in families.iter_mut() {
        for version in &mut family.versions {
            let mut hardware = version.block.clone();
            hardware.name = format!("{} {}", version.code, version.name);
            let model = EcuModel {
                cylinders: hardware.cylinders,
                switching: hardware.cams.len() > 1,
                phasers: hardware.intake_phasing > 0.0 || hardware.exhaust_phasing > 0.0,
                wideband: false,
                locked: true,
            };
            let maps = EngineMaps::for_hardware(&hardware);
            let tune = Ecu::calibrated(&hardware, &maps, PUMP_OCTANE, model);
            version.preset.ecu = ecus.len();
            ecus.push(EcuDef {
                id: format!("ecu.{}", version.code.to_lowercase()),
                name: format!("{} ECU", version.code),
                model,
                factory: Some(version.code.clone()),
                tune: Some(tune),
            });
        }
    }
    let programmable = |id: &str, name: &str, model| EcuDef {
        id: id.into(),
        name: name.into(),
        model,
        factory: None,
        tune: None,
    };
    ecus.push(programmable(
        "ecu.programmable-4",
        "Programmable ECU (4 cylinders)",
        EcuModel {
            cylinders: 4,
            ..EcuModel::PROGRAMMABLE
        },
    ));
    ecus.push(programmable(
        "ecu.programmable-10",
        "Programmable ECU (up to 10 cylinders)",
        EcuModel {
            cylinders: 10,
            switching: false,
            phasers: false,
            ..EcuModel::PROGRAMMABLE
        },
    ));
    ecus
}

/// Every version's factory head made one of its family's heads (`<code>.head`), and the
/// version's preset fitting it.
fn with_heads(mut family: Family) -> Family {
    family.heads = (family.versions.iter())
        .map(|version| {
            let id = format!("{}.head", version.code.to_lowercase());
            let name = format!("{} cylinder head", version.code);
            HeadDef::of(&id, &name, &version.block)
        })
        .collect();
    for (v, version) in family.versions.iter_mut().enumerate() {
        version.preset.head = v;
    }
    family
}

pub fn families() -> Vec<Family> {
    [
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
                    "B16",
                    "1.6 SOHC four",
                    || {
                        block(Block {
                            cylinders: 4,
                            bore: 78.0,
                            stroke: 83.6,
                            compression: 9.4,
                            duration: 236.0,
                            separation: 112.0,
                            high: None,
                            phaser: 0.0,
                            redline: 6300.0,
                            runner: 420.0,
                            header: 700.0,
                            inertia: 0.13,
                            flow: 0.95,
                            head: Head::SINGLE_CAM,
                        })
                    },
                    none,
                ),
                version(
                    "B20",
                    "2.0 SOHC four",
                    || {
                        block(Block {
                            cylinders: 4,
                            bore: 84.0,
                            stroke: 89.0,
                            compression: 9.2,
                            duration: 240.0,
                            separation: 112.0,
                            high: None,
                            phaser: 0.0,
                            redline: 6000.0,
                            runner: 440.0,
                            header: 700.0,
                            inertia: 0.15,
                            flow: 0.95,
                            head: Head::SINGLE_CAM,
                        })
                    },
                    none,
                ),
                version(
                    "B16R",
                    "1.6 DOHC four",
                    || {
                        // Short and oversquare, a mild cam for the street and a wild one
                        // from the middle up, big valves and ports that flow freely: it
                        // revs.
                        block(Block {
                            cylinders: 4,
                            bore: 81.0,
                            stroke: 77.4,
                            compression: 10.4,
                            duration: 232.0,
                            separation: 110.0,
                            high: Some((270.0, 106.0, 1.22)),
                            phaser: 0.0,
                            redline: 8200.0,
                            runner: 300.0,
                            header: 700.0,
                            inertia: 0.11,
                            flow: 1.35,
                            head: Head::BIG_VALVES,
                        })
                    },
                    none,
                ),
            ],
            manifolds: b_manifolds(),
            heads: Vec::new(),
            bellhousing: "B-series".into(),
        },
        Family {
            name: "R-series V10".into(),
            about: "Eight litres in a low, wide V10 with pushrods: torque everywhere".into(),
            layout: Layout::OddFireV10,
            length: 0.86,
            width: 0.78,
            height: 0.60,
            mass: 300.0,
            versions: vec![version(
                "R80",
                "8.0 V10",
                || {
                    block(Block {
                        cylinders: 10,
                        bore: 101.6,
                        stroke: 98.6,
                        compression: 9.1,
                        // A long duration on a wide lobe separation: the intake
                        // closes late (the torque comes early and stays flat, as on
                        // the big American V10s), with little overlap (it idles low).
                        duration: 276.0,
                        separation: 130.0,
                        high: None,
                        phaser: 0.0,
                        redline: 6000.0,
                        runner: 400.0,
                        header: 500.0,
                        inertia: 0.35,
                        flow: 0.95,
                        head: Head::PUSHROD,
                    })
                },
                none,
            )],
            manifolds: r_manifolds(),
            heads: Vec::new(),
            bellhousing: "R-series".into(),
        },
        Family {
            name: "K-series four".into(),
            about: "A modern twin-cam four, square or long-stroke: strong everywhere".into(),
            layout: Layout::Inline4,
            length: 0.62,
            width: 0.62,
            height: 0.68,
            mass: 115.0,
            versions: vec![
                version(
                    "K20",
                    "2.0 DOHC four",
                    || {
                        // Square, mild fixed cams and long runners: torque low down and
                        // a modest top end.
                        block(Block {
                            cylinders: 4,
                            bore: 86.0,
                            stroke: 86.0,
                            compression: 9.8,
                            duration: K_DURATION,
                            separation: K_SEPARATION,
                            high: None,
                            phaser: 0.0,
                            redline: 6900.0,
                            runner: 430.0,
                            header: K_HEADER,
                            inertia: 0.13,
                            flow: K_FLOW,
                            head: Head::TWIN_CAM,
                        })
                    },
                    none,
                ),
                version(
                    "K20R",
                    "2.0 DOHC four",
                    || {
                        // The same block with more compression, a head with big valves
                        // and ports that flow under a mild cam (phased on the intake)
                        // and a wild one with far more lift switched in from the middle
                        // up (about 5500 rpm), and long-tube headers: it revs. (In the
                        // model the wild cam pays only with long headers; its runners
                        // hold the top end to the table's power, and leave short ones
                        // to the swaps.)
                        block(Block {
                            cylinders: 4,
                            bore: 86.0,
                            stroke: 86.0,
                            compression: 11.5,
                            duration: 224.0,
                            separation: 112.0,
                            high: Some((236.0, 110.0, 1.3)),
                            phaser: 50.0,
                            redline: 8600.0,
                            runner: 440.0,
                            header: 960.0,
                            inertia: 0.12,
                            flow: 1.4,
                            head: Head::K_R,
                        })
                    },
                    // Tubular 4-2-1 headers from the factory.
                    |preset| preset.manifold = 2,
                ),
                version(
                    "K24",
                    "2.4 DOHC four",
                    || {
                        // Long-stroke: the K20's head on a taller block, rods made for
                        // its lower revs.
                        block(Block {
                            cylinders: 4,
                            bore: 87.0,
                            stroke: 99.0,
                            compression: 9.7,
                            duration: K_DURATION,
                            separation: K_SEPARATION,
                            high: None,
                            phaser: 0.0,
                            redline: 6500.0,
                            runner: 450.0,
                            header: K_HEADER,
                            inertia: 0.15,
                            flow: K_FLOW,
                            head: Head::TWIN_CAM,
                        })
                    },
                    none,
                ),
            ],
            manifolds: k_manifolds(),
            heads: Vec::new(),
            bellhousing: "K-series".into(),
        },
    ]
    .into_iter()
    .map(with_heads)
    .collect()
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
    fn the_factory_manifolds_are_the_blocks_headers() {
        for family in &builtin().families {
            for version in &family.versions {
                let manifold = &family.manifolds[version.preset.manifold];
                assert!(!manifold.mounts, "{}", version.code);
                assert_eq!(
                    manifold.manifold.primary_length, version.block.exhaust.manifold.primary_length,
                    "{}",
                    version.code
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
