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

use crate::body::{Body, Wheels};
use crate::catalogue::{Catalogue, Paint, Shared, Suspension};
use crate::physics::drivetrain::Drive;
use crate::physics::engine::parts::Parts;
use crate::physics::fuel::Tank;
use crate::wheels::{Wheel, WheelRoom};

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

#[derive(Clone, Debug, PartialEq)]
pub struct EngineBay {
    pub place: String,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Gearbox {
    pub name: String,
    pub kind: GearboxKind,
    pub gears: Vec<f32>,
    pub reverse: f32,
    pub mass: f32,
    /// Most torque its stock clutch passes (N·m).
    pub clutch_torque: f32,
    /// Most torque its gears take for good (N·m at the input shaft): more wears them.
    pub torque: f32,
    /// Time the clutch stays open for a gear change, and takes to close again (s).
    pub shift_time: f32,
    pub engage_time: f32,
}

/// The cold tire pressures that can be set (bar).
pub const PRESSURES: std::ops::RangeInclusive<f32> = 1.2..=3.5;
/// The toe that can be set (degrees, toe-in positive), and the factory's front and rear.
pub const TOES: std::ops::RangeInclusive<f32> = -1.0..=1.0;
pub const FACTORY_TOE: [f32; 2] = [0.0, 0.25];
/// How much stronger or weaker the brakes can be set than the factory's (a share of its
/// torque), and how far their balance can be moved from the factory's (a share of the
/// braking at the front).
pub const BRAKE_FORCES: std::ops::RangeInclusive<f32> = 0.7..=1.3;
pub const BRAKE_BALANCES: std::ops::RangeInclusive<f32> = -0.1..=0.1;

#[derive(Clone, Debug, PartialEq)]
pub struct Chassis {
    pub name: String,
    pub class: String,
    pub about: String,
    /// Sprung mass without engine, gearbox and seats (kg), its centre behind the front axle (m),
    /// and the height of the whole car's centre of mass (m).
    pub mass: f32,
    pub mass_at: f32,
    pub cg_height: f32,
    pub wheelbase: f32,
    /// Front and rear: track (m), the factory's rims and tires, the room in the wheel
    /// wells, and the unsprung mass of a corner without its wheel and brake (hub, upright;
    /// kg).
    pub track: [f32; 2],
    pub wheels: [Wheel; 2],
    pub room: [WheelRoom; 2],
    pub hub: [f32; 2],
    /// Ride frequency front and rear (Hz), and damping ratio.
    pub ride: [f32; 2],
    pub damping: f32,
    /// The anti-roll bars made for the front and the rear.
    pub bars: [RollBars; 2],
    /// Static camber front and rear (degrees).
    pub camber: [f32; 2],
    /// Cold tire pressures front and rear (bar): what suits the factory tires under the
    /// car's weight (see `TireParams::suited_pressure`).
    pub pressure: [f32; 2],
    pub max_steer: f32,
    pub drag_area: f32,
    pub downforce_area: [f32; 2],
    /// The fuel tank.
    pub tank: Tank,
    pub bay: EngineBay,
    pub drive: Drive,
    /// Limited-slip differentials from the factory.
    pub limited_slip: bool,
    /// The factory's gearbox (in the catalogue's `gearboxes`) and final drive.
    pub gearbox: usize,
    pub final_drive: f32,
    /// Codes of the engines the factory offers, the base one first.
    pub engines: Vec<String>,
    /// The one paint (in the catalogue's `paints`, a dull factory one) it leaves the
    /// factory in.
    pub paint: usize,
    pub body: Body,
    /// The driver's eyes: behind the nose, above the ground, and left of the centreline.
    pub eye: [f32; 3],
    /// How well the cabin keeps out sound, 0 (stripped out, or no roof) to 1 (a quiet
    /// saloon): a share of what comes through the body.
    pub insulation: f32,
    /// How often it turns up in traffic, relative to the others.
    pub traffic: f32,
    /// Where it carries loose parts (see `cargo`).
    pub hold: crate::cargo::Hold,
}

/// The anti-roll bars made for one axle of a chassis: solid bars of these diameters (mm),
/// the factory's among them. They all sit on the same links, so a bar's roll stiffness
/// goes with its diameter to the fourth.
#[derive(Clone, Debug, PartialEq)]
pub struct RollBars {
    pub sizes: Vec<u8>,
    /// The factory's (in `sizes`).
    pub factory: usize,
    /// The roll stiffness a 20 mm bar gives the axle (N·m per degree of body roll).
    pub rate: f32,
}

impl RollBars {
    /// The roll stiffness bar `bar` (in `sizes`) gives the axle (N·m per degree).
    pub fn stiffness(&self, bar: usize) -> f32 {
        self.rate * (f32::from(self.sizes[bar]) / 20.0).powi(4)
    }

