//! Parts to build an engine from, as a garage stocks them: a slot per kind of part, each
//! with a few to choose from, fitted onto a factory version of an engine family. A part is
//! data, a list of `Effect`s on the engine's hardware, so a catalogue can describe parts
//! the game's code has never heard of, as long as they do what effects can. The effects
//! are relative to the engine they go onto (a stroker crank for any block, turbos sized
//! for its displacement). Some slots change the cylinder side, so fitting their parts
//! means baking the maps again; the rest act live.

use super::ecu::EcuModel;
use super::families::{Family, Version, relative_size};
use super::hardware::{
    CamProfile, Compressor, Diverter, EngineHardware, Intercooler, Turbine, Turbo, Wastegate,
};
use crate::catalogue::{Catalogue, Shared};

/// A kind of part, and the parts there are of it (the first is stock).
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub name: String,
    pub about: String,
    pub parts: Vec<Part>,
    /// What the slot's part comes to on an engine, e.g. "92 mm, 2.1 l" for a crank.
    pub readout: Readout,
    /// Its parts only do something with a turbo (an intercooler).
    pub needs_turbo: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub effects: Vec<Effect>,
}

/// What a part does to an engine's hardware.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Changes a number, then rounds it to a whole number of `step`s (a power of ten;
    /// none for 0). A number that's missing (no turbo, no catalyst) stays missing.
    Change {
        field: Field,
        change: Change,
        step: f32,
    },
    /// Takes the catalyst or the silencer out.
    Remove(Field),
    /// Regrinds every cam profile.
    Regrind(Regrind),
    /// Cams that switch between two profiles: a single profile becomes `mild` and `wild`
    /// made from it; on an engine switching already, the wild profile becomes `wilder`.
    Switching {
        mild: Regrind,
        wild: Regrind,
        wilder: Regrind,
    },
    /// A turbo kit (see `TurboKit`).
    Turbo(TurboKit),
    /// A supercharger kit (see `SuperchargerKit`).
    Supercharger(SuperchargerKit),
}

/// A number in the hardware a part can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    CompressionRatio,
    Stroke,
    RodLength,
    RodStrength,
    RodMass,
    PistonMass,
    PistonToughness,
    HeadGasket,
    IntakePhasing,
    ExhaustPhasing,
    SeatForce,
    SpringRate,
    ValveMass,
    PortFlow,
    FilterArea,
    ThrottleDiameter,
    RunnerLength,
    TractVolume,
    HeaderLength,
    PipeArea,
    CatArea,
    SilencerArea,
    IntercoolerEffectiveness,
    IntercoolerArea,
    RadiatorArea,
    RadiatorEffectiveness,
    Inertia,
}

impl Field {
    pub const ALL: [Field; 27] = [
        Field::CompressionRatio,
        Field::Stroke,
        Field::RodLength,
        Field::RodStrength,
        Field::RodMass,
        Field::PistonMass,
        Field::PistonToughness,
        Field::HeadGasket,
        Field::IntakePhasing,
        Field::ExhaustPhasing,
        Field::SeatForce,
        Field::SpringRate,
        Field::ValveMass,
        Field::PortFlow,
        Field::FilterArea,
        Field::ThrottleDiameter,
        Field::RunnerLength,
        Field::TractVolume,
        Field::HeaderLength,
        Field::PipeArea,
        Field::CatArea,
        Field::SilencerArea,
        Field::IntercoolerEffectiveness,
        Field::IntercoolerArea,
        Field::RadiatorArea,
        Field::RadiatorEffectiveness,
        Field::Inertia,
    ];

