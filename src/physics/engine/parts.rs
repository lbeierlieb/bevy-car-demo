//! Parts to build an engine from, as a garage stocks them: a slot per kind of part, each
//! with a few to choose from, fitted onto the stock 2-litre four. Some slots change the
//! cylinder side, so fitting their parts means baking the maps again; the rest act live.

use super::hardware::{
    CamProfile, Compressor, Diverter, EngineHardware, Intercooler, Turbine, Turbo, Wastegate,
};

/// A kind of part, and the parts there are of it (the first is stock).
pub struct Slot {
    pub name: &'static str,
    pub about: &'static str,
    pub parts: &'static [Part],
}

pub struct Part {
    pub name: &'static str,
    fit: fn(&mut EngineHardware),
}

const fn part(name: &'static str, fit: fn(&mut EngineHardware)) -> Part {
    Part { name, fit }
}

const PISTONS: usize = 0;
const TURBO: usize = 8;
/// The intercooler's slot: it only matters with a turbo.
pub const INTERCOOLER: usize = 9;

pub const SLOTS: [Slot; 15] = [
    Slot {
        name: "Pistons",
        about: "Compression ratio: more is more efficient, but knocks earlier",
        parts: &[
            part("Stock (10.5:1)", |_| {}),
            part("Low compression (8.8:1)", |h| h.compression_ratio = 8.8),
            part("High compression (11.5:1)", |h| h.compression_ratio = 11.5),
            part("Race (12.5:1)", |h| h.compression_ratio = 12.5),
        ],
    },
    Slot {
        name: "Crankshaft",
        about: "Stroke: a longer one makes a bigger engine that runs out of breath sooner",
        parts: &[
            part("Stock (86 mm, 2.0 l)", |_| {}),
            part("Short stroke (80 mm, 1.9 l)", |h| {
                h.stroke = 0.080;
                h.redline += 400.0;
            }),
            part("Stroker (92 mm, 2.1 l)", |h| {
                h.stroke = 0.092;
                h.redline -= 300.0;
            }),
        ],
    },
    Slot {
        name: "Connecting rods",
        about: "Longer rods keep the piston near top dead centre longer, with less friction",
        parts: &[
            part("Stock (143 mm)", |_| {}),
            part("Short (136 mm)", |h| h.rod_length = 0.136),
            part("Long (150 mm)", |h| h.rod_length = 0.150),
        ],
    },
    Slot {
        name: "Camshafts",
        about: "Longer duration and more lift breathe better high up (and rev higher), worse \
                low down and at idle",
        parts: &[
            part("Stock (240°, 9.5 mm)", |_| {}),
            part("Sport (256°, 10.5 mm)", |h| {
                h.cams = vec![cam(256.0, 108.0, 0.0105)];
                h.redline += 600.0;
            }),
            part("Race (276°, 11.5 mm)", |h| {
                h.cams = vec![cam(276.0, 106.0, 0.0115)];
                h.redline += 1200.0;
            }),
        ],
    },
    Slot {
        name: "Intake manifold",
        about: "Runner length tunes the intake: long runners fill best low, short ones high",
        parts: &[
            part("Stock (350 mm runners)", |_| {}),
            part("Long runners (500 mm)", |h| h.intake.runner_length = 0.5),
            part("Short runners (220 mm)", |h| h.intake.runner_length = 0.22),
        ],
    },
    Slot {
        name: "Exhaust manifold",
        about: "Header length tunes the scavenging during the valve overlap",
        parts: &[
            part("Stock headers (800 mm)", |_| {}),
            part("Short headers (450 mm)", |h| h.exhaust.header_length = 0.45),
            part("Long-tube headers (1100 mm)", |h| {
                h.exhaust.header_length = 1.1
            }),
        ],
    },
    Slot {
        name: "Air filter",
        about: "Less restriction ahead of the throttle (or the compressor)",
        parts: &[
            part("Stock", |_| {}),
            part("Sport panel", |h| h.intake.filter_area = 3.2e-3),
            part("Open cone", |h| h.intake.filter_area = 4.5e-3),
        ],
    },
    Slot {
        name: "Throttle body",
        about: "A bigger throttle flows more when wide open, and is touchier part-open",
        parts: &[
            part("Stock (55 mm)", |_| {}),
            part("60 mm", |h| h.intake.throttle_diameter = 0.060),
            part("65 mm", |h| h.intake.throttle_diameter = 0.065),
        ],
    },
    Slot {
        name: "Turbocharger",
        about: "A small turbo spools early; a big one can hold more boost at high rpm",
        parts: &[
            part("None", |_| {}),
            part("Small", |h| {
                fit_turbo(h, 0.044, 0.048, 4.5e-4, 1.6e-5, 6e-4)
            }),
            part("Medium", |h| fit_turbo(h, 0.05, 0.054, 6e-4, 2.5e-5, 3e-4)),
            part("Large", |h| fit_turbo(h, 0.058, 0.062, 8e-4, 4.2e-5, 3e-4)),
        ],
    },
    Slot {
        name: "Intercooler",
        about: "Cools the charge after the compressor (only with a turbo)",
        parts: &[
            part("Stock", |_| {}),
            part("Large", |h| set_intercooler(h, 0.8, 4.5e-3)),
            part("None", |h| set_intercooler(h, 0.0, 1.0)),
        ],
    },
    Slot {
        name: "Catalyst",
        about: "The catalytic converter's restriction",
        parts: &[
            part("Stock", |_| {}),
            part("High-flow", |h| h.exhaust.cat_area = Some(3.0e-3)),
            part("None", |h| h.exhaust.cat_area = None),
        ],
    },
    Slot {
        name: "Silencer",
        about: "The silencer's restriction",
        parts: &[
            part("Stock", |_| {}),
            part("Sport", |h| h.exhaust.silencer_area = Some(2.4e-3)),
            part("None (straight pipe)", |h| h.exhaust.silencer_area = None),
        ],
    },
    Slot {
        name: "Exhaust pipe",
        about: "Pipe diameter after the manifold",
        parts: &[
            part("Stock (50 mm)", |_| {}),
            part("Big bore (58 mm)", |h| h.exhaust.pipe_area = 2.6e-3),
        ],
    },
    Slot {
        name: "Radiator",
        about: "How much heat the cooling system can get rid of",
        parts: &[
            part("Stock", |_| {}),
            part("Race", |h| {
                h.cooling.radiator_area = 0.4;
                h.cooling.radiator_effectiveness = 0.7;
            }),
        ],
    },
    Slot {
        name: "Flywheel",
        about: "A lighter flywheel revs up and down quicker",
        parts: &[
            part("Stock (0.18 kg·m²)", |_| {}),
            part("Lightweight (0.12 kg·m²)", |h| h.inertia = 0.12),
        ],
    },
];