    /// A bar (in `sizes`) by its diameter and stiffness.
    pub fn label(&self, bar: usize) -> String {
        format!("{} mm ({:.0} N·m/°)", self.sizes[bar], self.stiffness(bar))
    }

    /// The bar (in `sizes`) of `diameter` (mm), if there's one.
    pub fn find(&self, diameter: u8) -> Option<usize> {
        self.sizes.iter().position(|&size| size == diameter)
    }
}

/// Anti-roll bars of `sizes` (mm), the factory's at `factory` in them, and a 20 mm one
/// giving the axle `rate` (N·m per degree of roll).
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
            radius: self.wheels.map(|wheel| wheel.tire.radius()),
            width: self.wheels.map(|wheel| wheel.tire.width_m()),
        }
    }

    /// Unsprung mass of a front or rear corner (kg) on the factory's wheels, without the
    /// brake.
    pub fn unsprung(&self, axle: usize) -> f32 {
        self.hub[axle] + self.wheels[axle].mass()
    }

    /// How an engine of `family` (in `catalogue`'s `families`) fits, or why it doesn't:
    /// it needs the space, and nothing else.
    pub fn fit(&self, catalogue: &Catalogue, family: usize) -> Result<Fit, Misfit> {
        let engine = &catalogue.families[family];
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
            swap: !catalogue
                .factory_families(self)
                .any(|offered| offered == family),
            header_room: (bay.header_room * share * 100.0).round() / 100.0,
        })
    }
}

/// A car as built, apart from its engine (which is its `Parts`): the chassis and what's
/// fitted to it, each by its place in its catalogue.
#[derive(Clone, Debug, PartialEq)]
pub struct CarBuild {
    /// What everything's in.
    pub catalogue: Shared,
    pub chassis: usize,
    pub paint: usize,
    pub gearbox: usize,
    pub clutch: usize,
    pub final_drive: f32,
    pub limited_slip: bool,
    pub suspension: usize,
    /// The anti-roll bars front and rear (in the chassis' `bars`).
    pub bars: [usize; 2],
    /// Front and rear.
    pub wheels: [Wheel; 2],
    /// Static camber (degrees) and cold tire pressure (bar), front and rear.
    pub camber: [f32; 2],
    pub pressure: [f32; 2],
    /// Toe-in (degrees), front and rear.
    pub toe: [f32; 2],
    /// The brakes' torque as a share of the factory's, and their balance moved this far
    /// to the front from the factory's (a share of the braking).
    pub brake_force: f32,
    pub brake_balance: f32,
    /// The ride height the springs' perches set, front and rear: mm over the design height
    /// (less than nothing: lower). The springs' knob, where they have one.
    pub ride_height: [f32; 2],
    /// The rear wing's angle (degrees), if there's a wing: its knob.
    pub wing: f32,
}

impl CarBuild {
    /// The chassis as it leaves the factory (of the built-in catalogue).
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn factory(chassis: usize) -> Self {
        Self::factory_in(&crate::catalogue::builtin(), chassis)
    }

    /// `catalogue`'s `chassis` as it leaves the factory.
    pub fn factory_in(catalogue: &Shared, chassis: usize) -> Self {
        let made = &catalogue.chassis[chassis];
        Self {
            catalogue: catalogue.clone(),
            chassis,
            paint: made.paint,
            gearbox: made.gearbox,
            clutch: 0,
            final_drive: made.final_drive,
            limited_slip: made.limited_slip,
            suspension: 0,
            bars: [0, 1].map(|axle| made.bars[axle].factory),
            wheels: made.wheels,
            camber: made.camber,
            pressure: made.pressure,
            toe: FACTORY_TOE,
            brake_force: 1.0,
            brake_balance: 0.0,
            ride_height: [0.0; 2],
            wing: 0.0,
        }
    }