    /// The number in `h`, if it has it.
    fn of(self, h: &mut EngineHardware) -> Option<&mut f32> {
        Some(match self {
            Field::CompressionRatio => &mut h.compression_ratio,
            Field::Stroke => &mut h.stroke,
            Field::RodLength => &mut h.rod_length,
            Field::RodStrength => &mut h.bottom_end.rod_strength,
            Field::RodMass => &mut h.bottom_end.rod_mass,
            Field::PistonMass => &mut h.bottom_end.piston_mass,
            Field::PistonToughness => &mut h.bottom_end.piston_toughness,
            Field::HeadGasket => &mut h.head_gasket,
            Field::IntakePhasing => &mut h.intake_phasing,
            Field::ExhaustPhasing => &mut h.exhaust_phasing,
            Field::SeatForce => &mut h.valves.seat_force,
            Field::SpringRate => &mut h.valves.rate,
            Field::ValveMass => &mut h.valves.mass,
            Field::PortFlow => &mut h.port_flow,
            Field::FilterArea => &mut h.intake.filter_area,
            Field::ThrottleDiameter => &mut h.intake.throttle_diameter,
            Field::RunnerLength => &mut h.intake.runner_length,
            Field::TractVolume => &mut h.intake.tract_volume,
            Field::HeaderLength => &mut h.exhaust.header_length,
            Field::PipeArea => &mut h.exhaust.pipe_area,
            Field::CatArea => h.exhaust.cat_area.as_mut()?,
            Field::SilencerArea => h.exhaust.silencer_area.as_mut()?,
            Field::IntercoolerEffectiveness => &mut h.turbo.as_mut()?.intercooler.effectiveness,
            Field::IntercoolerArea => &mut h.turbo.as_mut()?.intercooler.area,
            Field::RadiatorArea => &mut h.cooling.radiator_area,
            Field::RadiatorEffectiveness => &mut h.cooling.radiator_effectiveness,
            Field::Inertia => &mut h.inertia,
        })
    }
}

/// How a number changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Times(f32),
    Over(f32),
    Plus(f32),
    To(f32),
    AtLeast(f32),
}

impl Change {
    fn apply(self, x: f32) -> f32 {
        match self {
            Change::Times(by) => x * by,
            Change::Over(by) => x / by,
            Change::Plus(by) => x + by,
            Change::To(value) => value,
            Change::AtLeast(value) => x.max(value),
        }
    }
}

/// A cam profile reground: `duration` more crank degrees, the lobes `separation` cam
/// degrees further apart, and `lift` times the lift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Regrind {
    pub duration: f32,
    pub separation: f32,
    pub lift: f32,
}

impl Regrind {
    fn apply(&self, cam: &CamProfile) -> CamProfile {
        CamProfile {
            intake_duration: cam.intake_duration + self.duration,
            exhaust_duration: cam.exhaust_duration + self.duration,
            lobe_separation: cam.lobe_separation + self.separation,
            lift: round_to(cam.lift * self.lift, 1e-4),
        }
    }
}

/// A turbo kit: the turbo with compressor and turbine wheels of these diameters (m), the
/// turbine's flow area (m²), the shaft's inertia (kg·m²) and the wastegate's flow area
/// (m²; a small turbine needs a big one to let the excess exhaust past), a diverter valve,
/// an intercooler, and the piping to it (more volume ahead of the throttle). The numbers
/// are for a 2-litre engine: the kit is scaled to the engine's displacement (flow areas
/// with it, so wheel diameters with its root).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurboKit {
    pub compressor: f32,
    pub turbine: f32,
    pub turbine_area: f32,
    pub inertia: f32,
    pub wastegate: f32,
}

impl TurboKit {
    fn fit(&self, hardware: &mut EngineHardware) {
        let size = relative_size(hardware);
        let wheel = size.sqrt();
        hardware.intake.tract_volume *= 3.0;
        hardware.turbo = Some(Turbo {
            compressor: Compressor {
                diameter: self.compressor * wheel,
                head: 0.7,
                choke: 0.29,
                efficiency: 0.75,
            },
            turbine: Turbine {
                diameter: self.turbine * wheel,
                area: self.turbine_area * size,
                efficiency: 0.7,
            },
            inertia: self.inertia * wheel.powi(5),
            wastegate: Wastegate {
                area: self.wastegate * size,
                spring: 0.6e5,
                span: 0.12e5,
            },
            intercooler: Intercooler {
                effectiveness: 0.7,
                area: 3e-3 * size,
            },
            diverter: Some(Diverter {
                area: 5e-4 * size,
                opens: 0.3e5,
                span: 0.2e5,
            }),
        });
    }
}

