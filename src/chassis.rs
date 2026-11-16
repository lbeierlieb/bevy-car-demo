//! The cars there are, without their engines: each chassis has an engine bay, a body,
//! suspension, wheels and brakes, and a list of the engines the factory offers in it.
//! Engines, gearboxes and the rest are independent of the chassis: a car is built from a
//! chassis and whatever fits it (`CarBuild`, and the engine's `Parts`).
//!
//! What fits: an engine goes into any bay it has the space for, the only hard limit. An
//! engine the factory never offered there needs a swap subframe, a gearbox made for other
//! engines an adapter plate, and the room around the engine limits how long the exhaust
//! manifold's primaries can be.

use bevy::prelude::*;

use crate::body::{Body, Detail, Tone, Wheels};
use crate::physics::drivetrain::Drive;
use crate::physics::engine::families::{FAMILIES, Family};
use crate::physics::engine::parts::Parts;
use crate::physics::fuel::Tank;

/// Width a transverse engine's gearbox takes up at the end of its crank (m).
const TRANSAXLE_LENGTH: f32 = 0.36;
/// Room beside the engine with which the longest primaries fit, and the share of them
/// that fits with no room to spare.
const HEADER_SLACK: f32 = 0.3;
const HEADER_LEAST: f32 = 0.55;

/// Where the engine sits and which way its crank points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Across the car, the gearbox on the end of the crank beside it.
    Transverse,
    Longitudinal,
}

#[derive(Clone, Copy, Debug)]
pub struct EngineBay {
    pub place: &'static str,
    pub orientation: Orientation,
    /// The space there is: along the car, across it, and up (m).
    pub length: f32,
    pub width: f32,
    pub height: f32,
    /// Where the engine's and the gearbox's centres are, behind the front axle (m).
    pub engine_at: f32,
    pub gearbox_at: f32,
    /// The longest exhaust primaries that fit, around an engine with room to spare (m).
    pub header_room: f32,
}

/// How a gearbox is built, which is what a chassis needs of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GearboxKind {
    /// Beside a transverse engine, with the differential in it.
    Transverse,
    /// Behind a longitudinal engine at the front, a shaft to the driven axle.
    Inline,
    /// At the driven axle of a mid- or rear-engined car, with the differential in it.
    Transaxle,
}

pub struct Gearbox {
    pub name: &'static str,
    pub kind: GearboxKind,
    pub gears: &'static [f32],
    pub reverse: f32,
    pub mass: f32,
    /// Most torque its stock clutch passes (N·m).
    pub clutch_torque: f32,
    /// Time the clutch stays open for a gear change, and takes to close again (s).
    pub shift_time: f32,
    pub engage_time: f32,
}

pub static GEARBOXES: [Gearbox; 8] = [
    Gearbox {
        name: "5-speed (transverse)",
        kind: GearboxKind::Transverse,
        gears: &[3.55, 1.95, 1.31, 0.97, 0.78],
        reverse: 3.4,
        mass: 35.0,
        clutch_torque: 200.0,
        shift_time: 0.28,
        engage_time: 0.16,
    },
    Gearbox {
        name: "6-speed close-ratio (transverse)",
        kind: GearboxKind::Transverse,
        gears: &[3.3, 2.05, 1.48, 1.14, 0.92, 0.76],
        reverse: 3.3,
        mass: 42.0,
        clutch_torque: 380.0,
        shift_time: 0.22,
        engage_time: 0.15,
    },
    Gearbox {
        name: "5-speed",
        kind: GearboxKind::Inline,
        gears: &[3.4, 2.0, 1.4, 1.0, 0.81],
        reverse: 3.4,
        mass: 38.0,
        clutch_torque: 270.0,
        shift_time: 0.22,
        engage_time: 0.15,
    },
    Gearbox {
        name: "6-speed",
        kind: GearboxKind::Inline,
        gears: &[3.3, 2.1, 1.5, 1.17, 0.95, 0.8],
        reverse: 3.3,
        mass: 48.0,
        clutch_torque: 600.0,
        shift_time: 0.25,
        engage_time: 0.15,
    },
    Gearbox {
        name: "6-speed dog box",
        kind: GearboxKind::Inline,
        gears: &[3.1, 2.2, 1.7, 1.35, 1.1, 0.92],
        reverse: 3.1,
        mass: 52.0,
        clutch_torque: 520.0,
        shift_time: 0.1,
        engage_time: 0.1,
    },
    Gearbox {
        name: "6-speed heavy duty",
        kind: GearboxKind::Inline,
        gears: &[2.66, 1.78, 1.3, 1.0, 0.74, 0.5],
        reverse: 2.9,
        mass: 60.0,
        clutch_torque: 760.0,
        shift_time: 0.28,
        engage_time: 0.16,
    },
    Gearbox {
        name: "6-speed transaxle",
        kind: GearboxKind::Transaxle,
        gears: &[3.2, 2.0, 1.45, 1.12, 0.92, 0.78],
        reverse: 3.2,
        mass: 55.0,
        clutch_torque: 760.0,
        shift_time: 0.18,
        engage_time: 0.12,
    },
    Gearbox {
        name: "7-speed dual-clutch transaxle",
        kind: GearboxKind::Transaxle,
        gears: &[3.4, 2.3, 1.75, 1.4, 1.15, 0.97, 0.82],
        reverse: 3.2,
        mass: 75.0,
        clutch_torque: 820.0,
        shift_time: 0.06,
        engage_time: 0.08,
    },
];