    /// The camber (degrees) the suspension kit is aligned to on `axle`.
    pub fn kit_camber(&self, axle: usize) -> f32 {
        self.chassis().camber[axle] - self.suspension().camber
    }

    /// The cambers (degrees) that can be set on `axle` with the suspension kit.
    pub fn camber_range(&self, axle: usize) -> std::ops::RangeInclusive<f32> {
        let (kit, reach) = (self.kit_camber(axle), self.suspension().reach);
        kit - reach..=kit + reach
    }

    /// The suspension kit.
    pub fn suspension(&self) -> &Suspension {
        &self.catalogue.suspensions[self.suspension]
    }

    /// Fits the suspension kit `kit` (in the catalogue's `suspensions`), aligned as it
    /// comes.
    pub fn set_suspension(&mut self, kit: usize) {
        self.suspension = kit;
        self.camber = [0, 1].map(|axle| self.kit_camber(axle));
    }

    /// The same car on the suspension kit `kit`.
    pub fn with_suspension(mut self, kit: usize) -> Self {
        self.set_suspension(kit);
        self
    }

    /// Keeps the camber within what the suspension kit can be set to, the tire pressures
    /// within `PRESSURES`, and the toe and brakes within theirs.
    pub fn clamp_alignment(&mut self) {
        for axle in 0..2 {
            let range = self.camber_range(axle);
            self.camber[axle] = self.camber[axle].clamp(*range.start(), *range.end());
            let pressure = self.pressure[axle];
            self.pressure[axle] = pressure.clamp(*PRESSURES.start(), *PRESSURES.end());
            self.toe[axle] = self.toe[axle].clamp(*TOES.start(), *TOES.end());
        }
        (self.brake_force, self.brake_balance) = (
            (self.brake_force).clamp(*BRAKE_FORCES.start(), *BRAKE_FORCES.end()),
            (self.brake_balance).clamp(*BRAKE_BALANCES.start(), *BRAKE_BALANCES.end()),
        );
    }

    pub fn chassis(&self) -> &Chassis {
        &self.catalogue.chassis[self.chassis]
    }

    /// Why the wheels don't go on the chassis, if they don't.
    pub fn wheel_misfit(&self) -> Option<String> {
        let chassis = self.chassis();
        (0..2).find_map(|axle| {
            let why = chassis.room[axle].misfit(&self.wheels[axle])?;
            Some(format!("{}: {why}", ["Front", "Rear"][axle]))
        })
    }

    /// The same car on tires of `compound` (in the catalogue's `compounds`) all round.
    pub fn on_compound(mut self, compound: usize) -> Self {
        for wheel in &mut self.wheels {
            wheel.compound = compound;
        }
        self
    }

    pub fn gearbox(&self) -> &Gearbox {
        &self.catalogue.gearboxes[self.gearbox]
    }

    /// The tire compound's rubber on `axle`.
    pub fn rubber(&self, axle: usize) -> &crate::physics::tire::Compound {
        &self.catalogue.compounds[self.wheels[axle].compound].rubber
    }

    /// The paint (in the catalogue's `paints`).
    pub fn paint(&self) -> &Paint {
        &self.catalogue.paints[self.paint]
    }

    pub fn color(&self) -> Color {
        let [r, g, b] = self.paint().color;
        Color::srgb(r, g, b)
    }

    /// E.g. `205/55 R16 street on 16×7.0"`, the wheels on `axle`.
    pub fn wheel_label(&self, axle: usize) -> String {
        self.catalogue.wheel_label(&self.wheels[axle])
    }

    /// The factory's engine `option` for the chassis (the base one if there's no such),
    /// fitted to it.
    pub fn factory_engine(&self, option: usize) -> Parts {
        let engines = &self.chassis().engines;
        let code = engines.get(option).unwrap_or(&engines[0]);
        let parts = Parts::factory_in(&self.catalogue, code).expect("a factory engine");
        self.install(parts).expect("factory engines fit")
    }

    /// `parts` as they sit in this car (with the room there is for their exhaust
    /// manifold), or why the engine doesn't fit.
    pub fn install(&self, mut parts: Parts) -> Result<Parts, Misfit> {
        parts.header_room = self
            .chassis()
            .fit(&self.catalogue, parts.family)?
            .header_room;
        Ok(parts)
    }
}