/// A supercharger kit for a 2-litre engine, scaled to the engine's displacement like a
/// turbo kit: a Roots blower's volume per turn of its rotors (m³) and its leak area (m²),
/// or a centrifugal one's impeller diameter (m); the pulleys' (and step-up gear's) ratio;
/// the belt's drag (N·m, and per 1000 rpm). The piping adds volume ahead of the throttle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SuperchargerKit {
    pub roots: bool,
    pub displacement: f32,
    pub leak: f32,
    pub impeller: f32,
    pub ratio: f32,
    pub drag: [f32; 2],
}

impl SuperchargerKit {
    fn fit(&self, hardware: &mut EngineHardware) {
        let size = relative_size(hardware);
        hardware.intake.tract_volume *= if self.roots { 1.5 } else { 2.5 };
        hardware.supercharger = Some(super::hardware::Supercharger {
            roots: self.roots,
            displacement: self.displacement * size,
            leak: self.leak * size,
            compressor: super::supercharger::impeller(self.impeller * size.sqrt()),
            ratio: self.ratio,
            drag: self.drag.map(|drag| drag * size),
            bypass: Diverter {
                area: 6e-4 * size,
                opens: 0.3e5,
                span: 0.2e5,
            },
        });
    }
}

impl Effect {
    pub fn apply(&self, h: &mut EngineHardware) {
        match self {
            Effect::Change {
                field,
                change,
                step,
            } => {
                if let Some(x) = field.of(h) {
                    let changed = change.apply(*x);
                    *x = if *step > 0.0 {
                        round_to(changed, *step)
                    } else {
                        changed
                    };
                }
            }
            Effect::Remove(field) => match field {
                Field::CatArea => h.exhaust.cat_area = None,
                Field::SilencerArea => h.exhaust.silencer_area = None,
                _ => {}
            },
            Effect::Regrind(regrind) => {
                for cam in &mut h.cams {
                    *cam = regrind.apply(cam);
                }
            }
            Effect::Switching { mild, wild, wilder } => {
                h.cams = match h.cams[..] {
                    [cam] => vec![mild.apply(&cam), wild.apply(&cam)],
                    [first, second, ..] => vec![first, wilder.apply(&second)],
                    [] => vec![],
                };
            }
            Effect::Turbo(kit) => kit.fit(h),
            Effect::Supercharger(kit) => kit.fit(h),
        }
    }
}

/// What a slot's part comes to on an engine, shown with its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readout {
    Nothing,
    Compression,
    Crank,
    RodLength,
    HeadGasket,
    Cams,
    ValveFloat,
    Phasers,
    Runners,
    Headers,
    Throttle,
    Pipe,
    Flywheel,
    PortFlow,
}

impl Readout {
    pub const ALL: [Readout; 14] = [
        Readout::Nothing,
        Readout::Compression,
        Readout::Crank,
        Readout::RodLength,
        Readout::HeadGasket,
        Readout::Cams,
        Readout::ValveFloat,
        Readout::Phasers,
        Readout::Runners,
        Readout::Headers,
        Readout::Throttle,
        Readout::Pipe,
        Readout::Flywheel,
        Readout::PortFlow,
    ];