/// Clutches: the name, and how much more torque than the gearbox's stock one they pass.
pub const CLUTCHES: [(&str, f32); 3] = [("Stock", 1.0), ("Sport", 1.6), ("Race", 2.5)];

/// Suspension kits: the name, the ride frequency and damping relative to the chassis'
/// own, and the extra negative camber (degrees).
pub const SUSPENSIONS: [(&str, f32, f32, f32); 3] = [
    ("Stock", 1.0, 1.0, 0.0),
    ("Sport", 1.2, 1.15, 0.8),
    ("Track", 1.45, 1.3, 1.8),
];


/// Paints every car comes in.
pub const PAINTS: [(&str, [f32; 3]); 10] = [
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

pub struct Chassis {
    pub name: &'static str,
    pub class: &'static str,
    pub about: &'static str,
    /// Sprung mass without engine and gearbox (kg), its centre behind the front axle (m),
    /// and the height of the whole car's centre of mass (m).
    pub mass: f32,
    pub mass_at: f32,
    pub cg_height: f32,
    pub wheelbase: f32,
    /// Front and rear: track, tire radius and width (m), unsprung mass per corner (kg).
    pub track: [f32; 2],
    pub tire_radius: [f32; 2],
    pub tire_width: [f32; 2],
    pub unsprung: [f32; 2],
    /// Peak friction of the stock tires.
    pub grip: f32,
    /// Ride frequency front and rear (Hz), and damping ratio.
    pub ride: [f32; 2],
    pub damping: f32,
    /// Static camber front and rear (degrees).
    pub camber: [f32; 2],
    pub max_steer: f32,
    /// Share of the stock tires' grip the brakes use with the pedal to the floor. Well
    /// short of all of it, they don't lock the wheels on a dry road; with all-wheel drive
    /// that matters, as locked front wheels drag the rear ones along through the centre
    /// differential.
    pub brakes: f32,
    pub drag_area: f32,
    pub downforce_area: [f32; 2],
    /// The fuel tank.
    pub tank: Tank,
    pub bay: EngineBay,
    pub drive: Drive,
    /// Limited-slip differentials from the factory.
    pub limited_slip: bool,
    /// The factory's gearbox (in `GEARBOXES`) and final drive.
    pub gearbox: usize,
    pub final_drive: f32,
    /// Codes of the engines the factory offers, the base one first.
    pub engines: &'static [&'static str],
    /// The paint (in `PAINTS`) it's shown in.
    pub paint: usize,
    pub body: Body,
    /// The driver's eyes: behind the nose, above the ground, and left of the centreline.
    pub eye: [f32; 3],
}

/// Why an engine doesn't go into a bay.
#[derive(Clone, Debug, PartialEq)]
pub struct Misfit(pub String);

/// How an engine sits in a chassis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// The factory never offered the engine's family here: it needs a swap subframe.
    pub swap: bool,
    /// Longest exhaust primaries there's room for around the engine (m).
    pub header_room: f32,
}

impl Chassis {
    pub fn wheels(&self) -> Wheels {
        Wheels {
            wheelbase: self.wheelbase,
            track: self.track,
            radius: self.tire_radius,
            width: self.tire_width,
        }
    }