/// A cam profile with the same intake and exhaust lobes.
fn cam(duration: f32, lobe_separation: f32, lift: f32) -> CamProfile {
    CamProfile {
        intake_duration: duration,
        exhaust_duration: duration,
        lobe_separation,
        lift,
    }
}

/// A turbo kit: the turbo with compressor and turbine wheels of these diameters, the
/// turbine's flow area, the shaft's inertia and the wastegate's flow area (a small turbine
/// needs a big one to let the excess exhaust past), a diverter valve, the stock
/// intercooler, and the piping to it (more volume ahead of the throttle).
fn fit_turbo(
    hardware: &mut EngineHardware,
    compressor: f32,
    turbine: f32,
    turbine_area: f32,
    inertia: f32,
    wastegate: f32,
) {
    hardware.intake.tract_volume = 6.0e-3;
    hardware.turbo = Some(Turbo {
        compressor: Compressor {
            diameter: compressor,
            head: 0.7,
            choke: 0.29,
            efficiency: 0.75,
        },
        turbine: Turbine {
            diameter: turbine,
            area: turbine_area,
            efficiency: 0.7,
        },
        inertia,
        wastegate: Wastegate {
            area: wastegate,
            spring: 0.6e5,
            span: 0.12e5,
        },
        intercooler: Intercooler {
            effectiveness: 0.7,
            area: 3e-3,
        },
        diverter: Some(Diverter {
            area: 5e-4,
            opens: 0.3e5,
            span: 0.2e5,
        }),
    });
}

