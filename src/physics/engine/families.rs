//! The engines there are: families of engines that share a design (and so their parts,
//! their size and what bolts to them), each in a few factory versions. A version is a
//! block (bore, stroke, cams and the rest of the stock hardware) and the parts the factory
//! fits to it, such as a turbo. They're independent of the cars: any engine goes into any
//! car it fits into. The catalogue says which there are (see `catalogue::engines` for
//! the built-in ones).

use super::hardware::{CamProfile, EngineHardware, Manifold, ValveLayout, ValveTrain};
use super::parts::Preset;

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
    /// A 90° V10 with a common pin for each pair of cylinders: firings 54° and 90° of
    /// crank apart in turn, the banks in turn (each bank fires evenly, the two out of
    /// step).
    OddFireV10,
}

impl Layout {
    /// Every layout (the order is their number on the network).
    pub const ALL: [Layout; 11] = [
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
        Layout::OddFireV10,
    ];

    #[cfg(test)]
    pub fn cylinders(self) -> u32 {
        match self {
            Layout::Inline3 => 3,
            Layout::Inline4 | Layout::Flat4 => 4,
            Layout::Inline6 | Layout::Flat6 | Layout::V6 => 6,
            Layout::V8 | Layout::FlatPlaneV8 => 8,
            Layout::V10 | Layout::OddFireV10 => 10,
            Layout::V12 => 12,
        }
    }

    /// Whether its cylinders stand in two banks in a V.
    pub fn is_vee(self) -> bool {
        matches!(
            self,
            Layout::V6
                | Layout::V8
                | Layout::FlatPlaneV8
                | Layout::V10
                | Layout::V12
                | Layout::OddFireV10
        )
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
            Layout::OddFireV10 => "odd-fire V10",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    pub name: String,
    pub about: String,
    pub layout: Layout,
    /// Length along the crank, width and height with its ancillaries (m), and its mass
    /// without a turbo (kg).
    pub length: f32,
    pub width: f32,
    pub height: f32,
    pub mass: f32,
    pub versions: Vec<Version>,
    /// The exhaust manifolds made for the family (the first is the factory's).
    pub manifolds: Vec<ManifoldDef>,
    /// The cylinder heads made for the family (each version's factory head is its
    /// preset's): any goes onto any of its blocks.
    pub heads: Vec<HeadDef>,
}

/// A cylinder head made for an engine family: its valves, their springs, the cams over
/// them (or, on a pushrod engine, the cam working them) and how its ports flow. Its valves
/// keep their size on a block of another bore: they're relative to the bore it's made for.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadDef {
    /// Stable, for saved cars and items: e.g. `b16r.head`.
    pub id: String,
    pub name: String,
    /// The bore it's made for (m).
    pub bore: f32,
    pub cams: Vec<CamProfile>,
    pub valves: ValveTrain,
    pub valvetrain: ValveLayout,
    pub port_flow: f32,
}

impl HeadDef {
    /// The head of `block` (a version's factory hardware).
    pub fn of(id: &str, name: &str, block: &EngineHardware) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            bore: block.bore,
            cams: block.cams.clone(),
            valves: block.valves,
            valvetrain: block.valvetrain,
            port_flow: block.port_flow,
        }
    }

    /// This head on `hardware`'s block, its valves the size they are on its own bore.
    pub fn fit(&self, hardware: &mut EngineHardware) {
        hardware.cams = self.cams.clone();
        hardware.valves = self.valves;
        hardware.port_flow = self.port_flow;
        let mut layout = self.valvetrain;
        if self.bore != hardware.bore {
            let k = self.bore / hardware.bore;
            layout.intake_diameter = super::parts::round_to(layout.intake_diameter * k, 1e-4);
            layout.exhaust_diameter = super::parts::round_to(layout.exhaust_diameter * k, 1e-4);
        }
        hardware.valvetrain = layout;
    }
}

/// An exhaust manifold made for an engine family: how it gathers the exhaust, and whether
/// each of its branches has a mount for a turbo (a manifold without mounts can't carry
/// one). One item, even when it's a pair (a V engine's, one per bank).
#[derive(Clone, Debug, PartialEq)]
pub struct ManifoldDef {
    /// Stable, for saved cars and items: e.g. `b.headers.long-tube`.
    pub id: String,
    pub name: String,
    pub manifold: Manifold,
    pub mounts: bool,
    /// Cast in iron (stubby primaries into a log) rather than bent of tubes: only how it
    /// looks.
    pub cast: bool,
}

impl ManifoldDef {
    /// How many turbos it carries when filled.
    pub fn mounts(&self) -> u32 {
        if self.mounts {
            self.manifold.branches
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    /// The factory's code, e.g. "W20T".
    pub code: String,
    pub name: String,
    /// The block as the factory builds it, before any parts.
    pub block: EngineHardware,
    /// The parts the factory fits.
    pub preset: Preset,
}

impl Family {
    /// Mass of an engine of this family as built (kg): turbos with their plumbing and
    /// intercooler add to it (each sized for its share of the engine).
    pub fn built_mass(&self, hardware: &EngineHardware) -> f32 {
        let turbo = if hardware.turbo.is_some() {
            let turbos = hardware.turbos.max(1) as f32;
            turbos * (12.0 + 6.0 * relative_size(hardware) / turbos)
        } else {
            0.0
        };
        self.mass + turbo
    }
}

/// Displacement relative to the 2-litre four's, which the parts around the cylinders (and
/// turbos) are sized by.
pub fn relative_size(hardware: &EngineHardware) -> f32 {
    hardware.displacement() / EngineHardware::default().displacement()
}
