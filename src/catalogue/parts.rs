//! The built-in catalogue's engine parts and tire compounds.

use super::TireCompound;
use crate::physics::engine::parts::{
    Change, Effect, Field, Part, Readout, Regrind, Slot, TurboKit,
};
use crate::physics::tire::{COMPOUNDS, RACE, SPORT, STREET, TOURING};

pub const PISTONS: usize = 0;
/// The exhaust manifold's slot: how long its primaries can be depends on the car.
pub const HEADERS: usize = 9;
pub const TURBO: usize = 12;
/// The intercooler's slot: it only matters with a turbo.
pub const INTERCOOLER: usize = 13;
/// How many slots there are.
pub const SLOTS: usize = 20;

pub fn compounds() -> Vec<TireCompound> {
    let compound = |rubber: usize, name: &str, about: &str| TireCompound {
        name: name.into(),
        about: about.into(),
        rubber: COMPOUNDS[rubber],
    };
    vec![
        compound(
            TOURING,
            "Touring",
            "Made to last and to roll easily: grips from cold and never overheats on the \
             road, but has the least grip and lets go early",
        ),
        compound(
            STREET,
            "Street",
            "Grips from cold and lasts, and lets go gently past the limit, but overheats \
             when driven hard for long",
        ),
        compound(
            SPORT,
            "Sport",
            "More grip and sharper, with a higher working temperature and faster wear",
        ),
        compound(
            RACE,
            "Race",
            "The most grip once hot, little when cold; peaks early and drops off past \
             it, and wears fast",
        ),
    ]
}

fn part(name: &str, effects: &[Effect]) -> Part {
    Part {
        name: name.into(),
        effects: effects.to_vec(),
    }
}

fn stock() -> Part {
    part("Stock", &[])
}

fn slot(name: &str, about: &str, readout: Readout, parts: Vec<Part>) -> Slot {
    Slot {
        name: name.into(),
        about: about.into(),
        parts,
        readout,
        needs_turbo: false,
    }
}

/// `field` times `by`, rounded to `step` (none for 0).
const fn times(field: Field, by: f32, step: f32) -> Effect {
    Effect::Change {
        field,
        change: Change::Times(by),
        step,
    }
}

const fn set(field: Field, value: f32) -> Effect {
    Effect::Change {
        field,
        change: Change::To(value),
        step: 0.0,
    }
}

const fn at_least(field: Field, value: f32) -> Effect {
    Effect::Change {
        field,
        change: Change::AtLeast(value),
        step: 0.0,
    }
}

const fn compression(change: f32) -> Effect {
    Effect::Change {
        field: Field::CompressionRatio,
        change: Change::Plus(change),
        step: 0.1,
    }
}

const fn regrind(duration: f32, separation: f32, lift: f32) -> Regrind {
    Regrind {
        duration,
        separation,
        lift,
    }
}

/// Valve springs with this many times the seat force and rate, and moving mass.
const fn springs(seat_force: f32, rate: f32, mass: f32) -> [Effect; 3] {
    [
        times(Field::SeatForce, seat_force, 1.0),
        times(Field::SpringRate, rate, 100.0),
        times(Field::ValveMass, mass, 1e-3),
    ]
}

const FORGED_RODS: [Effect; 2] = [
    times(Field::RodStrength, 1.4, 100.0),
    times(Field::RodMass, 0.917, 1e-3),
];

const fn turbo(
    compressor: f32,
    turbine: f32,
    turbine_area: f32,
    inertia: f32,
    wastegate: f32,
) -> Effect {
    Effect::Turbo(TurboKit {
        compressor,
        turbine,
        turbine_area,
        inertia,
        wastegate,
    })
}

