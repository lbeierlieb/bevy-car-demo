//! The catalogue on the network: the server sends it to every player on joining. Reading
//! checks that it holds together (every place it refers to is in it, every list that must
//! have something does), so a car built from it can't index out of it.

use super::{Catalogue, Clutch, Paint, Suspension, TireCompound};
use crate::body::{Body, Detail, Tone};
use crate::cargo::{Area, Hold};
use crate::chassis::{Chassis, EngineBay, Gearbox, GearboxKind, Orientation, RollBars};
use crate::items::{Does, Item, Kind, Knob, Panel, PartDef, Seat, SlotDef, Trim};
use crate::net::wire::{Reader, Wire};
use crate::physics::drivetrain::Drive;
use crate::physics::engine::families::{Family, Layout, Version};
use crate::physics::engine::hardware::{
    BottomEnd, CamProfile, Compressor, Cooling, Diverter, EngineHardware, Exhaust, Intake,
    Intercooler, Turbine, Turbo, ValveTrain, Wastegate,
};
use crate::physics::engine::parts::{
    Change, Effect, Field, Part, Preset, Readout, Regrind, Slot, TurboKit,
};
use crate::physics::fuel::Tank;
use crate::physics::tire::Compound;
use crate::wheels::WheelRoom;

/// The longest text in a catalogue (bytes).
const MAX_TEXT: usize = 2000;

/// Text longer than a chat line: names and descriptions.
struct Text(String);

impl Wire for Text {
    fn put(&self, out: &mut Vec<u8>) {
        let mut end = self.0.len().min(MAX_TEXT);
        while !self.0.is_char_boundary(end) {
            end -= 1;
        }
        (end as u16).put(out);
        out.extend_from_slice(&self.0.as_bytes()[..end]);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let n = usize::from(r.get::<u16>()?);
        (n <= MAX_TEXT).then_some(())?;
        Some(Self(String::from_utf8(r.take(n)?.to_vec()).ok()?))
    }
}

fn put_text(text: &str, out: &mut Vec<u8>) {
    Text(text.to_string()).put(out);
}

fn text(r: &mut Reader) -> Option<String> {
    Some(r.get::<Text>()?.0)
}

/// A struct as its fields one after another; `text` fields as `Text`.
macro_rules! fields {
    ($t:ty { $($f:ident),* $(,)? }) => {
        fields!($t { $($f),* } text {});
    };
    ($t:ty { $($f:ident),* $(,)? } text { $($s:ident),* $(,)? }) => {
        impl Wire for $t {
            fn put(&self, out: &mut Vec<u8>) {
                $(put_text(&self.$s, out);)*
                $(self.$f.put(out);)*
            }
            fn get(r: &mut Reader) -> Option<Self> {
                $(let $s = text(r)?;)*
                Some(Self { $($s,)* $($f: r.get()?,)* })
            }
        }
    };
}

/// An enum without data, by its place in `all`.
macro_rules! listed {
    ($t:ty, $all:expr) => {
        impl Wire for $t {
            fn put(&self, out: &mut Vec<u8>) {
                ($all.iter().position(|x| x == self).unwrap_or(0)).put(out);
            }
            fn get(r: &mut Reader) -> Option<Self> {
                $all.get(r.get::<usize>()?).copied()
            }
        }
    };
}

listed!(
    Tone,
    [
        Tone::Paint,
        Tone::Dark,
        Tone::Glass,
        Tone::Lamp,
        Tone::TailLamp,
        Tone::Metal
    ]
);
listed!(
    Orientation,
    [Orientation::Transverse, Orientation::Longitudinal]
);
listed!(
    GearboxKind,
    [
        GearboxKind::Transverse,
        GearboxKind::Inline,
        GearboxKind::Transaxle
    ]
);
listed!(Field, Field::ALL);
listed!(Readout, Readout::ALL);
listed!(Layout, Layout::ALL);