fn set_intercooler(hardware: &mut EngineHardware, effectiveness: f32, area: f32) {
    if let Some(turbo) = &mut hardware.turbo {
        turbo.intercooler = Intercooler {
            effectiveness,
            area,
        };
    }
}

/// The parts an engine is built from: one per slot, the cam gears' timing and the
/// wastegate's spring.
#[derive(Clone, Debug, PartialEq)]
pub struct Parts {
    /// The part fitted in each slot of `SLOTS`, by index.
    pub chosen: [usize; SLOTS.len()],
    /// Cam timing (crank degrees; intake advanced, exhaust retarded).
    pub intake_timing: f32,
    pub exhaust_timing: f32,
    /// Boost where the wastegate starts to open (bar).
    pub boost: f32,
}

impl Parts {
    /// The stock naturally aspirated engine.
    pub fn stock() -> Self {
        Self {
            chosen: [0; SLOTS.len()],
            intake_timing: 0.0,
            exhaust_timing: 0.0,
            boost: 0.6,
        }
    }

    /// The stock turbo engine: low compression pistons and the medium turbo.
    pub fn turbo() -> Self {
        let mut parts = Self::stock();
        parts.chosen[PISTONS] = 1;
        parts.chosen[TURBO] = 2;
        parts
    }

    pub fn has_turbo(&self) -> bool {
        self.chosen[TURBO] != 0
    }

    /// The engine these parts make.
    pub fn hardware(&self) -> EngineHardware {
        let mut hardware = EngineHardware::default();
        for (slot, &chosen) in SLOTS.iter().zip(&self.chosen) {
            (slot.parts[chosen.min(slot.parts.len() - 1)].fit)(&mut hardware);
        }
        hardware.intake_timing = self.intake_timing;
        hardware.exhaust_timing = self.exhaust_timing;
        if let Some(turbo) = &mut hardware.turbo {
            turbo.wastegate.spring = (self.boost * 1e5).round();
        }
        hardware.name = if *self == Self::stock() {
            "2.0 naturally aspirated".to_string()
        } else if *self == Self::turbo() {
            "2.0 turbo".to_string()
        } else {
            let kind = if hardware.turbo.is_some() {
                "turbo"
            } else {
                "naturally aspirated"
            };
            format!("{:.1} {kind}, modified", hardware.displacement() * 1e3)
        };
        hardware
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_are_the_stock_engines() {
        assert_eq!(Parts::stock().hardware(), EngineHardware::default());
        let turbo = Parts::turbo().hardware();
        assert_eq!(turbo.name, "2.0 turbo");
        assert_eq!(turbo.compression_ratio, 8.8);
        let charger = turbo.turbo.unwrap();
        assert_eq!(charger.wastegate.spring, 0.6e5);
        assert_eq!(charger.wastegate.area, 3e-4);
    }

    #[test]
    fn every_part_fits() {
        for (i, slot) in SLOTS.iter().enumerate() {
            for (j, part) in slot.parts.iter().enumerate() {
                let mut parts = Parts::turbo();
                parts.chosen[i] = j;
                let hardware = parts.hardware();
                assert!(hardware.displacement() > 1.5e-3, "{}", part.name);
                if parts != Parts::turbo() {
                    assert_ne!(hardware, Parts::turbo().hardware(), "{}", part.name);
                }
            }
        }
        let mut modified = Parts::stock();
        modified.chosen[2] = 2;
        assert_eq!(
            modified.hardware().name,
            "2.0 naturally aspirated, modified"
        );
    }
}