pub fn slots() -> Vec<Slot> {
    use Field::*;
    let slots = vec![
        slot(
            "Pistons",
            "Compression ratio: more is more efficient, but knocks earlier",
            Readout::Compression,
            vec![
                stock(),
                part("Low compression", &[compression(-1.7)]),
                part("High compression", &[compression(1.0)]),
                part("Race", &[compression(2.0)]),
            ],
        ),
        slot(
            "Crankshaft",
            "Stroke: a longer one makes a bigger engine; the pistons move faster, so it \
             revs less",
            Readout::Crank,
            vec![
                stock(),
                part("Short stroke", &[times(Stroke, 0.93, 1e-4)]),
                part("Stroker", &[times(Stroke, 1.07, 1e-4)]),
            ],
        ),
        slot(
            "Connecting rods",
            "Longer rods keep the piston near top dead centre longer, with less friction",
            Readout::RodLength,
            vec![
                stock(),
                part("Short", &[times(RodLength, 0.95, 1e-3)]),
                part("Long", &[times(RodLength, 1.05, 1e-3)]),
            ],
        ),
        slot(
            "Internals",
            "Forged rods take more pull at high rpm and more push under boost; forged \
             pistons are lighter (easing the rods) and take more knock",
            Readout::Nothing,
            vec![
                part("Stock (cast)", &[]),
                part("Forged rods", &FORGED_RODS),
                part(
                    "Forged rods and pistons",
                    &[
                        FORGED_RODS[0],
                        FORGED_RODS[1],
                        times(PistonMass, 0.857, 1e-3),
                        set(PistonToughness, 2.0),
                    ],
                ),
            ],
        ),
        slot(
            "Head gasket",
            "Seals the cylinder pressure: boost and knock lift a weak one",
            Readout::HeadGasket,
            vec![
                stock(),
                part("Multi-layer steel, head studs", &[set(HeadGasket, 180e5)]),
            ],
        ),
        slot(
            "Camshafts",
            "Longer duration and more lift breathe better high up, worse low down and at \
             idle (more lift needs stiffer valve springs). A switching cam has a mild and a \
             wild profile, and needs an ECU to switch between them",
            Readout::Cams,
            vec![
                stock(),
                part("Sport", &[Effect::Regrind(regrind(16.0, -2.0, 1.105))]),
                part("Race", &[Effect::Regrind(regrind(36.0, -4.0, 1.21))]),
                part(
                    "Switching",
                    &[Effect::Switching {
                        mild: regrind(-4.0, 2.0, 0.947),
                        wild: regrind(24.0, -2.0, 1.158),
                        wilder: regrind(12.0, -1.0, 1.06),
                    }],
                ),
            ],
        ),
        slot(
            "Valve springs",
            "Stiffer springs keep the valves on the cams to higher rpm (where they'd float \
             and could meet the pistons), at a little more friction",
            Readout::ValveFloat,
            vec![
                stock(),
                part("Uprated", &springs(1.33, 1.4, 1.0)),
                part("Race, titanium retainers", &springs(1.56, 1.7, 0.85)),
            ],
        ),
        slot(
            "Cam phasers",
            "Turn the cams against the crank while running: more overlap and earlier \
             intake closing where they help",
            Readout::Phasers,
            vec![
                stock(),
                part("Intake", &[at_least(IntakePhasing, 40.0)]),
                part(
                    "Intake and exhaust",
                    &[
                        at_least(IntakePhasing, 40.0),
                        at_least(ExhaustPhasing, 30.0),
                    ],
                ),
            ],
        ),
        slot(
            "Intake manifold",
            "Runner length tunes the intake: long runners fill best low, short ones high",
            Readout::Runners,
            vec![
                stock(),
                part("Long runners", &[times(RunnerLength, 1.43, 0.01)]),
                part("Short runners", &[times(RunnerLength, 0.63, 0.01)]),
            ],
        ),
        slot(
            "Exhaust manifold",
            "Header length tunes the scavenging during the valve overlap. Long primaries \
             need room around the engine: what fits depends on the car",
            Readout::Headers,
            vec![
                stock(),
                part("Short headers", &[times(HeaderLength, 0.56, 0.01)]),
                part("Long-tube headers", &[times(HeaderLength, 1.375, 0.01)]),
            ],
        ),
        slot(
            "Air filter",
            "Less restriction ahead of the throttle (or the compressor)",
            Readout::Nothing,
            vec![
                stock(),
                part("Sport panel", &[times(FilterArea, 1.28, 0.0)]),
                part("Open cone", &[times(FilterArea, 1.8, 0.0)]),
            ],
        ),
        slot(
            "Throttle body",
            "A bigger throttle flows more when wide open, and is touchier part-open",
            Readout::Throttle,
            vec![
                stock(),
                part("Bigger", &[times(ThrottleDiameter, 1.09, 1e-3)]),
                part("Biggest", &[times(ThrottleDiameter, 1.18, 1e-3)]),
            ],
        ),
        slot(
            "Turbocharger",
            "A small turbo spools early; a big one can hold more boost at high rpm. Big \
             engines get two, sized to match",
            Readout::Nothing,
            vec![
                part("None", &[]),
                part("Small", &[turbo(0.044, 0.048, 4.5e-4, 1.6e-5, 6e-4)]),
                part("Medium", &[turbo(0.05, 0.054, 6e-4, 2.5e-5, 3e-4)]),
                part("Large", &[turbo(0.058, 0.062, 8e-4, 4.2e-5, 3e-4)]),
            ],
        ),
        Slot {
            needs_turbo: true,
            ..slot(
                "Intercooler",
                "Cools the charge after the compressor (only with a turbo)",
                Readout::Nothing,
                vec![
                    stock(),
                    part(
                        "Large",
                        &[
                            set(IntercoolerEffectiveness, 0.8),
                            times(IntercoolerArea, 1.5, 0.0),
                        ],
                    ),
                    part(
                        "None",
                        &[
                            set(IntercoolerEffectiveness, 0.0),
                            set(IntercoolerArea, 1.0),
                        ],
                    ),
                ],
            )
        },
        slot(
            "Catalyst",
            "The catalytic converter's restriction",
            Readout::Nothing,
            vec![
                stock(),
                part("High-flow", &[times(CatArea, 1.5, 0.0)]),
                part("None", &[Effect::Remove(CatArea)]),
            ],
        ),
        slot(
            "Silencer",
            "The silencer's restriction",
            Readout::Nothing,
            vec![
                stock(),
                part("Sport", &[times(SilencerArea, 1.5, 0.0)]),
                part("None (straight pipe)", &[Effect::Remove(SilencerArea)]),
            ],
        ),
        slot(
            "Exhaust pipe",
            "Pipe diameter after the manifold",
            Readout::Pipe,
            vec![stock(), part("Big bore", &[times(PipeArea, 1.37, 0.0)])],
        ),
        slot(
            "Radiator",
            "How much heat the cooling system can get rid of",
            Readout::Nothing,
            vec![
                stock(),
                part(
                    "Race",
                    &[
                        times(RadiatorArea, 4.0 / 3.0, 0.0),
                        set(RadiatorEffectiveness, 0.7),
                    ],
                ),
            ],
        ),
        slot(
            "Flywheel",
            "A lighter flywheel revs up and down quicker",
            Readout::Flywheel,
            vec![
                stock(),
                part(
                    "Lightweight",
                    &[Effect::Change {
                        field: Inertia,
                        change: Change::Over(1.5),
                        step: 0.01,
                    }],
                ),
            ],
        ),
        slot(
            "Cylinder head",
            "Ported runners and bigger valves flow more air at high rpm",
            Readout::PortFlow,
            vec![
                stock(),
                part("Ported", &[times(PortFlow, 1.1, 0.01)]),
                part("Race, big valves", &[times(PortFlow, 1.2, 0.01)]),
            ],
        ),
    ];
    debug_assert_eq!(slots.len(), SLOTS);
    slots
}