fields!(Detail { at, size, tone });
fields!(Tank { litres, at, height });
fields!(WheelRoom {
    diameter,
    width,
    min_rim
});
fields!(RollBars {
    sizes,
    factory,
    rate
});
fields!(EngineBay {
    orientation,
    length,
    width,
    height,
    engine_at,
    gearbox_at,
    header_room
} text { place });
fields!(Body {
    length,
    width,
    front_overhang,
    floor,
    deck,
    cabin,
    nose,
    tail,
    tumblehome,
    open_wheels,
    openings,
    details,
    door,
    bumpers,
    bumper_top
});
fields!(Gearbox {
    kind,
    gears,
    reverse,
    mass,
    clutch_torque,
    shift_time,
    engage_time
} text { name });
fields!(Clutch { torque } text { name });
fields!(Suspension {
    ride,
    damping,
    camber,
    reach
} text { name });
fields!(Paint { color, shop } text { name });
fields!(Compound {
    grip,
    peak_angle,
    peak_ratio,
    c_lat,
    c_long,
    best,
    cold,
    overheat,
    life,
    rolling
});
fields!(TireCompound { rubber } text { name, about });
fields!(Chassis {
    mass,
    mass_at,
    cg_height,
    wheelbase,
    track,
    wheels,
    room,
    hub,
    ride,
    damping,
    bars,
    camber,
    pressure,
    max_steer,
    brakes,
    drag_area,
    downforce_area,
    tank,
    bay,
    drive,
    limited_slip,
    gearbox,
    final_drive,
    engines,
    paint,
    body,
    eye,
    insulation,
    traffic,
    hold
} text { name, class, about });
fields!(Hold {
    from,
    left,
    base,
    areas
});
fields!(Area {
    along,
    across,
    floor,
    ceiling,
    seat
});

impl Wire for Drive {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Drive::Rear => 0u8.put(out),
            Drive::Front => 1u8.put(out),
            Drive::All { front } => {
                2u8.put(out);
                front.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Drive::Rear,
            1 => Drive::Front,
            2 => Drive::All {
                front: r.get::<f32>()?.clamp(0.0, 1.0),
            },
            _ => return None,
        })
    }
}

fields!(CamProfile {
    intake_duration,
    exhaust_duration,
    lobe_separation,
    lift
});
fields!(BottomEnd {
    piston_mass,
    rod_mass,
    rod_strength,
    piston_toughness
});
fields!(ValveTrain {
    seat_force,
    rate,
    mass
});
fields!(Intake {
    filter_area,
    throttle_diameter,
    idle_valve_area,
    runner_length,
    plenum_volume,
    tract_volume
});
fields!(Compressor {
    diameter,
    head,
    choke,
    efficiency
});
fields!(Turbine {
    diameter,
    area,
    efficiency
});
fields!(Wastegate { area, spring, span });
fields!(Diverter { area, opens, span });
fields!(Intercooler {
    effectiveness,
    area
});
fields!(Turbo {
    compressor,
    turbine,
    inertia,
    wastegate,
    intercooler,
    diverter
});
fields!(Exhaust {
    header_length,
    pipe_area,
    cat_area,
    silencer_area
});
fields!(Cooling {
    radiator_area,
    radiator_effectiveness,
    thermostat_opens,
    thermostat_open,
    fan_on,
    fan_off,
    fan_speed,
    coolant_capacity,
    oil_capacity
});
fields!(EngineHardware {
    cylinders,
    bore,
    stroke,
    rod_length,
    compression_ratio,
    bottom_end,
    head_gasket,
    cams,
    intake_phasing,
    exhaust_phasing,
    intake_timing,
    exhaust_timing,
    valves,
    port_flow,
    intake,
    turbo,
    exhaust,
    cooling,
    inertia
} text { name });
fields!(Preset { chosen, boost, ecu });
fields!(Version { block, preset } text { code, name });
fields!(Family {
    layout,
    length,
    width,
    height,
    mass,
    versions
} text { name, about });

fields!(Regrind {
    duration,
    separation,
    lift
});
fields!(TurboKit {
    compressor,
    turbine,
    turbine_area,
    inertia,
    wastegate
});

impl Wire for Change {
    fn put(&self, out: &mut Vec<u8>) {
        let (kind, x) = match *self {
            Change::Times(x) => (0u8, x),
            Change::Over(x) => (1, x),
            Change::Plus(x) => (2, x),
            Change::To(x) => (3, x),
            Change::AtLeast(x) => (4, x),
        };
        kind.put(out);
        x.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let kind: u8 = r.get()?;
        let x: f32 = r.get()?;
        Some(match kind {
            0 => Change::Times(x),
            1 if x != 0.0 => Change::Over(x),
            2 => Change::Plus(x),
            3 => Change::To(x),
            4 => Change::AtLeast(x),
            _ => return None,
        })
    }
}