    /// The families of the engines the factory offers.
    fn factory_families(&self) -> impl Iterator<Item = usize> {
        self.engines
            .iter()
            .filter_map(|code| Family::find(code).map(|(family, _)| family))
    }

    /// How an engine of `family` (in `FAMILIES`) fits, or why it doesn't: it needs the
    /// space, and nothing else.
    pub fn fit(&self, family: usize) -> Result<Fit, Misfit> {
        let engine = &FAMILIES[family];
        let bay = &self.bay;
        // Across the car, the gearbox sits on the end of the crank.
        let (along, across) = match bay.orientation {
            Orientation::Longitudinal => (engine.length, engine.width),
            Orientation::Transverse => (engine.width, engine.length + TRANSAXLE_LENGTH),
        };
        let short = [
            (along - bay.length, "long"),
            (across - bay.width, "wide"),
            (engine.height - bay.height, "tall"),
        ];
        let over: Vec<String> = short
            .iter()
            .filter(|(by, _)| *by > 1e-4)
            .map(|(by, way)| format!("{:.0} cm too {way}", (by * 100.0 - 0.05).ceil().max(1.0)))
            .collect();
        if !over.is_empty() {
            return Err(Misfit(format!(
                "{} for the {} bay",
                over.join(", "),
                bay.place
            )));
        }
        // The primaries run beside a longitudinal engine, and ahead of or behind a
        // transverse one.
        let slack = match bay.orientation {
            Orientation::Longitudinal => bay.width - across,
            Orientation::Transverse => bay.length - along,
        };
        let share = (HEADER_LEAST + (1.0 - HEADER_LEAST) * slack / HEADER_SLACK).min(1.0);
        Ok(Fit {
            swap: !self.factory_families().any(|offered| offered == family),
            header_room: (bay.header_room * share * 100.0).round() / 100.0,
        })
    }

    /// The gearboxes that suit the chassis' layout (in `GEARBOXES`).
    pub fn gearboxes(&self) -> impl Iterator<Item = usize> {
        let kind = GEARBOXES[self.gearbox].kind;
        (0..GEARBOXES.len()).filter(move |&gearbox| GEARBOXES[gearbox].kind == kind)
    }
}

/// Whether `gearbox` needs an adapter plate to bolt to an engine of `family`: it does
/// unless some factory car pairs the two.
pub fn needs_adapter(gearbox: usize, family: usize) -> bool {
    !CHASSIS.iter().any(|chassis| {
        chassis.gearbox == gearbox && chassis.factory_families().any(|offered| offered == family)
    })
}

/// A car as built, apart from its engine (which is its `Parts`): the chassis and what's
/// fitted to it, each by its place in its catalogue.
#[derive(Clone, Debug, PartialEq)]
pub struct CarBuild {
    pub chassis: usize,
    pub paint: usize,
    pub gearbox: usize,
    pub clutch: usize,
    pub final_drive: f32,
    pub limited_slip: bool,
    pub suspension: usize,
    pub tires: usize,
}

impl CarBuild {
    /// The chassis as it leaves the factory.
    pub fn factory(chassis: usize) -> Self {
        let made = &CHASSIS[chassis];
        Self {
            chassis,
            paint: made.paint,
            gearbox: made.gearbox,
            clutch: 0,
            final_drive: made.final_drive,
            limited_slip: made.limited_slip,
            suspension: 0,
            tires: 0,
        }
    }

    pub fn chassis(&self) -> &'static Chassis {
        &CHASSIS[self.chassis]
    }

    pub fn gearbox(&self) -> &'static Gearbox {
        &GEARBOXES[self.gearbox]
    }

    pub fn paint(&self) -> Color {
        let [r, g, b] = PAINTS[self.paint % PAINTS.len()].1;
        Color::srgb(r, g, b)
    }

    /// The factory's engine `option` for the chassis (the base one if there's no such),
    /// fitted to it.
    pub fn factory_engine(&self, option: usize) -> Parts {
        let engines = self.chassis().engines;
        let code = engines.get(option).unwrap_or(&engines[0]);
        let parts = Parts::factory(code).expect("a factory engine");
        self.install(parts).expect("factory engines fit")
    }

    /// `parts` as they sit in this car (with the room there is for their exhaust
    /// manifold), or why the engine doesn't fit.
    pub fn install(&self, mut parts: Parts) -> Result<Parts, Misfit> {
        parts.header_room = self.chassis().fit(parts.family)?.header_room;
        Ok(parts)
    }
}

