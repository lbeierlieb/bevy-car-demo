//! The built-in catalogue's chassis, gearboxes, clutches, suspension kits and paints.

use super::{Clutch, Paint, Suspension};
use crate::body::{Body, Detail, Tone};
use crate::chassis::{Chassis, EngineBay, Gearbox, GearboxKind, Orientation, RollBars};
use crate::physics::drivetrain::Drive;
use crate::physics::fuel::Tank;
use crate::physics::tire::{SPORT, STREET, TOURING};
use crate::wheels::{Rim, TireSize, Wheel, WheelRoom};

pub fn clutches() -> Vec<Clutch> {
    let clutch = |name: &str, torque| Clutch {
        name: name.into(),
        torque,
    };
    vec![
        clutch("Stock", 1.0),
        clutch("Sport", 1.6),
        clutch("Race", 2.5),
    ]
}

pub fn suspensions() -> Vec<Suspension> {
    let kit = |name: &str, ride, damping, camber, reach| Suspension {
        name: name.into(),
        ride,
        damping,
        camber,
        reach,
    };
    vec![
        kit("Stock", 1.0, 1.0, 0.0, 0.5),
        kit("Sport", 1.2, 1.15, 0.8, 1.0),
        kit("Track", 1.45, 1.3, 1.8, 2.0),
    ]
}

/// The paint shops' first, then the dull ones cars leave the factory in (one per
/// chassis, so there's a reason to go to a paint shop).
pub fn paints() -> Vec<Paint> {
    let shop = [
        ("Signal red", [0.80, 0.10, 0.08]),
        ("Racing blue", [0.10, 0.28, 0.75]),
        ("Sunflower", [0.95, 0.72, 0.08]),
        ("Glacier white", [0.90, 0.90, 0.88]),
        ("Graphite", [0.20, 0.21, 0.23]),
        ("Silver", [0.62, 0.64, 0.67]),
        ("Forest green", [0.08, 0.32, 0.18]),
        ("Tangerine", [0.92, 0.42, 0.08]),
        ("Black", [0.04, 0.04, 0.05]),
        ("Sky blue", [0.40, 0.66, 0.86]),
    ];
    let factory = [
        ("Pale beige", [0.76, 0.71, 0.60]),
        ("Putty", [0.62, 0.60, 0.54]),
        ("Faded olive", [0.42, 0.43, 0.32]),
        ("Primer grey", [0.47, 0.48, 0.47]),
        ("Mushroom", [0.55, 0.50, 0.45]),
        ("Off-white", [0.80, 0.79, 0.74]),
        ("Mud brown", [0.36, 0.29, 0.22]),
        ("Slate", [0.33, 0.36, 0.39]),
        ("Dull navy", [0.18, 0.21, 0.30]),
        ("Dove grey", [0.66, 0.66, 0.64]),
        ("Khaki", [0.58, 0.53, 0.40]),
    ];
    let paint = |shop: bool| {
        move |(name, color): (&str, [f32; 3])| Paint {
            name: name.into(),
            color,
            shop,
        }
    };
    (shop.into_iter().map(paint(true)))
        .chain(factory.into_iter().map(paint(false)))
        .collect()
}

/// Anti-roll bars of `sizes` (mm), the factory's at `factory` in them, and a 20 mm one
/// giving the axle `rate` (N·m per degree of roll).
fn bars(sizes: &[u8], factory: usize, rate: f32) -> RollBars {
    RollBars {
        sizes: sizes.to_vec(),
        factory,
        rate,
    }
}

/// The codes of engines.
fn codes(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|code| code.to_string()).collect()
}

pub fn gearboxes() -> Vec<Gearbox> {
    vec![
        Gearbox {
            name: "5-speed (transverse)".into(),
            kind: GearboxKind::Transverse,
            gears: vec![3.55, 1.95, 1.31, 0.97, 0.78],
            reverse: 3.4,
            mass: 35.0,
            clutch_torque: 200.0,
            shift_time: 0.28,
            engage_time: 0.16,
        },
        Gearbox {
            name: "6-speed close-ratio (transverse)".into(),
            kind: GearboxKind::Transverse,
            gears: vec![3.3, 2.05, 1.48, 1.14, 0.92, 0.76],
            reverse: 3.3,
            mass: 42.0,
            clutch_torque: 380.0,
            shift_time: 0.22,
            engage_time: 0.15,
        },
        Gearbox {
            name: "5-speed".into(),
            kind: GearboxKind::Inline,
            gears: vec![3.4, 2.0, 1.4, 1.0, 0.81],
            reverse: 3.4,
            mass: 38.0,
            clutch_torque: 270.0,
            shift_time: 0.22,
            engage_time: 0.15,
        },
        Gearbox {
            name: "6-speed".into(),
            kind: GearboxKind::Inline,
            gears: vec![3.3, 2.1, 1.5, 1.17, 0.95, 0.8],
            reverse: 3.3,
            mass: 48.0,
            clutch_torque: 600.0,
            shift_time: 0.25,
            engage_time: 0.15,
        },
        Gearbox {
            name: "6-speed dog box".into(),
            kind: GearboxKind::Inline,
            gears: vec![3.1, 2.2, 1.7, 1.35, 1.1, 0.92],
            reverse: 3.1,
            mass: 52.0,
            clutch_torque: 520.0,
            shift_time: 0.1,
            engage_time: 0.1,
        },
        Gearbox {
            name: "6-speed heavy duty".into(),
            kind: GearboxKind::Inline,
            gears: vec![2.66, 1.78, 1.3, 1.0, 0.74, 0.5],
            reverse: 2.9,
            mass: 60.0,
            clutch_torque: 760.0,
            shift_time: 0.28,
            engage_time: 0.16,
        },
        Gearbox {
            name: "6-speed transaxle".into(),
            kind: GearboxKind::Transaxle,
            gears: vec![3.2, 2.0, 1.45, 1.12, 0.92, 0.78],
            reverse: 3.2,
            mass: 55.0,
            clutch_torque: 760.0,
            shift_time: 0.18,
            engage_time: 0.12,
        },
        Gearbox {
            name: "7-speed dual-clutch transaxle".into(),
            kind: GearboxKind::Transaxle,
            gears: vec![3.4, 2.3, 1.75, 1.4, 1.15, 0.97, 0.82],
            reverse: 3.2,
            mass: 75.0,
            clutch_torque: 820.0,
            shift_time: 0.06,
            engage_time: 0.08,
        },
    ]
}