impl Wire for Effect {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Effect::Change {
                field,
                change,
                step,
            } => {
                0u8.put(out);
                field.put(out);
                change.put(out);
                step.put(out);
            }
            Effect::Remove(field) => {
                1u8.put(out);
                field.put(out);
            }
            Effect::Regrind(regrind) => {
                2u8.put(out);
                regrind.put(out);
            }
            Effect::Switching { mild, wild, wilder } => {
                3u8.put(out);
                mild.put(out);
                wild.put(out);
                wilder.put(out);
            }
            Effect::Turbo(kit) => {
                4u8.put(out);
                kit.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Effect::Change {
                field: r.get()?,
                change: r.get()?,
                step: r.get::<f32>()?.max(0.0),
            },
            1 => Effect::Remove(r.get()?),
            2 => Effect::Regrind(r.get()?),
            3 => Effect::Switching {
                mild: r.get()?,
                wild: r.get()?,
                wilder: r.get()?,
            },
            4 => Effect::Turbo(r.get()?),
            _ => return None,
        })
    }
}

fields!(Part { effects } text { name });
fields!(Slot {
    parts,
    readout,
    needs_turbo
} text { name, about });

listed!(Panel, Panel::ALL);
listed!(Seat, Seat::ALL);

impl Wire for Kind {
    fn put(&self, out: &mut Vec<u8>) {
        let tag: u8 = match self {
            Kind::Chassis => 0,
            Kind::Body(_) => 1,
            Kind::Seat(_) => 2,
            Kind::Wheel => 3,
            Kind::Rim => 4,
            Kind::Tire => 5,
            Kind::Brake => 6,
            Kind::Spring => 7,
            Kind::Damper => 8,
            Kind::AntiRollBar => 9,
            Kind::Gearbox => 10,
            Kind::FinalDrive => 11,
            Kind::Differential => 12,
            Kind::Clutch => 13,
            Kind::Ecu => 14,
            Kind::Engine => 15,
            Kind::LongBlock => 16,
            Kind::EnginePart(_) => 17,
        };
        tag.put(out);
        match self {
            Kind::Body(panel) => panel.put(out),
            Kind::Seat(seat) => seat.put(out),
            Kind::EnginePart(slot) => slot.put(out),
            _ => {}
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Kind::Chassis,
            1 => Kind::Body(r.get()?),
            2 => Kind::Seat(r.get()?),
            3 => Kind::Wheel,
            4 => Kind::Rim,
            5 => Kind::Tire,
            6 => Kind::Brake,
            7 => Kind::Spring,
            8 => Kind::Damper,
            9 => Kind::AntiRollBar,
            10 => Kind::Gearbox,
            11 => Kind::FinalDrive,
            12 => Kind::Differential,
            13 => Kind::Clutch,
            14 => Kind::Ecu,
            15 => Kind::Engine,
            16 => Kind::LongBlock,
            17 => Kind::EnginePart(r.get()?),
            _ => return None,
        })
    }
}

impl Wire for Does {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Does::Chassis(c) => (0u8, *c).put(out),
            Does::Nothing => 1u8.put(out),
            Does::Wheel => 2u8.put(out),
            Does::Rim => 3u8.put(out),
            Does::Tire(c) => (4u8, *c).put(out),
            Does::Brake => 5u8.put(out),
            Does::Spring(k) => (6u8, *k).put(out),
            Does::Damper(k) => (7u8, *k).put(out),
            Does::AntiRollBar(a) => (8u8, *a).put(out),
            Does::Gearbox(g) => (9u8, *g).put(out),
            Does::FinalDrive => 10u8.put(out),
            Does::Differential { limited_slip } => {
                11u8.put(out);
                limited_slip.put(out);
            }
            Does::Clutch(k) => (12u8, *k).put(out),
            Does::Ecu(model) => {
                13u8.put(out);
                model.put(out);
            }
            Does::Engine => 14u8.put(out),
            Does::Block { family, version } => {
                15u8.put(out);
                (*family, *version).put(out);
            }
            Does::EnginePart { slot, part } => {
                16u8.put(out);
                (*slot, *part).put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Does::Chassis(r.get()?),
            1 => Does::Nothing,
            2 => Does::Wheel,
            3 => Does::Rim,
            4 => Does::Tire(r.get()?),
            5 => Does::Brake,
            6 => Does::Spring(r.get()?),
            7 => Does::Damper(r.get()?),
            8 => Does::AntiRollBar(r.get::<usize>()?.min(1)),
            9 => Does::Gearbox(r.get()?),
            10 => Does::FinalDrive,
            11 => Does::Differential {
                limited_slip: r.get()?,
            },
            12 => Does::Clutch(r.get()?),
            13 => Does::Ecu(r.get()?),
            14 => Does::Engine,
            15 => {
                let (family, version) = r.get()?;
                Does::Block { family, version }
            }
            16 => {
                let (slot, part) = r.get()?;
                Does::EnginePart { slot, part }
            }
            _ => return None,
        })
    }
}