const fn detail(at: [f32; 3], size: [f32; 3], tone: Tone) -> Detail {
    Detail { at, size, tone }
}

/// The coupé, which the game starts with.
pub const DEFAULT_CHASSIS: usize = 3;

pub static CHASSIS: [Chassis; 13] = [
    Chassis {
        name: "Minim",
        class: "City car",
        about: "Small, light and upright. Three cylinders are plenty; a turbo makes it \
                cheeky",
        mass: 610.0,
        mass_at: 1.2,
        cg_height: 0.54,
        wheelbase: 2.35,
        track: [1.42, 1.41],
        tire_radius: [0.29, 0.29],
        tire_width: [0.17, 0.17],
        unsprung: [30.0, 28.0],
        grip: 0.95,
        ride: [1.3, 1.45],
        damping: 0.3,
        camber: [-0.8, -1.2],
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
            place: "front, transverse",
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
        engines: &["A10", "A10T", "B12"],
        paint: 9,
        body: Body {
            length: 3.55,
            width: 1.63,
            front_overhang: 0.68,
            floor: 0.17,
            deck: &[
                [0.0, 0.54],
                [0.06, 0.68],
                [0.55, 0.82],
                [0.85, 0.92],
                [3.47, 0.94],
                [3.55, 0.84],
            ],
            cabin: &[
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
            openings: &[],
            details: &[],
        },
        eye: [1.75, 1.2, 0.34],
    },
    Chassis {
        name: "Vireo",
        class: "Hot hatch",
        about: "A practical hatchback that the factory couldn't leave alone: front-wheel \
                drive with a limited-slip differential and a choice of eager fours",
        mass: 890.0,
        mass_at: 1.32,
        cg_height: 0.51,
        wheelbase: 2.6,
        track: [1.53, 1.51],
        tire_radius: [0.31, 0.31],
        tire_width: [0.21, 0.21],
        unsprung: [36.0, 32.0],
        grip: 1.05,
        ride: [1.6, 1.85],
        damping: 0.34,
        camber: [-1.5, -1.5],
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
            place: "front, transverse",
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
        engines: &["B16", "R20", "W20T"],
        paint: 0,
        body: Body {
            length: 4.15,
            width: 1.78,
            front_overhang: 0.85,
            floor: 0.15,
            deck: &[
                [0.0, 0.5],
                [0.06, 0.64],
                [0.8, 0.78],
                [1.15, 0.88],
                [4.07, 0.92],
                [4.15, 0.82],
            ],
            cabin: &[
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
            openings: &[],
            details: &[
                // A roof spoiler over the hatch.
                detail([3.72, 1.42, 0.0], [0.3, 0.03, 1.1], Tone::Dark),
            ],
        },
        eye: [2.15, 1.17, 0.36],
    },
    Chassis {
        name: "Linnet",
        class: "Roadster",
        about: "Two seats, no roof, rear-wheel drive and not much weight: it doesn't need \
                power to be fun, though there's room for a V8",
        mass: 740.0,
        mass_at: 1.27,
        cg_height: 0.44,
        wheelbase: 2.31,
        track: [1.43, 1.45],
        tire_radius: [0.3, 0.3],
        tire_width: [0.19, 0.19],
        unsprung: [32.0, 34.0],
        grip: 1.05,
        ride: [1.5, 1.65],
        damping: 0.32,
        camber: [-1.2, -1.5],
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
            place: "front",
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
        engines: &["B16", "B16S"],
        paint: 6,
        body: Body {
            length: 3.95,
            width: 1.7,
            front_overhang: 0.8,
            floor: 0.14,
            deck: &[
                [0.0, 0.46],
                [0.06, 0.56],
                [0.9, 0.68],
                [1.45, 0.8],
                [2.6, 0.82],
                [3.2, 0.84],
                [3.87, 0.8],
                [3.95, 0.66],
            ],
            cabin: &[[1.42, 0.79, 1.0], [1.85, 1.16, 1.0]],
            nose: 0.8,
            tail: 0.84,
            tumblehome: 0.86,
            open_wheels: false,
            openings: &[[1.6, 2.72, 0.56]],
            details: &[
                // Hoops behind the seats.
                detail([2.74, 0.98, 0.32], [0.07, 0.3, 0.26], Tone::Dark),
            ],
        },
        eye: [2.3, 1.03, 0.33],
    },
    Chassis {
        name: "Tarn",
        class: "Sports coupé",
        about: "The all-rounder: engine in front, drive at the back, a long bay that takes \
                a straight six or a V8",
        mass: 930.0,
        mass_at: 1.42,
        cg_height: 0.46,
        wheelbase: 2.57,
        track: [1.52, 1.53],
        tire_radius: [0.32, 0.32],
        tire_width: [0.22, 0.23],
        unsprung: [38.0, 40.0],
        grip: 1.1,
        ride: [1.65, 1.8],
        damping: 0.33,
        camber: [-1.2, -1.4],
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
            place: "front",
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
        engines: &["W20", "W20T", "S30"],
        paint: 0,
        body: Body {
            length: 4.3,
            width: 1.8,
            front_overhang: 0.85,
            floor: 0.14,
            deck: &[
                [0.0, 0.48],
                [0.06, 0.6],
                [0.95, 0.74],
                [1.45, 0.84],
                [3.35, 0.9],
                [4.22, 0.9],
                [4.3, 0.76],
            ],
            cabin: &[
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
            openings: &[],
            details: &[],
        },
        eye: [2.45, 1.1, 0.36],
    },
    Chassis {
        name: "Halden",
        class: "Saloon",
        about: "Four doors and a big bay: comfortable with a turbo four, quick with a six, \
                and the only front-engined car that swallows a V12",
        mass: 1200.0,
        mass_at: 1.55,
        cg_height: 0.52,
        wheelbase: 2.85,
        track: [1.58, 1.6],
        tire_radius: [0.33, 0.33],
        tire_width: [0.23, 0.24],
        unsprung: [42.0, 44.0],
        grip: 1.05,
        ride: [1.35, 1.5],
        damping: 0.32,
        camber: [-0.8, -1.2],
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
            place: "front",
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
        engines: &["W20T", "S30", "S30T", "M50"],
        paint: 5,
        body: Body {
            length: 4.8,
            width: 1.87,
            front_overhang: 0.88,
            floor: 0.15,
            deck: &[
                [0.0, 0.52],
                [0.06, 0.66],
                [1.0, 0.82],
                [1.5, 0.92],
                [3.9, 0.96],
                [4.72, 0.96],
                [4.8, 0.8],
            ],
            cabin: &[
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
            openings: &[],
            details: &[],
        },
        eye: [2.5, 1.2, 0.38],
    },
    Chassis {
        name: "Skarn",
        class: "Rally car",
        about: "All-wheel drive, limited-slip differentials everywhere and a turbocharged \
                boxer ahead of the front axle: made for loose surfaces",
        mass: 980.0,
        mass_at: 1.4,
        cg_height: 0.49,
        wheelbase: 2.55,
        track: [1.53, 1.53],
        tire_radius: [0.32, 0.32],
        tire_width: [0.22, 0.22],
        unsprung: [40.0, 40.0],
        grip: 1.1,
        ride: [1.7, 1.8],
        damping: 0.36,
        camber: [-1.5, -1.5],
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
            place: "front",
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
        engines: &["X20T", "X25T", "W20T"],
        paint: 1,
        body: Body {
            length: 4.4,
            width: 1.8,
            front_overhang: 0.9,
            floor: 0.17,
            deck: &[
                [0.0, 0.52],
                [0.06, 0.66],
                [0.95, 0.8],
                [1.4, 0.9],
                [3.6, 0.94],
                [4.32, 0.96],
                [4.4, 0.8],
            ],
            cabin: &[
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
            openings: &[],
            details: &[
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
    },
    Chassis {
        name: "Harrow",
        class: "Muscle car",
        about: "A long bonnet with a V8 under it, a heavy-duty gearbox and wide rear tires \
                that still aren't wide enough",
        mass: 1210.0,
        mass_at: 1.55,
        cg_height: 0.5,
        wheelbase: 2.75,
        track: [1.6, 1.62],
        tire_radius: [0.34, 0.34],
        tire_width: [0.24, 0.27],
        unsprung: [44.0, 50.0],
        grip: 1.05,
        ride: [1.4, 1.5],
        damping: 0.3,
        camber: [-0.8, -1.0],
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
            place: "front",
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
        engines: &["M50", "M62", "V35"],
        paint: 7,
        body: Body {
            length: 4.8,
            width: 1.93,
            front_overhang: 0.9,
            floor: 0.15,
            deck: &[
                [0.0, 0.6],
                [0.05, 0.74],
                [1.2, 0.84],
                [1.75, 0.92],
                [3.8, 0.98],
                [4.72, 0.98],
                [4.8, 0.82],
            ],
            cabin: &[
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
            openings: &[],
            details: &[
                // A bulge down the bonnet, and a lip on the boot.
                detail([0.95, 0.83, 0.0], [1.3, 0.07, 0.5], Tone::Paint),
                detail([4.72, 1.0, 0.0], [0.12, 0.05, 1.5], Tone::Paint),
            ],
        },
        eye: [2.75, 1.15, 0.38],
    },
    Chassis {
        name: "Calder",
        class: "Mid-engined sports car",
        about: "The engine sits behind the seats, where its weight helps the car turn and \
                put its power down",
        mass: 870.0,
        mass_at: 1.2,
        cg_height: 0.42,
        wheelbase: 2.45,
        track: [1.54, 1.56],
        tire_radius: [0.32, 0.33],
        tire_width: [0.22, 0.27],
        unsprung: [38.0, 44.0],
        grip: 1.15,
        ride: [1.9, 2.1],
        damping: 0.36,
        camber: [-1.5, -1.8],
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
            place: "middle",
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
        engines: &["V35", "F40"],
        paint: 2,
        body: Body {
            length: 4.1,
            width: 1.85,
            front_overhang: 0.95,
            floor: 0.12,
            deck: &[
                [0.0, 0.4],
                [0.06, 0.5],
                [0.95, 0.64],
                [1.35, 0.76],
                [2.9, 0.84],
                [3.5, 0.9],
                [4.02, 0.88],
                [4.1, 0.72],
            ],
            cabin: &[
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
            openings: &[],
            details: &[
                // Intakes ahead of the rear wheels.
                detail([2.75, 0.56, 0.915], [0.4, 0.22, 0.03], Tone::Dark),
                detail([4.0, 0.93, 0.0], [0.16, 0.03, 1.5], Tone::Paint),
            ],
        },
        eye: [2.25, 1.02, 0.35],
    },
    Chassis {
        name: "Rheon",
        class: "Rear-engined sports car",
        about: "A flat six hung out behind the rear axle: traction for days, and a tail \
                that wants to lead when you lift. Only flat engines fit under its deck",
        mass: 970.0,
        mass_at: 1.2,
        cg_height: 0.45,
        wheelbase: 2.4,
        track: [1.5, 1.54],
        tire_radius: [0.32, 0.33],
        tire_width: [0.22, 0.28],
        unsprung: [38.0, 46.0],
        grip: 1.15,
        ride: [1.8, 2.0],
        damping: 0.36,
        camber: [-1.0, -1.8],
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
            place: "rear",
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
        engines: &["H30", "H36T"],
        paint: 5,
        body: Body {
            length: 4.35,
            width: 1.84,
            front_overhang: 0.95,
            floor: 0.13,
            deck: &[
                [0.0, 0.46],
                [0.06, 0.58],
                [0.95, 0.7],
                [1.4, 0.82],
                [3.2, 0.88],
                [4.0, 0.8],
                [4.27, 0.7],
                [4.35, 0.58],
            ],
            cabin: &[
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
            openings: &[],
            details: &[
                // The engine cover's grille.
                detail([4.08, 0.79, 0.0], [0.22, 0.03, 0.9], Tone::Dark),
            ],
        },
        eye: [2.4, 1.08, 0.35],
    },
    Chassis {
        name: "Vespera",
        class: "Supercar",
        about: "Low, wide and loud, with ten or twelve cylinders amidships, all-wheel drive \
                and wings that push it into the road",
        mass: 1050.0,
        mass_at: 1.3,
        cg_height: 0.4,
        wheelbase: 2.65,
        track: [1.66, 1.62],
        tire_radius: [0.33, 0.35],
        tire_width: [0.25, 0.32],
        unsprung: [42.0, 52.0],
        grip: 1.25,
        ride: [2.2, 2.4],
        damping: 0.4,
        camber: [-1.8, -2.0],
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
            place: "middle",
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
        engines: &["D52", "L65"],
        paint: 7,
        body: Body {
            length: 4.55,
            width: 2.0,
            front_overhang: 1.05,
            floor: 0.11,
            deck: &[
                [0.0, 0.34],
                [0.06, 0.42],
                [1.05, 0.58],
                [1.45, 0.7],
                [3.0, 0.8],
                [3.8, 0.88],
                [4.47, 0.88],
                [4.55, 0.7],
            ],
            cabin: &[
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
            openings: &[],
            details: &[
                // Side intakes, a splitter, and a wing on two posts.
                detail([3.0, 0.5, 0.99], [0.5, 0.26, 0.03], Tone::Dark),
                detail([0.1, 0.14, 0.0], [0.3, 0.03, 1.5], Tone::Dark),
                detail([4.35, 0.98, 0.6], [0.08, 0.2, 0.04], Tone::Dark),
                detail([4.37, 1.08, 0.0], [0.3, 0.03, 1.8], Tone::Paint),
            ],
        },
        eye: [2.4, 0.97, 0.38],
    },
    Chassis {
        name: "Wisp",
        class: "Track-day special",
        about: "A tub, four open wheels and nothing else: half the weight of anything \
                else, so a small four is all it takes",
        mass: 330.0,
        mass_at: 1.25,
        cg_height: 0.36,
        wheelbase: 2.25,
        track: [1.42, 1.4],
        tire_radius: [0.28, 0.28],
        tire_width: [0.19, 0.21],
        unsprung: [24.0, 26.0],
        grip: 1.2,
        ride: [2.0, 2.2],
        damping: 0.38,
        camber: [-1.8, -1.8],
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
            place: "front",
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
        engines: &["B16", "B16S", "R20"],
        paint: 2,
        body: Body {
            length: 3.2,
            width: 0.98,
            front_overhang: 0.45,
            floor: 0.12,
            deck: &[
                [0.0, 0.34],
                [0.1, 0.46],
                [0.9, 0.6],
                [1.45, 0.72],
                [2.75, 0.74],
                [3.12, 0.7],
                [3.2, 0.56],
            ],
            cabin: &[],
            nose: 0.5,
            tail: 0.8,
            tumblehome: 1.0,
            open_wheels: true,
            openings: &[[1.55, 2.3, 0.32]],
            details: &[
                // An aero screen and a roll hoop.
                detail([1.5, 0.79, 0.0], [0.03, 0.12, 0.7], Tone::Glass),
                detail([2.38, 0.9, 0.3], [0.05, 0.4, 0.05], Tone::Dark),
                detail([2.38, 1.09, 0.0], [0.05, 0.05, 0.65], Tone::Dark),
            ],
        },
        eye: [1.95, 0.9, 0.0],
    },
    Chassis {
        name: "Drover",
        class: "Pickup",
        about: "A tall four-wheel-drive truck on big tires: slow to turn, hard to stop, at \
                home where the road ends",
        mass: 1470.0,
        mass_at: 1.55,
        cg_height: 0.72,
        wheelbase: 3.1,
        track: [1.6, 1.6],
        tire_radius: [0.38, 0.38],
        tire_width: [0.26, 0.26],
        unsprung: [58.0, 62.0],
        grip: 0.95,
        ride: [1.25, 1.5],
        damping: 0.3,
        camber: [-0.3, -0.3],
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
            place: "front",
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
        engines: &["V40", "M50", "W20T"],
        paint: 4,
        body: Body {
            length: 5.25,
            width: 1.92,
            front_overhang: 0.9,
            floor: 0.28,
            deck: &[
                [0.0, 0.82],
                [0.06, 1.0],
                [1.15, 1.1],
                [1.55, 1.18],
                [3.2, 1.22],
                [5.17, 1.22],
                [5.25, 1.06],
            ],
            cabin: &[
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
            openings: &[[3.42, 5.15, 0.8]],
            details: &[
                // Bumpers.
                detail([0.0, 0.5, 0.0], [0.12, 0.16, 1.7], Tone::Metal),
                detail([5.25, 0.55, 0.0], [0.1, 0.14, 1.7], Tone::Metal),
            ],
        },
        eye: [2.45, 1.55, 0.4],
    },
    Chassis {
        name: "Carrow",
        class: "Van",
        about: "A big box on wheels, driven by the front ones. It was never meant to be \
                driven like this",
        mass: 1420.0,
        mass_at: 1.6,
        cg_height: 0.74,
        wheelbase: 3.0,
        track: [1.62, 1.6],
        tire_radius: [0.33, 0.33],
        tire_width: [0.21, 0.21],
        unsprung: [44.0, 44.0],
        grip: 0.95,
        ride: [1.3, 1.6],
        damping: 0.3,
        camber: [-0.3, -0.5],
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
            place: "front, transverse",
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
        engines: &["W20", "W20T"],
        paint: 3,
        body: Body {
            length: 4.9,
            width: 1.9,
            front_overhang: 0.85,
            floor: 0.2,
            deck: &[
                [0.0, 0.62],
                [0.06, 0.8],
                [0.55, 0.95],
                [0.85, 1.05],
                [4.82, 1.05],
                [4.9, 0.92],
            ],
            cabin: &[
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
            openings: &[],
            details: &[],
        },
        eye: [1.75, 1.6, 0.42],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::maps::EngineMaps;

    fn chassis(name: &str) -> usize {
        CHASSIS
            .iter()
            .position(|chassis| chassis.name == name)
            .expect("a chassis")
    }

    fn family(code: &str) -> usize {
        Family::find(code).expect("an engine").0
    }

    #[test]
    fn factory_cars_are_complete() {
        assert_eq!(CHASSIS[DEFAULT_CHASSIS].class, "Sports coupé");
        for (index, made) in CHASSIS.iter().enumerate() {
            let build = CarBuild::factory(index);
            assert!(made.gearboxes().any(|gearbox| gearbox == made.gearbox));
            assert!(made.paint < PAINTS.len());
            for (option, code) in made.engines.iter().enumerate() {
                // The factory's engines fit without a swap subframe or an adapter, with
                // their own exhaust manifold: their maps come baked with the game.
                let fit = made.fit(family(code)).expect("it fits");
                assert!(!fit.swap && !needs_adapter(made.gearbox, family(code)));
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
            let place = made.bay.place;
            assert_eq!(place.starts_with("front"), at < 0.3, "{}", made.name);
            assert_eq!(place == "rear", at > 1.0, "{}", made.name);
        }
    }

    #[test]
    fn space_decides_what_fits() {
        let fits = |car: &str, code: &str| CHASSIS[chassis(car)].fit(family(code));
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
        for (f, engine) in FAMILIES.iter().enumerate() {
            let flat = engine.height < 0.58;
            assert_eq!(
                CHASSIS[chassis("Rheon")].fit(f).is_ok(),
                flat,
                "{}",
                engine.name
            );
        }
        // Across the city car, there's no room for six cylinders.
        assert!(fits("Minim", "V35").is_err() && fits("Vireo", "V35").is_ok());
        // Every engine has a home, and every car takes more than its own engines.
        for (f, engine) in FAMILIES.iter().enumerate() {
            let homes = CHASSIS.iter().filter(|c| c.fit(f).is_ok()).count();
            assert!(homes >= 2, "{}: {homes}", engine.name);
        }
        for made in &CHASSIS {
            let takes = (0..FAMILIES.len()).filter(|&f| made.fit(f).is_ok());
            assert!(takes.count() >= 2, "{}", made.name);
        }
    }

    #[test]
    fn tight_bays_shorten_the_headers() {
        // The V8 fills the roadster's bay from side to side: short primaries only. A
        // four leaves room for long ones.
        let roadster = &CHASSIS[chassis("Linnet")];
        let (v8, four) = (
            roadster.fit(family("M62")).unwrap(),
            roadster.fit(family("B16")).unwrap(),
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
        let coupe = &CHASSIS[chassis("Tarn")];
        let boxes: Vec<&str> = coupe.gearboxes().map(|g| GEARBOXES[g].name).collect();
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
        assert!(!needs_adapter(coupe.gearbox, family("S30")));
        assert!(needs_adapter(coupe.gearbox, family("R20")));
        // The track-day special has the R20 on the 5-speed from the factory.
        assert!(!needs_adapter(2, family("R20")));
    }
}
