//! Parts to build an engine from, as a garage stocks them: a slot per kind of part, each
//! with a few to choose from, fitted onto a factory version of an engine family. The
//! parts are made relative to the engine they go onto (a stroker crank for any block,
//! turbos sized for its displacement). Some slots change the cylinder side, so fitting
//! their parts means baking the maps again; the rest act live.

use super::ecu::EcuModel;
#[cfg(test)]
use super::families::Family;
use super::families::{FAMILIES, Version, WORKHORSE, relative_size};
use super::hardware::{
    CamProfile, Compressor, Diverter, EngineHardware, Intercooler, Turbine, Turbo, Wastegate,
};

/// A kind of part, and the parts there are of it (the first is stock).
pub struct Slot {
    pub name: &'static str,
    pub about: &'static str,
    pub parts: &'static [Part],
    /// What the slot's part comes to on an engine, e.g. "92 mm, 2.1 l" for a crank.
    detail: fn(&EngineHardware) -> String,
}

pub struct Part {
    pub name: &'static str,
    fit: fn(&mut EngineHardware),
}

const fn part(name: &'static str, fit: fn(&mut EngineHardware)) -> Part {
    Part { name, fit }
}

fn no_detail(_: &EngineHardware) -> String {
    String::new()
}

/// `value` rounded to a whole number of `step`s (a power of ten), to the nearest f32
/// there, so that 0.6 m comes out as 0.6.
pub(super) fn round_to(value: f32, step: f32) -> f32 {
    if step >= 1.0 {
        (value / step).round() * step
    } else {
        let per = (1.0 / step).round();
        (value * per).round() / per
    }
}

pub const PISTONS: usize = 0;
pub const TURBO: usize = 12;
/// The intercooler's slot: it only matters with a turbo.
pub const INTERCOOLER: usize = 13;