fields!(Knob { low, high, default } text { name });
fields!(SlotDef { kind, required } text { name });
fields!(PartDef {
    kind,
    does,
    fits,
    size,
    mass,
    price,
    painted,
    knobs,
    slots
} text { id, name });
fields!(Trim { car } text { id, name });

/// How deep a tree of items goes at most (a car, its engine, its long block, a part).
const ITEM_DEPTH: usize = 6;

impl Wire for Item {
    fn put(&self, out: &mut Vec<u8>) {
        // Places past 255: definitions, and tire sizes.
        (self.def as u16).put(out);
        self.paint.put(out);
        (self.variant as u16).put(out);
        self.knobs.put(out);
        self.slots.put(out);
    }

    /// Read only into a tree that's right for the reader's catalogue (`Item::fine`).
    fn get(r: &mut Reader) -> Option<Self> {
        let item = read_item(r, 0)?;
        match r.catalogue() {
            Some(catalogue) => item.fine(catalogue).then_some(item),
            None => Some(item),
        }
    }
}

fn read_item(r: &mut Reader, depth: usize) -> Option<Item> {
    (depth < ITEM_DEPTH).then_some(())?;
    let def = usize::from(r.get::<u16>()?);
    let paint = r.get()?;
    let variant = usize::from(r.get::<u16>()?);
    let knobs = r.get()?;
    // As `Vec<Option<Item>>` puts them.
    let n: u16 = r.get()?;
    (n <= 256).then_some(())?;
    let slots = (0..n)
        .map(|_| match r.get::<bool>()? {
            false => Some(None),
            true => Some(Some(read_item(r, depth + 1)?)),
        })
        .collect::<Option<_>>()?;
    Some(Item {
        def,
        paint,
        variant,
        knobs,
        slots,
    })
}

impl Item {
    /// Whether the tree refers only to what's in `catalogue`, with each part in a slot
    /// of its kind, sizes, paints and knobs within what there is (the chassis it's in
    /// deciding the anti-roll bars' sizes).
    /// A loose part (not a car) goes by the chassis it's made for, if any.
    pub fn fine(&self, catalogue: &Catalogue) -> bool {
        let chassis = match catalogue.defs.get(self.def) {
            Some(PartDef {
                does: Does::Chassis(c),
                ..
            }) => Some(*c),
            Some(def) => (def.fits.first())
                .and_then(|name| catalogue.chassis.iter().position(|c| c.name == *name)),
            None => None,
        };
        self.fine_in(catalogue, chassis)
    }

    fn fine_in(&self, catalogue: &Catalogue, chassis: Option<usize>) -> bool {
        let Some(def) = catalogue.defs.get(self.def) else {
            return false;
        };
        let knobs = self.knobs.len() == def.knobs.len()
            && (self.knobs.iter().zip(&def.knobs))
                .all(|(x, knob)| (knob.low - 1e-3..=knob.high + 1e-3).contains(x));
        let paint = match self.paint {
            Some(paint) => def.painted && paint < catalogue.paints.len(),
            None => !def.painted,
        };
        let slots = self.slots.len() == def.slots.len()
            && (self.slots.iter().zip(&def.slots)).all(|(item, slot)| match item {
                Some(item) => {
                    (catalogue.defs.get(item.def)).is_some_and(|d| d.kind == slot.kind)
                        && item.fine_in(catalogue, chassis)
                }
                None => !slot.required,
            });
        knobs && paint && slots && self.variant < def.variants(catalogue, chassis).max(1)
    }
}

impl Wire for Catalogue {
    fn put(&self, out: &mut Vec<u8>) {
        self.chassis.put(out);
        self.gearboxes.put(out);
        self.clutches.put(out);
        self.suspensions.put(out);
        self.paints.put(out);
        self.compounds.put(out);
        self.families.put(out);
        self.slots.put(out);
        self.start.put(out);
        self.defs.put(out);
        self.trims.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let mut catalogue = Catalogue::new(
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
        );
        catalogue.defs = r.get()?;
        catalogue.trims = r.get()?;
        catalogue.rehash();
        catalogue.holds_together().then_some(catalogue)
    }
}