    /// E.g. "92 mm, 2.1 l" for a crank.
    pub fn of(self, h: &EngineHardware) -> String {
        match self {
            Readout::Nothing => String::new(),
            Readout::Compression => format!("{:.1}:1", h.compression_ratio),
            Readout::Crank => {
                let litres = h.displacement() * 1e3;
                format!("{:.1} mm, {litres:.1} l", h.stroke * 1e3)
            }
            Readout::RodLength => format!("{:.0} mm", h.rod_length * 1e3),
            Readout::HeadGasket => format!("{:.0} bar", h.head_gasket / 1e5),
            Readout::Cams => {
                let durations: Vec<String> = (h.cams.iter())
                    .map(|cam| format!("{:.0}°", cam.intake_duration))
                    .collect();
                let lift = h.cams.last().map_or(0.0, |cam| cam.lift * 1e3);
                format!("{}, {lift:.1} mm", durations.join(" / "))
            }
            Readout::ValveFloat => match h.cams.last() {
                Some(cam) => format!(
                    "float at {:.0} rpm",
                    round_to(h.valves.float_rpm(cam), 100.0)
                ),
                None => String::new(),
            },
            Readout::Phasers => match (h.intake_phasing, h.exhaust_phasing) {
                (0.0, 0.0) => "none".to_string(),
                (intake, 0.0) => format!("intake {intake:.0}°"),
                (intake, exhaust) => format!("{intake:.0}° / {exhaust:.0}°"),
            },
            Readout::Runners => format!("{:.0} mm runners", h.intake.runner_length * 1e3),
            Readout::Headers => format!("{:.0} mm", h.exhaust.header_length * 1e3),
            Readout::Throttle => format!("{:.0} mm", h.intake.throttle_diameter * 1e3),
            Readout::Pipe => {
                let diameter = (h.exhaust.pipe_area * 4.0 / std::f32::consts::PI).sqrt();
                format!("{:.0} mm", diameter * 1e3)
            }
            Readout::Flywheel => format!("{:.2} kg·m²", h.inertia),
            Readout::PortFlow => format!("{:.0} % flow", h.port_flow * 100.0),
        }
    }
}

/// `value` rounded to a whole number of `step`s (a power of ten), to the nearest f32
/// there, so that 0.6 m comes out as 0.6.
pub fn round_to(value: f32, step: f32) -> f32 {
    if step >= 1.0 {
        (value / step).round() * step
    } else {
        let per = (1.0 / step).round();
        (value * per).round() / per
    }
}

/// What the factory fits to a version of an engine: a part per slot (by its place in the
/// slot), the wastegate's spring and the ECU.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub chosen: Vec<usize>,
    pub boost: f32,
    pub ecu: EcuModel,
}

impl Preset {
    /// The stock part in every one of `slots` slots.
    pub fn stock(slots: usize) -> Self {
        Self {
            chosen: vec![0; slots],
            boost: 0.6,
            ecu: EcuModel::Stock,
        }
    }
}

/// The parts an engine is built from: the factory version of an engine family it started
/// as, a part per slot, the cam gears' timing, the wastegate's spring, and the ECU.
#[derive(Clone, Debug, PartialEq)]
pub struct Parts {
    /// What the family and the parts are in.
    pub catalogue: Shared,
    /// The engine family (in the catalogue's `families`) and its version.
    pub family: usize,
    pub version: usize,
    /// The part fitted in each of the catalogue's `slots`, by its place in the slot.
    pub chosen: Vec<usize>,
    /// How far the block is bored over (a step of `items::OVERSIZES`).
    pub bore: usize,
    /// Cam timing (crank degrees; intake advanced, exhaust retarded).
    pub intake_timing: f32,
    pub exhaust_timing: f32,
    /// Boost where the wastegate starts to open (bar).
    pub boost: f32,
    pub ecu: EcuModel,
    /// Longest header primaries there's room for around the engine in its car (m).
    pub header_room: f32,
}

impl Parts {
    /// Version `version` of `family` as the factory builds it (wherever it's fitted).
    pub fn of_version(catalogue: &Shared, family: usize, version: usize) -> Self {
        let preset = &catalogue.families[family].versions[version].preset;
        Self {
            catalogue: catalogue.clone(),
            family,
            version,
            chosen: preset.chosen.clone(),
            bore: 0,
            intake_timing: 0.0,
            exhaust_timing: 0.0,
            boost: preset.boost,
            ecu: preset.ecu,
            header_room: f32::INFINITY,
        }
    }

    /// The factory version with the code `code` in `catalogue`.
    pub fn factory_in(catalogue: &Shared, code: &str) -> Option<Self> {
        let (family, version) = catalogue.find_engine(code)?;
        Some(Self::of_version(catalogue, family, version))
    }

