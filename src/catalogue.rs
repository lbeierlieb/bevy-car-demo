//! What cars and parts there are: chassis, gearboxes, clutches, suspension kits, paints,
//! tire compounds, engine families with their factory versions, and the engine parts by
//! slot. The server decides it and tells every player on joining; the game's code knows
//! no car by itself, only how to build and simulate whatever a catalogue describes.
//!
//! A car refers to its catalogue (`CarBuild::catalogue`, `Parts::catalogue`) and to the
//! entries in it by their place, which is also how they go over the network: both ends
//! have the same catalogue.
//!
//! The catalogue made with the game is `builtin`: the server's, unless it's given another
//! (see `chassis`, `engines` and `parts` for its entries).

use std::sync::{Arc, LazyLock};

use crate::chassis::{Chassis, Gearbox};
use crate::physics::engine::families::Family;
use crate::physics::engine::parts::Slot;
use crate::physics::tire::Compound;

mod chassis;
mod engines;
mod parts;
mod wire;

pub use chassis::DEFAULT_CHASSIS;
pub use parts::{HEADERS, INTERCOOLER, PISTONS, TURBO};

/// A catalogue shared by the cars built from it.
pub type Shared = Arc<Catalogue>;

#[derive(Clone)]
pub struct Catalogue {
    pub chassis: Vec<Chassis>,
    pub gearboxes: Vec<Gearbox>,
    pub clutches: Vec<Clutch>,
    pub suspensions: Vec<Suspension>,
    /// The paint shops' (`Paint::shop`) and the dull ones cars leave the factory in.
    pub paints: Vec<Paint>,
    pub compounds: Vec<TireCompound>,
    pub families: Vec<Family>,
    /// The kinds of engine part, each with the parts there are of it (the first stock).
    pub slots: Vec<Slot>,
    /// The chassis a player starts in (with its base engine).
    pub start: usize,
    /// Tells catalogues apart: two with the same fingerprint are the same.
    fingerprint: u64,
}

/// A clutch: how much more torque than the gearbox's stock one it passes.
#[derive(Clone, Debug, PartialEq)]
pub struct Clutch {
    pub name: String,
    pub torque: f32,
}

/// A suspension kit: the ride frequency and damping relative to the chassis' own, the
/// extra negative camber it's aligned to, and how far either side of that the camber can
/// be set (degrees).
#[derive(Clone, Debug, PartialEq)]
pub struct Suspension {
    pub name: String,
    pub ride: f32,
    pub damping: f32,
    pub camber: f32,
    pub reach: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
    pub name: String,
    /// sRGB.
    pub color: [f32; 3],
    /// Paint shops have it (else it's one cars leave the factory in).
    pub shop: bool,
}

/// A tire compound as it's offered, and its rubber.
#[derive(Clone, Debug, PartialEq)]
pub struct TireCompound {
    pub name: String,
    pub about: String,
    pub rubber: Compound,
}

impl Catalogue {
    /// A catalogue of these entries.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chassis: Vec<Chassis>,
        gearboxes: Vec<Gearbox>,
        clutches: Vec<Clutch>,
        suspensions: Vec<Suspension>,
        paints: Vec<Paint>,
        compounds: Vec<TireCompound>,
        families: Vec<Family>,
        slots: Vec<Slot>,
        start: usize,
    ) -> Self {
        let mut catalogue = Self {
            chassis,
            gearboxes,
            clutches,
            suspensions,
            paints,
            compounds,
            families,
            slots,
            start,
            fingerprint: 0,
        };
        catalogue.fingerprint = catalogue.hash();
        catalogue
    }

    /// FNV-1a over everything in it.
    fn hash(&self) -> u64 {
        let text = format!(
            "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{}",
            self.chassis,
            self.gearboxes,
            self.clutches,
            self.suspensions,
            self.paints,
            self.compounds,
            self.families,
            self.slots,
            self.start
        );
        text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// The paints the paint shops have (in `paints`).
    pub fn shop_paints(&self) -> Vec<usize> {
        (0..self.paints.len())
            .filter(|&paint| self.paints[paint].shop)
            .collect()
    }

    /// The family and version of the engine with the factory code `code`.
    pub fn find_engine(&self, code: &str) -> Option<(usize, usize)> {
        self.families.iter().enumerate().find_map(|(f, family)| {
            let version = family.versions.iter().position(|v| v.code == code)?;
            Some((f, version))
        })
    }

    /// The chassis called `name`.
    pub fn find_chassis(&self, name: &str) -> Option<usize> {
        self.chassis.iter().position(|chassis| chassis.name == name)
    }

    /// The families of the engines the factory offers in `chassis`.
    pub fn factory_families<'a>(&'a self, chassis: &'a Chassis) -> impl Iterator<Item = usize> {
        (chassis.engines.iter()).filter_map(|code| Some(self.find_engine(code)?.0))
    }

    /// The gearboxes that suit `chassis`' layout (in `gearboxes`).
    pub fn gearboxes_for(&self, chassis: &Chassis) -> impl Iterator<Item = usize> {
        let kind = self.gearboxes[chassis.gearbox].kind;
        (0..self.gearboxes.len()).filter(move |&gearbox| self.gearboxes[gearbox].kind == kind)
    }

    /// Whether `gearbox` needs an adapter plate to bolt to an engine of `family`: it does
    /// unless some factory car pairs the two.
    pub fn needs_adapter(&self, gearbox: usize, family: usize) -> bool {
        !self.chassis.iter().any(|chassis| {
            chassis.gearbox == gearbox && self.factory_families(chassis).any(|f| f == family)
        })
    }
}

impl PartialEq for Catalogue {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl std::fmt::Debug for Catalogue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Catalogue({:016x})", self.fingerprint)
    }
}

/// The catalogue made with the game.
pub fn builtin() -> Shared {
    static BUILTIN: LazyLock<Shared> = LazyLock::new(|| {
        Arc::new(Catalogue::new(
            chassis::chassis(),
            chassis::gearboxes(),
            chassis::clutches(),
            chassis::suspensions(),
            chassis::paints(),
            parts::compounds(),
            engines::families(),
            parts::slots(),
            DEFAULT_CHASSIS,
        ))
    });
    BUILTIN.clone()
}

impl Catalogue {
    /// E.g. `205/55 R16 street on 16×7.0"`.
    pub fn wheel_label(&self, wheel: &crate::wheels::Wheel) -> String {
        format!(
            "{} {} on {}",
            wheel.tire.label(),
            self.compounds[wheel.compound].name.to_lowercase(),
            wheel.rim.label()
        )
    }
}

/// The catalogue the world's cars are built from: the server's.
#[derive(bevy::prelude::Resource, Clone, Debug, PartialEq)]
pub struct Current(pub Shared);

impl std::ops::Deref for Current {
    type Target = Shared;

    fn deref(&self) -> &Shared {
        &self.0
    }
}