impl Catalogue {
    /// Whether every place it refers to is in it, and the lists cars need have something.
    pub fn holds_together(&self) -> bool {
        let wheels = |wheels: &[crate::wheels::Wheel; 2]| {
            (wheels.iter()).all(|wheel| wheel.compound < self.compounds.len())
        };
        let chassis = |chassis: &Chassis| {
            chassis.gearbox < self.gearboxes.len()
                && chassis.paint < self.paints.len()
                && wheels(&chassis.wheels)
                && (chassis.bars.iter()).all(|bars| bars.factory < bars.sizes.len())
                && !chassis.engines.is_empty()
                && (chassis.engines.iter()).all(|code| self.find_engine(code).is_some())
                && chassis.wheelbase > 0.5
                && chassis.mass > 50.0
        };
        let version = |version: &Version| {
            version.block.cylinders > 0
                && !version.block.cams.is_empty()
                && crate::physics::engine::parts::fits(self, &version.preset.chosen)
        };
        let slot = |slot: &Slot| !slot.parts.is_empty();
        let does = |def: &PartDef| match def.does {
            Does::Chassis(c) => c < self.chassis.len(),
            Does::Tire(c) => c < self.compounds.len(),
            Does::Spring(k) | Does::Damper(k) => k < self.suspensions.len(),
            Does::Gearbox(g) => g < self.gearboxes.len(),
            Does::Clutch(k) => k < self.clutches.len(),
            Does::Block { family, version } => {
                (self.families.get(family)).is_some_and(|f| version < f.versions.len())
            }
            Does::EnginePart { slot, part } => {
                (self.slots.get(slot)).is_some_and(|s| part < s.parts.len())
            }
            _ => true,
        };
        let kind = |kind: &Kind| match kind {
            Kind::EnginePart(slot) => *slot < self.slots.len(),
            _ => true,
        };
        let def = |def: &PartDef| {
            does(def)
                && kind(&def.kind)
                && def.slots.iter().all(|slot| kind(&slot.kind))
                && def
                    .knobs
                    .iter()
                    .all(|knob| knob.low <= knob.default && knob.default <= knob.high)
        };
        // Every trim makes a car (`resolve` needs a shared catalogue).
        let shared = std::sync::Arc::new(self.clone());
        let trim = |trim: &Trim| trim.car.fine(self) && trim.car.resolve(&shared).is_ok();
        let gearbox = |gearbox: &Gearbox| !gearbox.gears.is_empty();
        self.start < self.chassis.len()
            && !self.clutches.is_empty()
            && !self.suspensions.is_empty()
            && self.chassis.iter().all(chassis)
            && self.gearboxes.iter().all(gearbox)
            && self.slots.iter().all(slot)
            && (self.families.iter())
                .all(|f| !f.versions.is_empty() && f.versions.iter().all(version))
            && self.defs.iter().all(def)
            && self.trims.iter().all(trim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;
    use crate::net::wire::{decode, encode};

    #[test]
    fn the_catalogue_reads_back_as_it_was() {
        let catalogue = builtin();
        assert!(catalogue.holds_together());
        let bytes = encode(&*catalogue);
        println!("the built-in catalogue is {} bytes", bytes.len());
        let read: Catalogue = decode(&bytes).expect("reads back");
        assert_eq!(read.fingerprint(), catalogue.fingerprint());
        assert_eq!(read.chassis, catalogue.chassis);
        assert_eq!(read.families, catalogue.families);
        assert_eq!(read.slots, catalogue.slots);
        assert_eq!(read.compounds, catalogue.compounds);
    }

    #[test]
    fn a_catalogue_that_refers_out_of_itself_is_refused() {
        let mut catalogue = Catalogue::clone(&builtin());
        catalogue.chassis[0].gearbox = catalogue.gearboxes.len();
        assert_eq!(decode::<Catalogue>(&encode(&catalogue)), None);
        let mut catalogue = Catalogue::clone(&builtin());
        catalogue.chassis[0].engines.push("Q99".into());
        assert_eq!(decode::<Catalogue>(&encode(&catalogue)), None);
    }
}