pub const SLOTS: [Slot; 20] = [
    Slot {
        name: "Pistons",
        about: "Compression ratio: more is more efficient, but knocks earlier",
        parts: &[
            part("Stock", |_| {}),
            part("Low compression", |h| compression(h, -1.7)),
            part("High compression", |h| compression(h, 1.0)),
            part("Race", |h| compression(h, 2.0)),
        ],
        detail: |h| format!("{:.1}:1", h.compression_ratio),
    },
    Slot {
        name: "Crankshaft",
        about: "Stroke: a longer one makes a bigger engine; the pistons move faster, so it \
                revs less",
        parts: &[
            part("Stock", |_| {}),
            part("Short stroke", |h| {
                h.stroke = round_to(h.stroke * 0.93, 1e-4)
            }),
            part("Stroker", |h| h.stroke = round_to(h.stroke * 1.07, 1e-4)),
        ],
        detail: |h| {
            let litres = h.displacement() * 1e3;
            format!("{:.1} mm, {litres:.1} l", h.stroke * 1e3)
        },
    },
    Slot {
        name: "Connecting rods",
        about: "Longer rods keep the piston near top dead centre longer, with less friction",
        parts: &[
            part("Stock", |_| {}),
            part("Short", |h| {
                h.rod_length = round_to(h.rod_length * 0.95, 1e-3)
            }),
            part("Long", |h| {
                h.rod_length = round_to(h.rod_length * 1.05, 1e-3)
            }),
        ],
        detail: |h| format!("{:.0} mm", h.rod_length * 1e3),
    },
    Slot {
        name: "Internals",
        about: "Forged rods take more pull at high rpm and more push under boost; forged \
                pistons are lighter (easing the rods) and take more knock",
        parts: &[
            part("Stock (cast)", |_| {}),
            part("Forged rods", forged_rods),
            part("Forged rods and pistons", |h| {
                forged_rods(h);
                h.bottom_end.piston_mass = round_to(h.bottom_end.piston_mass * 0.857, 1e-3);
                h.bottom_end.piston_toughness = 2.0;
            }),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Head gasket",
        about: "Seals the cylinder pressure: boost and knock lift a weak one",
        parts: &[
            part("Stock", |_| {}),
            part("Multi-layer steel, head studs", |h| h.head_gasket = 180e5),
        ],
        detail: |h| format!("{:.0} bar", h.head_gasket / 1e5),
    },
    Slot {
        name: "Camshafts",
        about: "Longer duration and more lift breathe better high up, worse low down and at \
                idle (more lift needs stiffer valve springs). A switching cam has a mild and a wild profile, and \
                needs an ECU to switch between them",
        parts: &[
            part("Stock", |_| {}),
            part("Sport", |h| {
                for cam in &mut h.cams {
                    *cam = reground(cam, 16.0, -2.0, 1.105);
                }
            }),
            part("Race", |h| {
                for cam in &mut h.cams {
                    *cam = reground(cam, 36.0, -4.0, 1.21);
                }
            }),
            part("Switching", |h| {
                // On an engine that switches cams already, a wilder second profile.
                h.cams = match h.cams[..] {
                    [cam] => vec![
                        reground(&cam, -4.0, 2.0, 0.947),
                        reground(&cam, 24.0, -2.0, 1.158),
                    ],
                    [mild, wild, ..] => vec![mild, reground(&wild, 12.0, -1.0, 1.06)],
                    [] => vec![],
                };
            }),
        ],
        detail: |h| {
            let durations: Vec<String> = h
                .cams
                .iter()
                .map(|cam| format!("{:.0}°", cam.intake_duration))
                .collect();
            let lift = h.cams.last().map_or(0.0, |cam| cam.lift * 1e3);
            format!("{}, {lift:.1} mm", durations.join(" / "))
        },
    },
    Slot {
        name: "Valve springs",
        about: "Stiffer springs keep the valves on the cams to higher rpm (where they'd \
                float and could meet the pistons), at a little more friction",
        parts: &[
            part("Stock", |_| {}),
            part("Uprated", |h| springs(h, 1.33, 1.4, 1.0)),
            part("Race, titanium retainers", |h| springs(h, 1.56, 1.7, 0.85)),
        ],
        detail: |h| {
            let cam = h.cams.last().expect("a cam");
            format!(
                "float at {:.0} rpm",
                round_to(h.valves.float_rpm(cam), 100.0)
            )
        },
    },
    Slot {
        name: "Cam phasers",
        about: "Turn the cams against the crank while running: more overlap and earlier \
                intake closing where they help",
        parts: &[
            part("Stock", |_| {}),
            part("Intake", |h| h.intake_phasing = h.intake_phasing.max(40.0)),
            part("Intake and exhaust", |h| {
                h.intake_phasing = h.intake_phasing.max(40.0);
                h.exhaust_phasing = h.exhaust_phasing.max(30.0);
            }),
        ],
        detail: |h| match (h.intake_phasing, h.exhaust_phasing) {
            (0.0, 0.0) => "none".to_string(),
            (intake, 0.0) => format!("intake {intake:.0}°"),
            (intake, exhaust) => format!("{intake:.0}° / {exhaust:.0}°"),
        },
    },
    Slot {
        name: "Intake manifold",
        about: "Runner length tunes the intake: long runners fill best low, short ones high",
        parts: &[
            part("Stock", |_| {}),
            part("Long runners", |h| {
                h.intake.runner_length = round_to(h.intake.runner_length * 1.43, 0.01)
            }),
            part("Short runners", |h| {
                h.intake.runner_length = round_to(h.intake.runner_length * 0.63, 0.01)
            }),
        ],
        detail: |h| format!("{:.0} mm runners", h.intake.runner_length * 1e3),
    },
    Slot {
        name: "Exhaust manifold",
        about: "Header length tunes the scavenging during the valve overlap. Long primaries \
                need room around the engine: what fits depends on the car",
        parts: &[
            part("Stock", |_| {}),
            part("Short headers", |h| {
                h.exhaust.header_length = round_to(h.exhaust.header_length * 0.56, 0.01)
            }),
            part("Long-tube headers", |h| {
                h.exhaust.header_length = round_to(h.exhaust.header_length * 1.375, 0.01)
            }),
        ],
        detail: |h| format!("{:.0} mm", h.exhaust.header_length * 1e3),
    },
    Slot {
        name: "Air filter",
        about: "Less restriction ahead of the throttle (or the compressor)",
        parts: &[
            part("Stock", |_| {}),
            part("Sport panel", |h| h.intake.filter_area *= 1.28),
            part("Open cone", |h| h.intake.filter_area *= 1.8),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Throttle body",
        about: "A bigger throttle flows more when wide open, and is touchier part-open",
        parts: &[
            part("Stock", |_| {}),
            part("Bigger", |h| {
                h.intake.throttle_diameter = round_to(h.intake.throttle_diameter * 1.09, 1e-3)
            }),
            part("Biggest", |h| {
                h.intake.throttle_diameter = round_to(h.intake.throttle_diameter * 1.18, 1e-3)
            }),
        ],
        detail: |h| format!("{:.0} mm", h.intake.throttle_diameter * 1e3),
    },
    Slot {
        name: "Turbocharger",
        about: "A small turbo spools early; a big one can hold more boost at high rpm. Big \
                engines get two, sized to match",
        parts: &[
            part("None", |_| {}),
            part("Small", |h| {
                fit_turbo(h, 0.044, 0.048, 4.5e-4, 1.6e-5, 6e-4)
            }),
            part("Medium", |h| fit_turbo(h, 0.05, 0.054, 6e-4, 2.5e-5, 3e-4)),
            part("Large", |h| fit_turbo(h, 0.058, 0.062, 8e-4, 4.2e-5, 3e-4)),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Intercooler",
        about: "Cools the charge after the compressor (only with a turbo)",
        parts: &[
            part("Stock", |_| {}),
            part("Large", |h| {
                if let Some(turbo) = &mut h.turbo {
                    turbo.intercooler.effectiveness = 0.8;
                    turbo.intercooler.area *= 1.5;
                }
            }),
            part("None", |h| {
                if let Some(turbo) = &mut h.turbo {
                    turbo.intercooler = Intercooler {
                        effectiveness: 0.0,
                        area: 1.0,
                    };
                }
            }),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Catalyst",
        about: "The catalytic converter's restriction",
        parts: &[
            part("Stock", |_| {}),
            part("High-flow", |h| {
                h.exhaust.cat_area = h.exhaust.cat_area.map(|area| area * 1.5)
            }),
            part("None", |h| h.exhaust.cat_area = None),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Silencer",
        about: "The silencer's restriction",
        parts: &[
            part("Stock", |_| {}),
            part("Sport", |h| {
                h.exhaust.silencer_area = h.exhaust.silencer_area.map(|area| area * 1.5)
            }),
            part("None (straight pipe)", |h| h.exhaust.silencer_area = None),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Exhaust pipe",
        about: "Pipe diameter after the manifold",
        parts: &[
            part("Stock", |_| {}),
            part("Big bore", |h| h.exhaust.pipe_area *= 1.37),
        ],
        detail: |h| {
            let diameter = (h.exhaust.pipe_area * 4.0 / std::f32::consts::PI).sqrt();
            format!("{:.0} mm", diameter * 1e3)
        },
    },
    Slot {
        name: "Radiator",
        about: "How much heat the cooling system can get rid of",
        parts: &[
            part("Stock", |_| {}),
            part("Race", |h| {
                h.cooling.radiator_area *= 4.0 / 3.0;
                h.cooling.radiator_effectiveness = 0.7;
            }),
        ],
        detail: no_detail,
    },
    Slot {
        name: "Flywheel",
        about: "A lighter flywheel revs up and down quicker",
        parts: &[
            part("Stock", |_| {}),
            part("Lightweight", |h| {
                h.inertia = round_to(h.inertia / 1.5, 0.01)
            }),
        ],
        detail: |h| format!("{:.2} kg·m²", h.inertia),
    },
    Slot {
        name: "Cylinder head",
        about: "Ported runners and bigger valves flow more air at high rpm",
        parts: &[
            part("Stock", |_| {}),
            part("Ported", |h| {
                h.port_flow = round_to(h.port_flow * 1.1, 0.01)
            }),
            part("Race, big valves", |h| {
                h.port_flow = round_to(h.port_flow * 1.2, 0.01)
            }),
        ],
        detail: |h| format!("{:.0} % flow", h.port_flow * 100.0),
    },
];

fn compression(hardware: &mut EngineHardware, change: f32) {
    hardware.compression_ratio = round_to(hardware.compression_ratio + change, 0.1);
}

fn forged_rods(hardware: &mut EngineHardware) {
    let bottom_end = &mut hardware.bottom_end;
    bottom_end.rod_strength = round_to(bottom_end.rod_strength * 1.4, 100.0);
    bottom_end.rod_mass = round_to(bottom_end.rod_mass * 0.917, 1e-3);
}

/// A cam profile with `duration` more crank degrees, the lobes `separation` cam degrees
/// further apart, and `lift` times the lift.
fn reground(cam: &CamProfile, duration: f32, separation: f32, lift: f32) -> CamProfile {
    CamProfile {
        intake_duration: cam.intake_duration + duration,
        exhaust_duration: cam.exhaust_duration + duration,
        lobe_separation: cam.lobe_separation + separation,
        lift: round_to(cam.lift * lift, 1e-4),
    }
}

/// Valve springs with this many times the seat force and rate, and moving mass.
fn springs(hardware: &mut EngineHardware, seat_force: f32, rate: f32, mass: f32) {
    let valves = &mut hardware.valves;
    valves.seat_force = (valves.seat_force * seat_force).round();
    valves.rate = round_to(valves.rate * rate, 100.0);
    valves.mass = round_to(valves.mass * mass, 1e-3);
}

/// A turbo kit: the turbo with compressor and turbine wheels of these diameters, the
/// turbine's flow area, the shaft's inertia and the wastegate's flow area (a small turbine
/// needs a big one to let the excess exhaust past), a diverter valve, an intercooler, and
/// the piping to it (more volume ahead of the throttle). The numbers are for a 2-litre
/// engine: the kit is scaled to the engine's displacement (flow areas with it, so wheel
/// diameters with its root).
fn fit_turbo(
    hardware: &mut EngineHardware,
    compressor: f32,
    turbine: f32,
    turbine_area: f32,
    inertia: f32,
    wastegate: f32,
) {
    let size = relative_size(hardware);
    let wheel = size.sqrt();
    hardware.intake.tract_volume *= 3.0;
    hardware.turbo = Some(Turbo {
        compressor: Compressor {
            diameter: compressor * wheel,
            head: 0.7,
            choke: 0.29,
            efficiency: 0.75,
        },
        turbine: Turbine {
            diameter: turbine * wheel,
            area: turbine_area * size,
            efficiency: 0.7,
        },
        inertia: inertia * wheel.powi(5),
        wastegate: Wastegate {
            area: wastegate * size,
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

/// The parts an engine is built from: the factory version of an engine family it started
/// as, a part per slot, the cam gears' timing, the wastegate's spring, and the ECU.
#[derive(Clone, Debug, PartialEq)]
pub struct Parts {
    /// The engine family (in `FAMILIES`) and its version.
    pub family: usize,
    pub version: usize,
    /// The part fitted in each slot of `SLOTS`, by index.
    pub chosen: [usize; SLOTS.len()],
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
    /// A block with the stock part in every slot (which isn't what the factory builds for
    /// every version: see `Version::parts`).
    pub fn all_stock(family: usize, version: usize) -> Self {
        Self {
            family,
            version,
            chosen: [0; SLOTS.len()],
            intake_timing: 0.0,
            exhaust_timing: 0.0,
            boost: 0.6,
            ecu: EcuModel::Stock,
            header_room: f32::INFINITY,
        }
    }

    /// The stock naturally aspirated 2-litre four.
    pub fn stock() -> Self {
        Version::parts(WORKHORSE, 0)
    }

    /// The stock turbo 2-litre four: low compression pistons and the medium turbo.
    #[cfg(test)]
    pub fn turbo() -> Self {
        Version::parts(WORKHORSE, 1)
    }

    /// The factory version with the code `code`.
    #[cfg(test)]
    pub fn factory(code: &str) -> Option<Self> {
        Family::find(code).map(|(family, version)| Version::parts(family, version))
    }

    pub fn family(&self) -> &'static super::families::Family {
        &FAMILIES[self.family]
    }

    fn version(&self) -> &'static Version {
        &self.family().versions[self.version]
    }

    pub fn has_turbo(&self) -> bool {
        self.chosen[TURBO] != 0
    }

    /// Just as the factory builds this version (wherever it's fitted).
    pub fn is_factory(&self) -> bool {
        let factory = Self {
            header_room: self.header_room,
            ..Version::parts(self.family, self.version)
        };
        *self == factory
    }

    /// The engine these parts make.
    pub fn hardware(&self) -> EngineHardware {
        let version = self.version();
        let mut hardware = version.block();
        for (slot, &chosen) in SLOTS.iter().zip(&self.chosen) {
            (slot.parts[chosen.min(slot.parts.len() - 1)].fit)(&mut hardware);
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
        let detail = (SLOTS[slot].detail)(&fitted.hardware());
        let name = SLOTS[slot].parts[part].name;
        if detail.is_empty() {
            name.to_string()
        } else {
            format!("{name} ({detail})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_are_the_stock_engines() {
        let stock = Parts::stock().hardware();
        assert_eq!(stock.name, "W20 2.0 four");
        assert_eq!(
            stock,
            EngineHardware {
                name: stock.name.clone(),
                ..EngineHardware::default()
            }
        );
        let turbo = Parts::turbo().hardware();
        assert_eq!(turbo.name, "W20T 2.0 turbo four");
        assert_eq!(turbo.compression_ratio, 8.8);
        let charger = turbo.turbo.unwrap();
        assert_eq!(charger.wastegate.spring, 0.6e5);
        assert_eq!(charger.wastegate.area, 3e-4);
        assert_eq!(Parts::factory("W20T"), Some(Parts::turbo()));
    }

    #[test]
    fn parts_are_sized_for_the_engine() {
        // A stroker crank adds 7 % to any block; a turbo for a 6.2 V8 flows three times
        // what one for a 2-litre does, on wheels 1.75 times the size.
        let mut parts = Parts::factory("M62").unwrap();
        let stock = parts.hardware();
        parts.chosen[1] = 2;
        parts.chosen[TURBO] = 2;
        let built = parts.hardware();
        let grown = built.displacement() / stock.displacement();
        assert!((grown - 1.07).abs() < 0.002, "{grown}");
        assert_eq!(built.name, "M-series V8 6.6 turbo, modified");
        let (big, small) = (
            built.turbo.unwrap(),
            Parts::turbo().hardware().turbo.unwrap(),
        );
        let wheels = big.compressor.diameter / small.compressor.diameter;
        assert!((wheels - 3.3f32.sqrt()).abs() < 0.01, "{wheels}");
        assert!(big.max_speed() < small.max_speed());
        assert_eq!(parts.label(1, 2), "Stroker (98.4 mm, 6.6 l)");
        assert_eq!(parts.label(TURBO, 2), "Medium");
    }

    #[test]
    fn headers_are_cut_to_the_room_there_is() {
        let mut parts = Parts::stock();
        parts.header_room = 0.6;
        let hardware = parts.hardware();
        assert_eq!(hardware.exhaust.header_length, 0.6);
        // It's still the factory's engine, in a tight car.
        assert_eq!(hardware.name, "W20 2.0 four");
        parts.chosen[9] = 1;
        assert_eq!(parts.hardware().exhaust.header_length, 0.45);
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
        for (f, family) in FAMILIES.iter().enumerate() {
            for (v, version) in family.versions.iter().enumerate() {
                let factory = Version::parts(f, v);
                let stock = factory.hardware();
                assert_eq!(stock.name, format!("{} {}", version.code, version.name));
                for (i, slot) in SLOTS.iter().enumerate() {
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