    /// The factory version with the code `code` in the built-in catalogue.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn factory(code: &str) -> Option<Self> {
        Self::factory_in(&crate::catalogue::builtin(), code)
    }

    /// The built-in catalogue's stock engine, the hot hatch's 1.6 four.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn stock() -> Self {
        Self::factory("B16").expect("the 1.6 four")
    }

    /// The 1.6 four turbocharged: low compression pistons and the medium turbo.
    #[cfg(test)]
    pub fn turbo() -> Self {
        let mut parts = Self::stock();
        parts.chosen[crate::catalogue::PISTONS] = 1;
        parts.chosen[crate::catalogue::TURBO] = 2;
        parts
    }

    pub fn family(&self) -> &Family {
        &self.catalogue.families[self.family]
    }

    fn version(&self) -> &Version {
        &self.family().versions[self.version]
    }

    /// The part fitted in each slot.
    pub fn fitted(&self) -> impl Iterator<Item = &Part> {
        let slots = &self.catalogue.slots;
        (slots.iter().zip(&self.chosen)).map(|(slot, &chosen)| &slot.parts[chosen])
    }

    pub fn has_turbo(&self) -> bool {
        let turbo = |effect: &Effect| matches!(effect, Effect::Turbo(_));
        self.fitted().any(|part| part.effects.iter().any(turbo))
            || self.version().block.turbo.is_some()
    }

    /// Just as the factory builds this version (wherever it's fitted).
    pub fn is_factory(&self) -> bool {
        let factory = Self {
            header_room: self.header_room,
            ..Self::of_version(&self.catalogue, self.family, self.version)
        };
        *self == factory
    }

    /// The engine these parts make.
    pub fn hardware(&self) -> EngineHardware {
        let version = self.version();
        let mut hardware = version.block.clone();
        for part in self.fitted() {
            for effect in &part.effects {
                effect.apply(&mut hardware);
            }
        }
        // A block bored over takes bigger pistons.
        if let Some(&mm) = crate::items::OVERSIZES
            .get(self.bore)
            .filter(|&&mm| mm > 0.0)
        {
            hardware.bore = round_to(hardware.bore + mm * 1e-3, 1e-5);
        }
        // Where the primaries the manifold should have don't fit, shorter ones are made.
        let room = round_to(self.header_room, 0.01);
        hardware.exhaust.header_length = hardware.exhaust.header_length.min(room);
        hardware.intake_timing = self.intake_timing;
        hardware.exhaust_timing = self.exhaust_timing;
        if let Some(turbo) = &mut hardware.turbo {
            turbo.wastegate.spring = (self.boost * 1e5).round();
        }
        hardware.name = if self.is_factory() {
            format!("{} {}", version.code, version.name)
        } else {
            let kind = if hardware.turbo.is_some() {
                " turbo"
            } else if hardware.supercharger.is_some() {
                " supercharged"
            } else {
                ""
            };
            let litres = hardware.displacement() * 1e3;
            format!("{} {litres:.1}{kind}, modified", self.family().name)
        };
        hardware
    }

    /// What `part` of `slot` comes to on this engine, e.g. "Stroker (92.0 mm, 2.1 l)".
    pub fn label(&self, slot: usize, part: usize) -> String {
        let mut fitted = self.clone();
        fitted.chosen[slot] = part;
        let slot = &self.catalogue.slots[slot];
        let detail = slot.readout.of(&fitted.hardware());
        let name = &slot.parts[part].name;
        if detail.is_empty() {
            name.to_string()
        } else {
            format!("{name} ({detail})")
        }
    }
}