const fn wheel(diameter: u8, rim_width: u8, width: u16, aspect: u8, compound: usize) -> Wheel {
    Wheel {
        rim: Rim {
            diameter,
            width: rim_width,
        },
        tire: TireSize {
            width,
            aspect,
            rim: diameter,
        },
        compound,
    }
}

const fn detail(at: [f32; 3], size: [f32; 3], tone: Tone) -> Detail {
    Detail { at, size, tone }
}

/// The hot hatch (with its base engine, the B16), which the game starts with.
pub const DEFAULT_CHASSIS: usize = 1;

pub fn chassis() -> Vec<Chassis> {
    vec![
        Chassis {
            name: "Minim".into(),
            class: "City car".into(),
            about: "Small, light and upright. Three cylinders are plenty; a turbo makes it \
                cheeky"
                .into(),
            mass: 610.0,
            mass_at: 1.2,
            cg_height: 0.54,
            wheelbase: 2.35,
            track: [1.42, 1.41],
            wheels: [
                wheel(14, 55, 165, 65, TOURING),
                wheel(14, 55, 165, 65, TOURING),
            ],
            room: [
                WheelRoom {
                    diameter: 0.62,
                    width: 195,
                    min_rim: 13,
                },
                WheelRoom {
                    diameter: 0.62,
                    width: 195,
                    min_rim: 13,
                },
            ],
            hub: [17.0, 15.0],
            ride: [1.3, 1.45],
            damping: 0.3,
            bars: [
                bars(&[16, 18, 20, 22, 24], 2, 240.0),
                bars(&[14, 16, 18, 20], 1, 590.0),
            ],
            camber: [-0.8, -1.2],
            pressure: [2.3, 2.1],
            max_steer: 0.56,
            brakes: 0.78,
            drag_area: 0.63,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 35.0,
                at: 1.95,
                height: 0.3,
            },
            bay: EngineBay {
                place: "front, transverse".into(),
                orientation: Orientation::Transverse,
                length: 0.62,
                width: 1.0,
                height: 0.66,
                engine_at: -0.12,
                gearbox_at: -0.12,
                header_room: 1.1,
            },
            drive: Drive::Front,
            limited_slip: false,
            gearbox: 0,
            final_drive: 4.1,
            engines: codes(&["A10", "A10T", "B12"]),
            paint: 10,
            body: Body {
                length: 3.55,
                width: 1.63,
                front_overhang: 0.68,
                floor: 0.17,
                deck: vec![
                    [0.0, 0.54],
                    [0.06, 0.68],
                    [0.55, 0.82],
                    [0.85, 0.92],
                    [3.47, 0.94],
                    [3.55, 0.84],
                ],
                cabin: vec![
                    [0.8, 0.9, 1.0],
                    [1.4, 1.42, 1.0],
                    [1.8, 1.48, 1.0],
                    [3.15, 1.46, 1.0],
                    [3.5, 0.95, 1.0],
                ],
                nose: 0.84,
                tail: 0.9,
                tumblehome: 0.84,
                open_wheels: false,
                openings: vec![],
                details: vec![],
            },
            eye: [1.75, 1.2, 0.34],
            insulation: 0.3,
            traffic: 4.0,
        },
        Chassis {
            name: "Vireo".into(),
            class: "Hot hatch".into(),
            about: "A practical hatchback that the factory couldn't leave alone: front-wheel \
                drive with a limited-slip differential and a choice of eager fours"
                .into(),
            mass: 890.0,
            mass_at: 1.32,
            cg_height: 0.51,
            wheelbase: 2.6,
            track: [1.53, 1.51],
            wheels: [
                wheel(17, 70, 205, 45, STREET),
                wheel(17, 70, 205, 45, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.67,
                    width: 235,
                    min_rim: 15,
                },
                WheelRoom {
                    diameter: 0.67,
                    width: 235,
                    min_rim: 15,
                },
            ],
            hub: [18.0, 15.0],
            ride: [1.6, 1.85],
            damping: 0.34,
            bars: [
                bars(&[20, 22, 24, 26], 1, 270.0),
                bars(&[18, 20, 22, 24, 26], 2, 340.0),
            ],
            camber: [-1.5, -1.5],
            pressure: [2.4, 2.1],
            max_steer: 0.52,
            brakes: 0.8,
            drag_area: 0.68,
            downforce_area: [0.0, 0.03],
            tank: Tank {
                litres: 50.0,
                at: 2.2,
                height: 0.3,
            },
            bay: EngineBay {
                place: "front, transverse".into(),
                orientation: Orientation::Transverse,
                length: 0.74,
                width: 1.0,
                height: 0.7,
                engine_at: -0.12,
                gearbox_at: -0.12,
                header_room: 1.15,
            },
            drive: Drive::Front,
            limited_slip: true,
            gearbox: 1,
            final_drive: 4.4,
            engines: codes(&["B16", "R20", "W20T"]),
            paint: 11,
            body: Body {
                length: 4.15,
                width: 1.78,
                front_overhang: 0.85,
                floor: 0.15,
                deck: vec![
                    [0.0, 0.5],
                    [0.06, 0.64],
                    [0.8, 0.78],
                    [1.15, 0.88],
                    [4.07, 0.92],
                    [4.15, 0.82],
                ],
                cabin: vec![
                    [1.1, 0.86, 1.0],
                    [1.8, 1.38, 1.0],
                    [2.3, 1.44, 1.0],
                    [3.6, 1.4, 1.0],
                    [4.08, 0.93, 1.0],
                ],
                nose: 0.82,
                tail: 0.9,
                tumblehome: 0.82,
                open_wheels: false,
                openings: vec![],
                details: vec![
                    // A roof spoiler over the hatch.
                    detail([3.72, 1.42, 0.0], [0.3, 0.03, 1.1], Tone::Dark),
                ],
            },
            eye: [2.15, 1.17, 0.36],
            insulation: 0.45,
            traffic: 3.0,
        },
        Chassis {
            name: "Linnet".into(),
            class: "Roadster".into(),
            about: "Two seats, no roof, rear-wheel drive and not much weight: it doesn't need \
                power to be fun, though there's room for a V8"
                .into(),
            mass: 740.0,
            mass_at: 1.27,
            cg_height: 0.44,
            wheelbase: 2.31,
            track: [1.43, 1.45],
            wheels: [
                wheel(16, 65, 195, 50, STREET),
                wheel(16, 65, 195, 50, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.65,
                    width: 225,
                    min_rim: 14,
                },
                WheelRoom {
                    diameter: 0.65,
                    width: 225,
                    min_rim: 14,
                },
            ],
            hub: [16.0, 18.0],
            ride: [1.5, 1.65],
            damping: 0.32,
            bars: [
                bars(&[18, 20, 22, 24], 1, 430.0),
                bars(&[14, 16, 18, 20], 1, 700.0),
            ],
            camber: [-1.2, -1.5],
            pressure: [2.2, 2.1],
            max_steer: 0.54,
            brakes: 0.8,
            drag_area: 0.6,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 45.0,
                at: 1.9,
                height: 0.4,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.7,
                width: 0.76,
                height: 0.7,
                engine_at: 0.12,
                gearbox_at: 0.8,
                header_room: 1.0,
            },
            drive: Drive::Rear,
            limited_slip: false,
            gearbox: 2,
            final_drive: 4.1,
            engines: codes(&["B16", "B16S"]),
            paint: 12,
            body: Body {
                length: 3.95,
                width: 1.7,
                front_overhang: 0.8,
                floor: 0.14,
                deck: vec![
                    [0.0, 0.46],
                    [0.06, 0.56],
                    [0.9, 0.68],
                    [1.45, 0.8],
                    [2.6, 0.82],
                    [3.2, 0.84],
                    [3.87, 0.8],
                    [3.95, 0.66],
                ],
                cabin: vec![[1.42, 0.79, 1.0], [1.85, 1.16, 1.0]],
                nose: 0.8,
                tail: 0.84,
                tumblehome: 0.86,
                open_wheels: false,
                openings: vec![[1.6, 2.72, 0.56]],
                details: vec![
                    // Hoops behind the seats.
                    detail([2.74, 0.98, 0.32], [0.07, 0.3, 0.26], Tone::Dark),
                ],
            },
            eye: [2.3, 1.03, 0.33],
            insulation: 0.2,
            traffic: 0.7,
        },
        Chassis {
            name: "Tarn".into(),
            class: "Sports coupé".into(),
            about: "The all-rounder: engine in front, drive at the back, a long bay that takes \
                a straight six or a V8"
                .into(),
            mass: 930.0,
            mass_at: 1.42,
            cg_height: 0.46,
            wheelbase: 2.57,
            track: [1.52, 1.53],
            wheels: [
                wheel(17, 75, 225, 45, STREET),
                wheel(17, 80, 235, 45, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.68,
                    width: 255,
                    min_rim: 16,
                },
                WheelRoom {
                    diameter: 0.69,
                    width: 265,
                    min_rim: 16,
                },
            ],
            hub: [19.0, 20.0],
            ride: [1.65, 1.8],
            damping: 0.33,
            bars: [
                bars(&[22, 24, 26, 28, 30], 1, 260.0),
                bars(&[16, 18, 20, 22], 1, 430.0),
            ],
            camber: [-1.2, -1.4],
            pressure: [2.2, 2.1],
            max_steer: 0.5,
            brakes: 0.8,
            drag_area: 0.62,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 55.0,
                at: 2.35,
                height: 0.35,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.88,
                width: 0.76,
                height: 0.7,
                engine_at: 0.08,
                gearbox_at: 0.85,
                header_room: 1.15,
            },
            drive: Drive::Rear,
            limited_slip: false,
            gearbox: 3,
            final_drive: 3.7,
            engines: codes(&["W20", "W20T", "S30"]),
            paint: 13,
            body: Body {
                length: 4.3,
                width: 1.8,
                front_overhang: 0.85,
                floor: 0.14,
                deck: vec![
                    [0.0, 0.48],
                    [0.06, 0.6],
                    [0.95, 0.74],
                    [1.45, 0.84],
                    [3.35, 0.9],
                    [4.22, 0.9],
                    [4.3, 0.76],
                ],
                cabin: vec![
                    [1.4, 0.83, 1.0],
                    [2.1, 1.27, 1.0],
                    [2.6, 1.31, 1.0],
                    [3.05, 1.27, 1.0],
                    [3.95, 0.92, 1.0],
                ],
                nose: 0.8,
                tail: 0.86,
                tumblehome: 0.8,
                open_wheels: false,
                openings: vec![],
                details: vec![],
            },
            eye: [2.45, 1.1, 0.36],
            insulation: 0.55,
            traffic: 3.0,
        },
        Chassis {
            name: "Halden".into(),
            class: "Saloon".into(),
            about: "Four doors and a big bay: comfortable with a turbo four, quick with a six, \
                and the only front-engined car that swallows a V12"
                .into(),
            mass: 1200.0,
            mass_at: 1.55,
            cg_height: 0.52,
            wheelbase: 2.85,
            track: [1.58, 1.6],
            wheels: [
                wheel(17, 75, 225, 50, STREET),
                wheel(17, 75, 225, 50, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.71,
                    width: 255,
                    min_rim: 15,
                },
                WheelRoom {
                    diameter: 0.71,
                    width: 255,
                    min_rim: 15,
                },
            ],
            hub: [22.0, 24.0],
            ride: [1.35, 1.5],
            damping: 0.32,
            bars: [
                bars(&[22, 24, 26, 28], 1, 310.0),
                bars(&[16, 18, 20, 22], 1, 500.0),
            ],
            camber: [-0.8, -1.2],
            pressure: [2.5, 2.4],
            max_steer: 0.52,
            brakes: 0.8,
            drag_area: 0.64,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 65.0,
                at: 2.55,
                height: 0.35,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.96,
                width: 0.8,
                height: 0.74,
                engine_at: 0.05,
                gearbox_at: 0.9,
                header_room: 1.3,
            },
            drive: Drive::Rear,
            limited_slip: false,
            gearbox: 3,
            final_drive: 3.4,
            engines: codes(&["W20T", "S30", "S30T", "M50"]),
            paint: 14,
            body: Body {
                length: 4.8,
                width: 1.87,
                front_overhang: 0.88,
                floor: 0.15,
                deck: vec![
                    [0.0, 0.52],
                    [0.06, 0.66],
                    [1.0, 0.82],
                    [1.5, 0.92],
                    [3.9, 0.96],
                    [4.72, 0.96],
                    [4.8, 0.8],
                ],
                cabin: vec![
                    [1.45, 0.91, 1.0],
                    [2.1, 1.4, 1.0],
                    [2.6, 1.45, 1.0],
                    [3.45, 1.42, 1.0],
                    [4.05, 0.97, 1.0],
                ],
                nose: 0.82,
                tail: 0.86,
                tumblehome: 0.8,
                open_wheels: false,
                openings: vec![],
                details: vec![],
            },
            eye: [2.5, 1.2, 0.38],
            insulation: 0.8,
            traffic: 4.0,
        },
        Chassis {
            name: "Skarn".into(),
            class: "Rally car".into(),
            about: "All-wheel drive, limited-slip differentials everywhere and a turbocharged \
                boxer ahead of the front axle: made for loose surfaces"
                .into(),
            mass: 980.0,
            mass_at: 1.4,
            cg_height: 0.49,
            wheelbase: 2.55,
            track: [1.53, 1.53],
            wheels: [
                wheel(17, 75, 215, 50, STREET),
                wheel(17, 75, 215, 50, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.7,
                    width: 245,
                    min_rim: 15,
                },
                WheelRoom {
                    diameter: 0.7,
                    width: 245,
                    min_rim: 15,
                },
            ],
            hub: [21.0, 21.0],
            ride: [1.7, 1.8],
            damping: 0.36,
            bars: [
                bars(&[20, 22, 24, 26, 28], 2, 210.0),
                bars(&[18, 20, 22, 24], 1, 320.0),
            ],
            camber: [-1.5, -1.5],
            pressure: [2.4, 2.2],
            max_steer: 0.52,
            brakes: 0.8,
            drag_area: 0.72,
            downforce_area: [0.03, 0.12],
            tank: Tank {
                litres: 60.0,
                at: 2.4,
                height: 0.35,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.7,
                width: 0.84,
                height: 0.68,
                engine_at: -0.25,
                gearbox_at: 0.4,
                header_room: 1.0,
            },
            drive: Drive::All { front: 0.45 },
            limited_slip: true,
            gearbox: 4,
            final_drive: 4.1,
            engines: codes(&["X20T", "X25T", "W20T"]),
            paint: 15,
            body: Body {
                length: 4.4,
                width: 1.8,
                front_overhang: 0.9,
                floor: 0.17,
                deck: vec![
                    [0.0, 0.52],
                    [0.06, 0.66],
                    [0.95, 0.8],
                    [1.4, 0.9],
                    [3.6, 0.94],
                    [4.32, 0.96],
                    [4.4, 0.8],
                ],
                cabin: vec![
                    [1.35, 0.89, 1.0],
                    [2.0, 1.38, 1.0],
                    [2.5, 1.42, 1.0],
                    [3.25, 1.4, 1.0],
                    [3.85, 0.95, 1.0],
                ],
                nose: 0.82,
                tail: 0.88,
                tumblehome: 0.8,
                open_wheels: false,
                openings: vec![],
                details: vec![
                    // Bonnet scoop, roof vent, and a wing on two posts.
                    detail([0.95, 0.85, 0.0], [0.3, 0.07, 0.45], Tone::Dark),
                    detail([2.3, 1.46, 0.0], [0.3, 0.05, 0.3], Tone::Dark),
                    detail([4.2, 1.1, 0.5], [0.06, 0.26, 0.04], Tone::Dark),
                    detail([4.22, 1.24, 0.0], [0.26, 0.03, 1.6], Tone::Paint),
                    // Mud flaps.
                    detail([1.3, 0.3, 0.83], [0.02, 0.26, 0.2], Tone::Dark),
                    detail([3.85, 0.3, 0.83], [0.02, 0.26, 0.2], Tone::Dark),
                ],
            },
            eye: [2.35, 1.17, 0.36],
            insulation: 0.1,
            traffic: 0.8,
        },
        Chassis {
            name: "Harrow".into(),
            class: "Muscle car".into(),
            about: "A long bonnet with a V8 under it, a heavy-duty gearbox and wide rear tires \
                that still aren't wide enough"
                .into(),
            mass: 1210.0,
            mass_at: 1.55,
            cg_height: 0.5,
            wheelbase: 2.75,
            track: [1.6, 1.62],
            wheels: [
                wheel(18, 85, 245, 45, STREET),
                wheel(18, 95, 275, 40, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.73,
                    width: 275,
                    min_rim: 17,
                },
                WheelRoom {
                    diameter: 0.73,
                    width: 305,
                    min_rim: 17,
                },
            ],
            hub: [22.0, 25.0],
            ride: [1.4, 1.5],
            damping: 0.3,
            bars: [
                bars(&[26, 28, 30, 32], 1, 170.0),
                bars(&[18, 20, 22, 24], 1, 280.0),
            ],
            camber: [-0.8, -1.0],
            pressure: [2.4, 2.3],
            max_steer: 0.5,
            brakes: 0.78,
            drag_area: 0.74,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 75.0,
                at: 3.1,
                height: 0.4,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.92,
                width: 0.86,
                height: 0.76,
                engine_at: 0.1,
                gearbox_at: 0.95,
                header_room: 1.2,
            },
            drive: Drive::Rear,
            limited_slip: true,
            gearbox: 5,
            final_drive: 3.5,
            engines: codes(&["M50", "M62", "V35"]),
            paint: 16,
            body: Body {
                length: 4.8,
                width: 1.93,
                front_overhang: 0.9,
                floor: 0.15,
                deck: vec![
                    [0.0, 0.6],
                    [0.05, 0.74],
                    [1.2, 0.84],
                    [1.75, 0.92],
                    [3.8, 0.98],
                    [4.72, 0.98],
                    [4.8, 0.82],
                ],
                cabin: vec![
                    [1.7, 0.91, 1.0],
                    [2.35, 1.33, 1.0],
                    [2.8, 1.37, 1.0],
                    [3.35, 1.34, 1.0],
                    [4.2, 0.99, 1.0],
                ],
                nose: 0.88,
                tail: 0.9,
                tumblehome: 0.78,
                open_wheels: false,
                openings: vec![],
                details: vec![
                    // A bulge down the bonnet, and a lip on the boot.
                    detail([0.95, 0.83, 0.0], [1.3, 0.07, 0.5], Tone::Paint),
                    detail([4.72, 1.0, 0.0], [0.12, 0.05, 1.5], Tone::Paint),
                ],
            },
            eye: [2.75, 1.15, 0.38],
            insulation: 0.45,
            traffic: 1.0,
        },
        Chassis {
            name: "Calder".into(),
            class: "Mid-engined sports car".into(),
            about: "The engine sits behind the seats, where its weight helps the car turn and \
                put its power down"
                .into(),
            mass: 870.0,
            mass_at: 1.2,
            cg_height: 0.42,
            wheelbase: 2.45,
            track: [1.54, 1.56],
            wheels: [wheel(18, 80, 225, 40, SPORT), wheel(18, 95, 265, 40, SPORT)],
            room: [
                WheelRoom {
                    diameter: 0.69,
                    width: 255,
                    min_rim: 17,
                },
                WheelRoom {
                    diameter: 0.72,
                    width: 295,
                    min_rim: 17,
                },
            ],
            hub: [18.0, 20.0],
            ride: [1.9, 2.1],
            damping: 0.36,
            bars: [
                bars(&[22, 24, 26, 28], 1, 220.0),
                bars(&[18, 20, 22, 24], 1, 230.0),
            ],
            camber: [-1.5, -1.8],
            pressure: [2.0, 2.0],
            max_steer: 0.5,
            brakes: 0.84,
            drag_area: 0.6,
            downforce_area: [0.04, 0.1],
            tank: Tank {
                litres: 50.0,
                at: 0.1,
                height: 0.35,
            },
            bay: EngineBay {
                place: "middle".into(),
                orientation: Orientation::Longitudinal,
                length: 0.72,
                width: 0.78,
                height: 0.68,
                engine_at: 1.85,
                gearbox_at: 2.4,
                header_room: 1.1,
            },
            drive: Drive::Rear,
            limited_slip: true,
            gearbox: 6,
            final_drive: 3.9,
            engines: codes(&["V35", "F40"]),
            paint: 17,
            body: Body {
                length: 4.1,
                width: 1.85,
                front_overhang: 0.95,
                floor: 0.12,
                deck: vec![
                    [0.0, 0.4],
                    [0.06, 0.5],
                    [0.95, 0.64],
                    [1.35, 0.76],
                    [2.9, 0.84],
                    [3.5, 0.9],
                    [4.02, 0.88],
                    [4.1, 0.72],
                ],
                cabin: vec![
                    [1.25, 0.74, 1.0],
                    [1.95, 1.17, 1.0],
                    [2.35, 1.2, 1.0],
                    [2.7, 1.16, 1.0],
                    [3.4, 0.9, 1.0],
                ],
                nose: 0.76,
                tail: 0.9,
                tumblehome: 0.78,
                open_wheels: false,
                openings: vec![],
                details: vec![
                    // Intakes ahead of the rear wheels.
                    detail([2.75, 0.56, 0.915], [0.4, 0.22, 0.03], Tone::Dark),
                    detail([4.0, 0.93, 0.0], [0.16, 0.03, 1.5], Tone::Paint),
                ],
            },
            eye: [2.25, 1.02, 0.35],
            insulation: 0.35,
            traffic: 0.4,
        },
        Chassis {
            name: "Rheon".into(),
            class: "Rear-engined sports car".into(),
            about: "A flat six hung out behind the rear axle: traction for days, and a tail \
                that wants to lead when you lift. Only flat engines fit under its deck"
                .into(),
            mass: 970.0,
            mass_at: 1.2,
            cg_height: 0.45,
            wheelbase: 2.4,
            track: [1.5, 1.54],
            wheels: [
                wheel(18, 80, 225, 40, SPORT),
                wheel(18, 100, 285, 35, SPORT),
            ],
            room: [
                WheelRoom {
                    diameter: 0.69,
                    width: 255,
                    min_rim: 17,
                },
                WheelRoom {
                    diameter: 0.71,
                    width: 315,
                    min_rim: 17,
                },
            ],
            hub: [18.0, 21.0],
            ride: [1.8, 2.0],
            damping: 0.36,
            bars: [
                bars(&[20, 22, 24, 26], 1, 310.0),
                bars(&[16, 18, 20, 22], 1, 350.0),
            ],
            camber: [-1.0, -1.8],
            pressure: [2.0, 2.2],
            max_steer: 0.5,
            brakes: 0.84,
            drag_area: 0.6,
            downforce_area: [0.0, 0.08],
            tank: Tank {
                litres: 65.0,
                at: -0.1,
                height: 0.35,
            },
            bay: EngineBay {
                place: "rear".into(),
                orientation: Orientation::Longitudinal,
                length: 0.66,
                width: 0.86,
                height: 0.58,
                engine_at: 2.75,
                gearbox_at: 2.2,
                header_room: 1.25,
            },
            drive: Drive::Rear,
            limited_slip: true,
            gearbox: 6,
            final_drive: 3.6,
            engines: codes(&["H30", "H36T"]),
            paint: 18,
            body: Body {
                length: 4.35,
                width: 1.84,
                front_overhang: 0.95,
                floor: 0.13,
                deck: vec![
                    [0.0, 0.46],
                    [0.06, 0.58],
                    [0.95, 0.7],
                    [1.4, 0.82],
                    [3.2, 0.88],
                    [4.0, 0.8],
                    [4.27, 0.7],
                    [4.35, 0.58],
                ],
                cabin: vec![
                    [1.35, 0.81, 1.0],
                    [2.0, 1.25, 1.0],
                    [2.45, 1.29, 1.0],
                    [2.95, 1.24, 1.0],
                    [3.95, 0.81, 1.0],
                ],
                nose: 0.78,
                tail: 0.86,
                tumblehome: 0.76,
                open_wheels: false,
                openings: vec![],
                details: vec![
                    // The engine cover's grille.
                    detail([4.08, 0.79, 0.0], [0.22, 0.03, 0.9], Tone::Dark),
                ],
            },
            eye: [2.4, 1.08, 0.35],
            insulation: 0.4,
            traffic: 0.8,
        },
        Chassis {
            name: "Vespera".into(),
            class: "Supercar".into(),
            about: "Low, wide and loud, with ten or twelve cylinders amidships, all-wheel drive \
                and wings that push it into the road"
                .into(),
            mass: 1050.0,
            mass_at: 1.3,
            cg_height: 0.4,
            wheelbase: 2.65,
            track: [1.66, 1.62],
            wheels: [
                wheel(19, 90, 255, 35, SPORT),
                wheel(19, 110, 315, 35, SPORT),
            ],
            room: [
                WheelRoom {
                    diameter: 0.71,
                    width: 285,
                    min_rim: 19,
                },
                WheelRoom {
                    diameter: 0.75,
                    width: 345,
                    min_rim: 19,
                },
            ],
            hub: [18.0, 22.0],
            ride: [2.2, 2.4],
            damping: 0.4,
            bars: [
                bars(&[26, 28, 30, 32], 1, 140.0),
                bars(&[20, 22, 24, 26], 1, 120.0),
            ],
            camber: [-1.8, -2.0],
            pressure: [2.1, 2.1],
            max_steer: 0.46,
            brakes: 0.82,
            drag_area: 0.68,
            downforce_area: [0.22, 0.42],
            tank: Tank {
                litres: 90.0,
                at: 1.3,
                height: 0.4,
            },
            bay: EngineBay {
                place: "middle".into(),
                orientation: Orientation::Longitudinal,
                length: 0.98,
                width: 0.8,
                height: 0.7,
                engine_at: 1.9,
                gearbox_at: 2.6,
                header_room: 1.15,
            },
            drive: Drive::All { front: 0.3 },
            limited_slip: true,
            gearbox: 7,
            final_drive: 4.0,
            engines: codes(&["D52", "L65"]),
            paint: 19,
            body: Body {
                length: 4.55,
                width: 2.0,
                front_overhang: 1.05,
                floor: 0.11,
                deck: vec![
                    [0.0, 0.34],
                    [0.06, 0.42],
                    [1.05, 0.58],
                    [1.45, 0.7],
                    [3.0, 0.8],
                    [3.8, 0.88],
                    [4.47, 0.88],
                    [4.55, 0.7],
                ],
                cabin: vec![
                    [1.3, 0.67, 1.0],
                    [2.15, 1.11, 1.0],
                    [2.5, 1.14, 1.0],
                    [2.85, 1.1, 1.0],
                    [3.75, 0.88, 1.0],
                ],
                nose: 0.74,
                tail: 0.92,
                tumblehome: 0.74,
                open_wheels: false,
                openings: vec![],
                details: vec![
                    // Side intakes, a splitter, and a wing on two posts.
                    detail([3.0, 0.5, 0.99], [0.5, 0.26, 0.03], Tone::Dark),
                    detail([0.1, 0.14, 0.0], [0.3, 0.03, 1.5], Tone::Dark),
                    detail([4.35, 0.98, 0.6], [0.08, 0.2, 0.04], Tone::Dark),
                    detail([4.37, 1.08, 0.0], [0.3, 0.03, 1.8], Tone::Paint),
                ],
            },
            eye: [2.4, 0.97, 0.38],
            insulation: 0.5,
            traffic: 0.2,
        },
        Chassis {
            name: "Wisp".into(),
            class: "Track-day special".into(),
            about: "A tub, four open wheels and nothing else: half the weight of anything \
                else, so a small four is all it takes"
                .into(),
            mass: 330.0,
            mass_at: 1.25,
            cg_height: 0.36,
            wheelbase: 2.25,
            track: [1.42, 1.4],
            wheels: [
                wheel(15, 70, 195, 45, STREET),
                wheel(15, 75, 205, 45, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.62,
                    width: 245,
                    min_rim: 14,
                },
                WheelRoom {
                    diameter: 0.63,
                    width: 265,
                    min_rim: 14,
                },
            ],
            hub: [9.0, 10.0],
            ride: [2.0, 2.2],
            damping: 0.38,
            bars: [
                bars(&[16, 18, 20, 22, 24], 1, 220.0),
                bars(&[12, 14, 16, 18, 20], 1, 200.0),
            ],
            camber: [-1.8, -1.8],
            pressure: [1.7, 1.6],
            max_steer: 0.5,
            brakes: 0.84,
            drag_area: 0.66,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 40.0,
                at: 1.55,
                height: 0.3,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.68,
                width: 0.62,
                height: 0.66,
                engine_at: 0.25,
                gearbox_at: 0.9,
                header_room: 1.5,
            },
            drive: Drive::Rear,
            limited_slip: true,
            gearbox: 2,
            final_drive: 3.9,
            engines: codes(&["B16", "B16S", "R20"]),
            paint: 13,
            body: Body {
                length: 3.2,
                width: 0.98,
                front_overhang: 0.45,
                floor: 0.12,
                deck: vec![
                    [0.0, 0.34],
                    [0.1, 0.46],
                    [0.9, 0.6],
                    [1.45, 0.72],
                    [2.75, 0.74],
                    [3.12, 0.7],
                    [3.2, 0.56],
                ],
                cabin: vec![],
                nose: 0.5,
                tail: 0.8,
                tumblehome: 1.0,
                open_wheels: true,
                openings: vec![[1.55, 2.3, 0.32]],
                details: vec![
                    // An aero screen and a roll hoop.
                    detail([1.5, 0.79, 0.0], [0.03, 0.12, 0.7], Tone::Glass),
                    detail([2.38, 0.9, 0.3], [0.05, 0.4, 0.05], Tone::Dark),
                    detail([2.38, 1.09, 0.0], [0.05, 0.05, 0.65], Tone::Dark),
                ],
            },
            eye: [1.95, 0.9, 0.0],
            insulation: 0.0,
            traffic: 0.15,
        },
        Chassis {
            name: "Drover".into(),
            class: "Pickup".into(),
            about: "A tall four-wheel-drive truck on big tires: slow to turn, hard to stop, at \
                home where the road ends"
                .into(),
            mass: 1470.0,
            mass_at: 1.55,
            cg_height: 0.72,
            wheelbase: 3.1,
            track: [1.6, 1.6],
            wheels: [
                wheel(17, 80, 265, 70, TOURING),
                wheel(17, 80, 265, 70, TOURING),
            ],
            room: [
                WheelRoom {
                    diameter: 0.85,
                    width: 295,
                    min_rim: 16,
                },
                WheelRoom {
                    diameter: 0.85,
                    width: 295,
                    min_rim: 16,
                },
            ],
            hub: [33.0, 37.0],
            ride: [1.25, 1.5],
            damping: 0.3,
            bars: [
                bars(&[26, 28, 30, 32], 1, 250.0),
                bars(&[18, 20, 22], 1, 520.0),
            ],
            camber: [-0.3, -0.3],
            pressure: [2.5, 2.3],
            max_steer: 0.54,
            brakes: 0.76,
            drag_area: 1.0,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 80.0,
                at: 2.6,
                height: 0.45,
            },
            bay: EngineBay {
                place: "front".into(),
                orientation: Orientation::Longitudinal,
                length: 0.98,
                width: 0.86,
                height: 0.8,
                engine_at: 0.1,
                gearbox_at: 1.0,
                header_room: 1.2,
            },
            drive: Drive::All { front: 0.5 },
            limited_slip: true,
            gearbox: 5,
            final_drive: 3.9,
            engines: codes(&["V40", "M50", "W20T"]),
            paint: 20,
            body: Body {
                length: 5.25,
                width: 1.92,
                front_overhang: 0.9,
                floor: 0.28,
                deck: vec![
                    [0.0, 0.82],
                    [0.06, 1.0],
                    [1.15, 1.1],
                    [1.55, 1.18],
                    [3.2, 1.22],
                    [5.17, 1.22],
                    [5.25, 1.06],
                ],
                cabin: vec![
                    [1.5, 1.17, 1.0],
                    [2.0, 1.72, 1.0],
                    [2.4, 1.8, 1.0],
                    [3.1, 1.78, 1.0],
                    [3.3, 1.22, 1.0],
                ],
                nose: 0.9,
                tail: 0.96,
                tumblehome: 0.86,
                open_wheels: false,
                openings: vec![[3.42, 5.15, 0.8]],
                details: vec![
                    // Bumpers.
                    detail([0.0, 0.5, 0.0], [0.12, 0.16, 1.7], Tone::Metal),
                    detail([5.25, 0.55, 0.0], [0.1, 0.14, 1.7], Tone::Metal),
                ],
            },
            eye: [2.45, 1.55, 0.4],
            insulation: 0.35,
            traffic: 2.0,
        },
        Chassis {
            name: "Carrow".into(),
            class: "Van".into(),
            about: "A big box on wheels, driven by the front ones. It was never meant to be \
                driven like this"
                .into(),
            mass: 1420.0,
            mass_at: 1.6,
            cg_height: 0.74,
            wheelbase: 3.0,
            track: [1.62, 1.6],
            wheels: [
                wheel(16, 70, 225, 65, STREET),
                wheel(16, 70, 225, 65, STREET),
            ],
            room: [
                WheelRoom {
                    diameter: 0.75,
                    width: 255,
                    min_rim: 15,
                },
                WheelRoom {
                    diameter: 0.75,
                    width: 255,
                    min_rim: 15,
                },
            ],
            hub: [24.5, 24.5],
            ride: [1.3, 1.6],
            damping: 0.3,
            bars: [
                bars(&[22, 24, 26, 28], 1, 420.0),
                bars(&[18, 20, 22], 1, 590.0),
            ],
            camber: [-0.3, -0.5],
            pressure: [2.6, 2.5],
            max_steer: 0.56,
            brakes: 0.76,
            drag_area: 1.15,
            downforce_area: [0.0, 0.0],
            tank: Tank {
                litres: 70.0,
                at: 1.9,
                height: 0.4,
            },
            bay: EngineBay {
                place: "front, transverse".into(),
                orientation: Orientation::Transverse,
                length: 0.76,
                width: 1.04,
                height: 0.72,
                engine_at: -0.2,
                gearbox_at: -0.2,
                header_room: 1.05,
            },
            drive: Drive::Front,
            limited_slip: false,
            gearbox: 1,
            final_drive: 4.4,
            engines: codes(&["W20", "W20T"]),
            paint: 15,
            body: Body {
                length: 4.9,
                width: 1.9,
                front_overhang: 0.85,
                floor: 0.2,
                deck: vec![
                    [0.0, 0.62],
                    [0.06, 0.8],
                    [0.55, 0.95],
                    [0.85, 1.05],
                    [4.82, 1.05],
                    [4.9, 0.92],
                ],
                cabin: vec![
                    [0.8, 1.04, 1.0],
                    [1.45, 1.85, 1.0],
                    [2.3, 1.95, 0.0],
                    [4.75, 1.95, 0.0],
                    [4.88, 1.06, 0.0],
                ],
                nose: 0.9,
                tail: 0.97,
                tumblehome: 0.92,
                open_wheels: false,
                openings: vec![],
                details: vec![],
            },
            eye: [1.75, 1.6, 0.42],
            insulation: 0.25,
            traffic: 3.0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use crate::catalogue::{DEFAULT_CHASSIS, builtin};
    use crate::chassis::{CarBuild, Misfit};
    use crate::physics::engine::maps::EngineMaps;
    use crate::physics::engine::parts::Parts;
    use crate::wheels::{Rim, TireSize};

    fn chassis(name: &str) -> usize {
        builtin().find_chassis(name).expect("a chassis")
    }

    fn family(code: &str) -> usize {
        builtin().find_engine(code).expect("an engine").0
    }

    #[test]
    fn factory_cars_are_complete() {
        let catalogue = builtin();
        assert_eq!(catalogue.chassis[DEFAULT_CHASSIS].name, "Vireo");
        for (index, made) in catalogue.chassis.iter().enumerate() {
            let build = CarBuild::factory(index);
            assert!(
                catalogue
                    .gearboxes_for(made)
                    .any(|gearbox| gearbox == made.gearbox)
            );
            assert!(!catalogue.paints[made.paint].shop);
            // The factory's wheels fit, on the sizes on offer.
            for (room, wheel) in made.room.iter().zip(&made.wheels) {
                assert_eq!(room.misfit(wheel), None, "{}", made.name);
                assert!(Rim::all().any(|rim| rim == wheel.rim), "{}", made.name);
                assert!(TireSize::all().any(|tire| tire == wheel.tire));
            }
            for (option, code) in made.engines.iter().enumerate() {
                // The factory's engines fit without a swap subframe or an adapter, with
                // their own exhaust manifold: their maps come baked with the game.
                let fit = made.fit(&catalogue, family(code)).expect("it fits");
                assert!(!fit.swap && !catalogue.needs_adapter(made.gearbox, family(code)));
                let parts = build.factory_engine(option);
                let hardware = parts.hardware();
                let free = Parts::factory(code).unwrap().hardware();
                assert_eq!(
                    hardware.exhaust.header_length, free.exhaust.header_length,
                    "{} with the {code}",
                    made.name
                );
                assert!(EngineMaps::stock(&hardware).is_some());
            }
            // The bay's place matches where the engine sits.
            let at = made.bay.engine_at / made.wheelbase;
            let place = &made.bay.place;
            assert_eq!(place.starts_with("front"), at < 0.3, "{}", made.name);
            assert_eq!(place == "rear", at > 1.0, "{}", made.name);
        }
    }

    #[test]
    fn space_decides_what_fits() {
        let catalogue = builtin();
        let fits =
            |car: &str, code: &str| catalogue.chassis[chassis(car)].fit(&catalogue, family(code));
        // The high-revving four from the hot hatch goes lengthways into the coupé and
        // the roadster on a swap subframe; the roadster even takes the big V8.
        for car in ["Tarn", "Linnet", "Wisp", "Minim"] {
            let fit = fits(car, "R20").unwrap_or_else(|misfit| panic!("{car}: {}", misfit.0));
            assert_eq!(fit.swap, car != "Wisp", "{car}");
        }
        assert!(fits("Linnet", "M62").is_ok());
        // A straight six is too long for the roadster, a V12 for the coupé, and only
        // flat engines go under the rear-engined car's deck.
        assert_eq!(
            fits("Linnet", "S30"),
            Err(Misfit("14 cm too long for the front bay".to_string()))
        );
        assert!(fits("Tarn", "L65").is_err() && fits("Halden", "L65").is_ok());
        let rheon = &catalogue.chassis[chassis("Rheon")];
        for (f, engine) in catalogue.families.iter().enumerate() {
            let flat = engine.height < 0.58;
            assert_eq!(rheon.fit(&catalogue, f).is_ok(), flat, "{}", engine.name);
        }
        // Across the city car, there's no room for six cylinders.
        assert!(fits("Minim", "V35").is_err() && fits("Vireo", "V35").is_ok());
        // Every engine has a home, and every car takes more than its own engines.
        for (f, engine) in catalogue.families.iter().enumerate() {
            let homes = (catalogue.chassis.iter())
                .filter(|c| c.fit(&catalogue, f).is_ok())
                .count();
            assert!(homes >= 2, "{}: {homes}", engine.name);
        }
        for made in &catalogue.chassis {
            let takes = (0..catalogue.families.len()).filter(|&f| made.fit(&catalogue, f).is_ok());
            assert!(takes.count() >= 2, "{}", made.name);
        }
    }

    #[test]
    fn tight_bays_shorten_the_headers() {
        // The V8 fills the roadster's bay from side to side: short primaries only. A
        // four leaves room for long ones.
        let catalogue = builtin();
        let roadster = &catalogue.chassis[chassis("Linnet")];
        let (v8, four) = (
            roadster.fit(&catalogue, family("M62")).unwrap(),
            roadster.fit(&catalogue, family("B16")).unwrap(),
        );
        assert!(v8.header_room < 0.65 && four.header_room > 0.7);
        let build = CarBuild::factory(chassis("Linnet"));
        let swapped = build.install(Parts::factory("M62").unwrap()).unwrap();
        let hardware = swapped.hardware();
        assert_eq!(hardware.exhaust.header_length, v8.header_room);
        // Its maps aren't the factory's any more: the manifold is part of the cylinder
        // side.
        assert!(EngineMaps::stock(&hardware).is_none());
    }

    #[test]
    fn gearboxes_bolt_to_their_engines() {
        let catalogue = builtin();
        let coupe = &catalogue.chassis[chassis("Tarn")];
        let boxes: Vec<&str> = (catalogue.gearboxes_for(coupe))
            .map(|g| catalogue.gearboxes[g].name.as_str())
            .collect();
        assert_eq!(
            boxes,
            [
                "5-speed",
                "6-speed",
                "6-speed dog box",
                "6-speed heavy duty"
            ]
        );
        // The coupé's own gearbox takes its engines; the hot hatch's four needs a plate.
        assert!(!catalogue.needs_adapter(coupe.gearbox, family("S30")));
        assert!(catalogue.needs_adapter(coupe.gearbox, family("R20")));
        // The track-day special has the R20 on the 5-speed from the factory.
        assert!(!catalogue.needs_adapter(2, family("R20")));
    }
}