/// Whether `catalogue`'s parts all fit `parts`' slots.
pub fn fits(catalogue: &Catalogue, chosen: &[usize]) -> bool {
    chosen.len() == catalogue.slots.len()
        && (chosen.iter().zip(&catalogue.slots)).all(|(&part, slot)| part < slot.parts.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{HEADERS, INTERCOOLER, TURBO, builtin};

    #[test]
    fn the_presets_are_the_stock_engines() {
        let stock = Parts::stock().hardware();
        assert_eq!(stock.name, "B16 1.6 four");
        let block = &builtin().families[0].versions[0].block;
        assert_eq!(
            stock,
            EngineHardware {
                name: stock.name.clone(),
                ..block.clone()
            }
        );
        let turbo = Parts::turbo().hardware();
        assert_eq!(turbo.compression_ratio, block.compression_ratio - 1.7);
        let charger = turbo.turbo.unwrap();
        assert_eq!(charger.wastegate.spring, 0.6e5);
    }

    #[test]
    fn parts_are_sized_for_the_engine() {
        // A stroker crank adds 7 % to any block, and a turbo for the bigger engine flows
        // 7 % more, on wheels as much bigger as its flow's square root.
        let mut parts = Parts::stock();
        let stock = parts.hardware();
        parts.chosen[1] = 2;
        parts.chosen[TURBO] = 2;
        let built = parts.hardware();
        let grown = built.displacement() / stock.displacement();
        assert!((grown - 1.07).abs() < 0.002, "{grown}");
        assert_eq!(built.name, "B-series four 1.7 turbo, modified");
        let (big, small) = (
            built.turbo.unwrap(),
            Parts::turbo().hardware().turbo.unwrap(),
        );
        let wheels = big.compressor.diameter / small.compressor.diameter;
        assert!((wheels - grown.sqrt()).abs() < 0.01, "{wheels}");
        assert_eq!(parts.label(1, 2), "Stroker (89.5 mm, 1.7 l)");
        assert_eq!(parts.label(TURBO, 2), "Medium");
    }

    #[test]
    fn headers_are_cut_to_the_room_there_is() {
        let mut parts = Parts::stock();
        parts.header_room = 0.6;
        let hardware = parts.hardware();
        assert_eq!(hardware.exhaust.header_length, 0.6);
        // It's still the factory's engine, in a tight car.
        assert_eq!(hardware.name, "B16 1.6 four");
        parts.chosen[HEADERS] = 1;
        assert_eq!(parts.hardware().exhaust.header_length, 0.39);
    }

    #[test]
    fn springs_and_stroke_set_the_redline() {
        let redline = |choices: &[(usize, usize)]| {
            let mut parts = Parts::stock();
            for &(slot, part) in choices {
                parts.chosen[slot] = part;
            }
            parts.hardware().redline()
        };
        let (crank, internals, cams, springs) = (1, 3, 5, 6);
        assert_eq!(redline(&[]), 6800.0);
        // A wilder cam on the stock springs floats later (the longer duration eases the
        // valves' deceleration more than the lift adds), but not much.
        let sport = redline(&[(cams, 1)]);
        assert!((6900.0..7400.0).contains(&sport), "{sport}");
        // Stiffer springs: the rods limit then, and a short stroke revs higher.
        let sprung = redline(&[(cams, 1), (springs, 2)]);
        assert!(sprung > sport, "{sprung}");
        assert!(redline(&[(cams, 1), (springs, 2), (crank, 1)]) >= sprung + 300.0);
        assert!(redline(&[(cams, 1), (springs, 2), (crank, 2)]) <= sprung - 300.0);
        // Forged internals rev higher still.
        let forged = redline(&[(cams, 1), (springs, 2), (internals, 2)]);
        assert!(forged > sprung + 1500.0, "{forged}");
    }

    #[test]
    fn every_part_fits_every_engine() {
        let catalogue = builtin();
        for (f, family) in catalogue.families.iter().enumerate() {
            for (v, version) in family.versions.iter().enumerate() {
                let factory = Parts::of_version(&catalogue, f, v);
                let stock = factory.hardware();
                assert_eq!(stock.name, format!("{} {}", version.code, version.name));
                for (i, slot) in catalogue.slots.iter().enumerate() {
                    for (j, part) in slot.parts.iter().enumerate() {
                        let mut parts = factory.clone();
                        parts.chosen[i] = j;
                        let hardware = parts.hardware();
                        let what = format!("{}: {}", version.code, part.name);
                        assert!(hardware.redline() >= 5000.0, "{what}");
                        assert!(!parts.label(i, j).is_empty());
                        // Every part changes something, except an intercooler without
                        // a turbo to cool.
                        let idle = i == INTERCOOLER && !parts.has_turbo();
                        if parts != factory && !idle {
                            assert_ne!(hardware, stock, "{what}");
                            assert!(hardware.name.ends_with(", modified"), "{what}");
                        }
                    }
                }
            }
        }
    }
}
